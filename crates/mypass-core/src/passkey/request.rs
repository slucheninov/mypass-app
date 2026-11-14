//! Parsing of the WebAuthn request JSON that Chrome hands to the extension's
//! `webAuthenticationProxy` (`requestDetailsJson`), for `create` and `get`.
//!
//! The page controls every value in it, so each field is bounded before any work: the strings
//! land in entry titles, user names and the pending ceremony table. Anything over a cap or with
//! characters the UI must not show is `InvalidRequest`; nothing over a cap is stored or echoed.

use super::{b64url, PasskeyError};
use crate::text::{is_clean_line, MAX_TITLE_CHARS, MAX_USERNAME_CHARS};
use serde_json::{Map, Value};

/// Largest `requestDetailsJson` accepted, in bytes.
pub const MAX_REQUEST_JSON_BYTES: usize = 64 * 1024;
/// Longest relying party id, in characters (the DNS name limit).
pub const MAX_RP_ID_CHARS: usize = 253;
/// Longest `user.id` (the WebAuthn user handle), in bytes after decoding.
pub const MAX_USER_ID_BYTES: usize = 64;
/// Shortest challenge, in bytes after decoding (WebAuthn asks for at least 16 random bytes).
pub const MIN_CHALLENGE_BYTES: usize = 16;
/// Longest challenge, in bytes after decoding.
pub const MAX_CHALLENGE_BYTES: usize = 1024;
/// Longest credential id in `excludeCredentials` / `allowCredentials`, in bytes after decoding
/// (the WebAuthn credential id limit).
pub const MAX_CREDENTIAL_ID_BYTES: usize = 1023;
/// Most entries in `excludeCredentials` or `allowCredentials`.
pub const MAX_CREDENTIAL_LIST_ITEMS: usize = 64;
/// Most entries in `pubKeyCredParams`.
pub const MAX_PUB_KEY_CRED_PARAMS: usize = 32;

/// COSE algorithm identifier of ES256 (ECDSA P-256 with SHA-256), the only one MyPass signs with.
const COSE_ALG_ES256: i64 = -7;

/// A parsed `navigator.credentials.create()` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateContext {
    /// `rp.id` as the page sent it, or `None` when it was omitted. It is not authoritative:
    /// `validate_origin` turns it (or, when absent, the origin host) into the canonical rpId,
    /// and only `ValidatedOrigin::rp_id` is hashed and stored.
    pub rp_id: Option<String>,
    /// `rp.name`, falling back to `rp.id`, or empty when both are absent (callers then show the
    /// validated rpId).
    pub rp_name: String,
    /// The challenge bytes.
    pub challenge: Vec<u8>,
    /// The user handle (`user.id`), 1 to 64 bytes.
    pub user_id: Vec<u8>,
    /// `user.name`, empty when absent.
    pub user_name: String,
    /// `user.displayName`, falling back to the user name.
    pub user_display_name: String,
    /// Decoded ids of the `public-key` entries of `excludeCredentials`.
    pub exclude_credential_ids: Vec<Vec<u8>>,
    /// The attestation conveyance, normalised: `direct`, `indirect` or `enterprise` verbatim,
    /// `none` for `none`, an absent value or anything unrecognised (WebAuthn: unknown values are
    /// ignored). This departs from the Swift app, which mapped every non-`none` value to packed.
    pub attestation: String,
}

/// A parsed `navigator.credentials.get()` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetContext {
    /// `rpId` as the page sent it, or `None` when it was omitted; see `CreateContext::rp_id`.
    pub rp_id: Option<String>,
    /// The challenge bytes.
    pub challenge: Vec<u8>,
    /// Decoded ids of the `public-key` entries of `allowCredentials`; empty means any credential
    /// of the relying party.
    pub allow_credential_ids: Vec<Vec<u8>>,
}

/// Parse a create request. Ports `PasskeyCeremony.parseCreateRequest`.
///
/// # Errors
/// `UnsupportedAlgorithm` when `pubKeyCredParams` is non-empty and offers no ES256 `public-key`
/// entry; `InvalidRequest` for everything else that is missing, malformed or over a cap.
pub fn parse_create_request(json: &str) -> Result<CreateContext, PasskeyError> {
    let root = parse_object(json)?;
    let challenge = challenge(&root)?;

    let rp = root
        .get("rp")
        .and_then(Value::as_object)
        .ok_or(PasskeyError::InvalidRequest)?;
    let rp_id = optional_rp_id(rp.get("id"))?;
    let rp_name = match rp.get("name").and_then(Value::as_str) {
        Some(name) => name.to_owned(),
        None => rp_id.clone().unwrap_or_default(),
    };
    if !is_clean_line(&rp_name, MAX_TITLE_CHARS) {
        return Err(PasskeyError::InvalidRequest);
    }

    let user = root
        .get("user")
        .and_then(Value::as_object)
        .ok_or(PasskeyError::InvalidRequest)?;
    let user_id = user
        .get("id")
        .and_then(Value::as_str)
        .and_then(b64url::decode)
        .filter(|id| (1..=MAX_USER_ID_BYTES).contains(&id.len()))
        .ok_or(PasskeyError::InvalidRequest)?;
    let user_name = user
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let user_display_name = user
        .get("displayName")
        .and_then(Value::as_str)
        .map_or_else(|| user_name.clone(), str::to_owned);
    if !is_clean_line(&user_name, MAX_USERNAME_CHARS)
        || !is_clean_line(&user_display_name, MAX_USERNAME_CHARS)
    {
        return Err(PasskeyError::InvalidRequest);
    }

    check_algorithms(root.get("pubKeyCredParams"))?;

    let attestation = match root.get("attestation").and_then(Value::as_str) {
        Some(known @ ("direct" | "indirect" | "enterprise")) => known.to_owned(),
        _ => "none".to_owned(),
    };

    // Chrome serialises descriptor ids as base64url. Decode to bytes before comparing with vault
    // ids so padding or alphabet cannot change the exclusion decision. A malformed list fails the
    // whole request rather than being dropped: a dropped exclusion would let the page register a
    // duplicate credential for an account that already has one.
    let exclude_credential_ids = match root.get("excludeCredentials") {
        None => Vec::new(),
        Some(list) => credential_descriptors(list)?,
    };

    Ok(CreateContext {
        rp_id,
        rp_name,
        challenge,
        user_id,
        user_name,
        user_display_name,
        exclude_credential_ids,
        attestation,
    })
}

/// Parse a get request. Ports `PasskeyCeremony.parseGetRequest`.
///
/// `allowCredentials` is held to the same descriptor rules as `excludeCredentials`: a malformed
/// list fails the request, and so does a non-empty list none of whose entries is a `public-key`
/// credential. The Swift app dropped undecodable entries, which could turn a list naming only
/// unusable ids into an empty one, and an empty list means "any credential of the relying party".
///
/// # Errors
/// `InvalidRequest` when anything is missing, malformed or over a cap.
pub fn parse_get_request(json: &str) -> Result<GetContext, PasskeyError> {
    let root = parse_object(json)?;
    let challenge = challenge(&root)?;
    let rp_id = optional_rp_id(root.get("rpId"))?;
    let allow_credential_ids = match root.get("allowCredentials") {
        None => Vec::new(),
        Some(list) => {
            let ids = credential_descriptors(list)?;
            let requested_some = list.as_array().is_some_and(|raw| !raw.is_empty());
            if requested_some && ids.is_empty() {
                return Err(PasskeyError::InvalidRequest);
            }
            ids
        }
    };
    Ok(GetContext {
        rp_id,
        challenge,
        allow_credential_ids,
    })
}

/// The request JSON as an object, refusing oversized input before parsing it.
pub(crate) fn parse_object(json: &str) -> Result<Map<String, Value>, PasskeyError> {
    if json.len() > MAX_REQUEST_JSON_BYTES {
        return Err(PasskeyError::InvalidRequest);
    }
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(PasskeyError::InvalidRequest),
    }
}

fn challenge(root: &Map<String, Value>) -> Result<Vec<u8>, PasskeyError> {
    root.get("challenge")
        .and_then(Value::as_str)
        .and_then(b64url::decode)
        .filter(|c| (MIN_CHALLENGE_BYTES..=MAX_CHALLENGE_BYTES).contains(&c.len()))
        .ok_or(PasskeyError::InvalidRequest)
}

/// An absent rpId is `None`; a present one must be a string of at most 253 characters. Its form
/// is checked by `validate_origin`.
fn optional_rp_id(value: Option<&Value>) -> Result<Option<String>, PasskeyError> {
    match value {
        None => Ok(None),
        Some(Value::String(id)) if id.chars().count() <= MAX_RP_ID_CHARS => Ok(Some(id.clone())),
        Some(_) => Err(PasskeyError::InvalidRequest),
    }
}

/// `pubKeyCredParams` absent or empty means the WebAuthn default, which includes ES256.
/// Otherwise it must hold a `public-key` entry with `alg == -7`; entries of other types or
/// algorithms are skipped (a real Chrome payload offers `[-7, -257]`).
fn check_algorithms(value: Option<&Value>) -> Result<(), PasskeyError> {
    let params = match value {
        None => return Ok(()),
        Some(Value::Array(params)) => params,
        Some(_) => return Err(PasskeyError::InvalidRequest),
    };
    if params.len() > MAX_PUB_KEY_CRED_PARAMS {
        return Err(PasskeyError::InvalidRequest);
    }
    let es256 = params.iter().any(|p| {
        p.get("type").and_then(Value::as_str) == Some("public-key")
            && p.get("alg").and_then(Value::as_i64) == Some(COSE_ALG_ES256)
    });
    if params.is_empty() || es256 {
        Ok(())
    } else {
        Err(PasskeyError::UnsupportedAlgorithm)
    }
}

/// A credential descriptor list: an array of at most 64 objects, each with a string `type` and a
/// non-empty base64url `id` of at most 1023 bytes. Ids of `public-key` entries are returned;
/// entries of other types cannot name one of our credentials and are skipped. Extra members
/// (Chrome adds `transports`) are ignored.
fn credential_descriptors(value: &Value) -> Result<Vec<Vec<u8>>, PasskeyError> {
    let list = value.as_array().ok_or(PasskeyError::InvalidRequest)?;
    if list.len() > MAX_CREDENTIAL_LIST_ITEMS {
        return Err(PasskeyError::InvalidRequest);
    }
    let mut ids = Vec::new();
    for descriptor in list {
        let kind = descriptor
            .get("type")
            .and_then(Value::as_str)
            .ok_or(PasskeyError::InvalidRequest)?;
        let id = descriptor
            .get("id")
            .and_then(Value::as_str)
            .and_then(b64url::decode)
            .filter(|id| (1..=MAX_CREDENTIAL_ID_BYTES).contains(&id.len()))
            .ok_or(PasskeyError::InvalidRequest)?;
        if kind == "public-key" {
            ids.push(id);
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    // Test-only support code: assertions may unwrap and index known fixtures.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use serde_json::json;

    fn challenge16() -> Vec<u8> {
        (0..16u8).collect()
    }

    fn create_json(algs: &[i64], attestation: Option<&str>) -> Value {
        let params: Vec<Value> = algs
            .iter()
            .map(|alg| json!({ "type": "public-key", "alg": alg }))
            .collect();
        let mut v = json!({
            "challenge": b64url::encode(&challenge16()),
            "rp": { "id": "example.com", "name": "Example" },
            "user": { "id": b64url::encode(&[1, 2, 3, 4]), "name": "alice", "displayName": "Alice" },
            "pubKeyCredParams": params,
        });
        if let Some(a) = attestation {
            v["attestation"] = json!(a);
        }
        v
    }

    fn create(v: &Value) -> Result<CreateContext, PasskeyError> {
        parse_create_request(&v.to_string())
    }

    fn get_json(ids: &[&[u8]]) -> Value {
        let allow: Vec<Value> = ids
            .iter()
            .map(|id| json!({ "type": "public-key", "id": b64url::encode(id) }))
            .collect();
        json!({
            "challenge": b64url::encode(&challenge16()),
            "rpId": "example.com",
            "allowCredentials": allow,
        })
    }

    #[test]
    fn create_extracts_rp_and_user() {
        let ctx = create(&create_json(&[-7], Some("none"))).unwrap();
        assert_eq!(ctx.rp_id.as_deref(), Some("example.com"));
        assert_eq!(ctx.rp_name, "Example");
        assert_eq!(ctx.user_name, "alice");
        assert_eq!(ctx.user_display_name, "Alice");
        assert_eq!(ctx.challenge, challenge16());
        assert_eq!(ctx.user_id, vec![1, 2, 3, 4]);
        assert_eq!(ctx.attestation, "none");
        assert!(ctx.exclude_credential_ids.is_empty());
    }

    #[test]
    fn create_decodes_exclusions_and_ignores_unsupported_types() {
        let mut v = create_json(&[-7], None);
        v["excludeCredentials"] = json!([
            { "type": "public-key", "id": "-_8" },
            { "type": "public-key", "id": "AQI=" },
            { "type": "future-type", "id": "Aw" },
        ]);
        let ctx = create(&v).unwrap();
        assert_eq!(
            ctx.exclude_credential_ids,
            vec![vec![0xfb, 0xff], vec![1, 2]]
        );
    }

    #[test]
    fn create_rejects_malformed_exclusion_rather_than_dropping_it() {
        let too_long = b64url::encode(&[7u8; MAX_CREDENTIAL_ID_BYTES + 1]);
        let too_many: Vec<Value> = (0..=MAX_CREDENTIAL_LIST_ITEMS)
            .map(|_| json!({ "type": "public-key", "id": "AQ" }))
            .collect();
        let malformed = [
            json!("not an array"),
            json!(null),
            json!({ "type": "public-key", "id": "AQ" }),
            json!([{ "type": "public-key", "id": "%%%" }]),
            json!([{ "type": "public-key", "id": "" }]),
            json!([{ "type": "public-key" }]),
            json!([{ "id": "AQ" }]),
            json!([{ "type": 1, "id": "AQ" }]),
            json!(["AQ"]),
            json!([{ "type": "public-key", "id": too_long }]),
            json!([{ "type": "future-type", "id": "%%%" }]),
            Value::Array(too_many),
        ];
        for descriptors in malformed {
            let mut v = create_json(&[-7], None);
            v["excludeCredentials"] = descriptors.clone();
            assert_eq!(
                create(&v),
                Err(PasskeyError::InvalidRequest),
                "{descriptors}"
            );
        }
    }

    #[test]
    fn create_without_es256_is_unsupported_algorithm() {
        let v = create_json(&[-257], None);
        assert_eq!(create(&v), Err(PasskeyError::UnsupportedAlgorithm));
        // ES256 under another credential type does not count.
        let mut v = create_json(&[], None);
        v["pubKeyCredParams"] = json!([{ "type": "future-type", "alg": -7 }]);
        assert_eq!(create(&v), Err(PasskeyError::UnsupportedAlgorithm));
    }

    #[test]
    fn create_accepts_es256_alongside_others() {
        // A real Chrome payload offers [-7, -257]; it must be accepted, not rejected.
        assert!(create(&create_json(&[-257, -7], None)).is_ok());
    }

    #[test]
    fn create_with_absent_or_empty_params_accepts_the_es256_default() {
        let mut v = create_json(&[], None);
        assert!(create(&v).is_ok());
        v.as_object_mut().unwrap().remove("pubKeyCredParams");
        assert!(create(&v).is_ok());
        v["pubKeyCredParams"] = json!("ES256");
        assert_eq!(create(&v), Err(PasskeyError::InvalidRequest));
        let many = vec![-7; MAX_PUB_KEY_CRED_PARAMS + 1];
        assert_eq!(
            create(&create_json(&many, None)),
            Err(PasskeyError::InvalidRequest)
        );
    }

    #[test]
    fn create_garbage_is_invalid_request() {
        for garbage in ["not json", "", "[]", "null", "\"x\"", "{}"] {
            assert_eq!(
                parse_create_request(garbage),
                Err(PasskeyError::InvalidRequest),
                "{garbage}"
            );
        }
        for missing in ["challenge", "rp", "user"] {
            let mut v = create_json(&[-7], None);
            v.as_object_mut().unwrap().remove(missing);
            assert_eq!(create(&v), Err(PasskeyError::InvalidRequest), "{missing}");
        }
        let mut v = create_json(&[-7], None);
        v["user"].as_object_mut().unwrap().remove("id");
        assert_eq!(create(&v), Err(PasskeyError::InvalidRequest));
        let mut v = create_json(&[-7], None);
        v["challenge"] = json!("%%%");
        assert_eq!(create(&v), Err(PasskeyError::InvalidRequest));
    }

    #[test]
    fn create_defaults_attestation_to_none() {
        let v = json!({
            "challenge": b64url::encode(&challenge16()),
            "rp": { "id": "example.com" },
            "user": { "id": b64url::encode(&[2]), "name": "alice" },
            "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }],
        });
        let ctx = create(&v).unwrap();
        assert_eq!(ctx.attestation, "none");
        assert_eq!(ctx.rp_name, "example.com", "rp name falls back to rp id");
        assert_eq!(
            ctx.user_display_name, "alice",
            "display name falls back to name"
        );
    }

    #[test]
    fn create_normalises_unknown_attestation_to_none() {
        for (given, want) in [
            ("none", "none"),
            ("direct", "direct"),
            ("indirect", "indirect"),
            ("enterprise", "enterprise"),
            ("DIRECT", "none"),
            ("packed", "none"),
            ("", "none"),
        ] {
            let ctx = create(&create_json(&[-7], Some(given))).unwrap();
            assert_eq!(ctx.attestation, want, "{given}");
        }
        let mut v = create_json(&[-7], None);
        v["attestation"] = json!(5);
        assert_eq!(create(&v).unwrap().attestation, "none");
    }

    #[test]
    fn create_without_rp_id_leaves_it_to_origin_validation() {
        let mut v = create_json(&[-7], None);
        v["rp"] = json!({ "name": "Example" });
        let ctx = create(&v).unwrap();
        assert_eq!(ctx.rp_id, None);
        assert_eq!(ctx.rp_name, "Example");
        v["rp"] = json!({});
        assert_eq!(create(&v).unwrap().rp_name, "");
        v["rp"] = json!({ "id": 5 });
        assert_eq!(create(&v), Err(PasskeyError::InvalidRequest));
    }

    #[test]
    fn create_enforces_size_and_text_caps() {
        type Mutation = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Mutation)> = vec![
            (
                "challenge too short",
                Box::new(|v| v["challenge"] = json!(b64url::encode(&[1u8; 15]))),
            ),
            (
                "challenge too long",
                Box::new(|v| v["challenge"] = json!(b64url::encode(&[1u8; 1025]))),
            ),
            ("user id empty", Box::new(|v| v["user"]["id"] = json!(""))),
            (
                "user id too long",
                Box::new(|v| v["user"]["id"] = json!(b64url::encode(&[1u8; 65]))),
            ),
            (
                "rp id too long",
                Box::new(|v| v["rp"]["id"] = json!("a".repeat(254))),
            ),
            (
                "rp name too long",
                Box::new(|v| v["rp"]["name"] = json!("n".repeat(257))),
            ),
            (
                "rp name with a newline",
                Box::new(|v| v["rp"]["name"] = json!("Bank\nPay here")),
            ),
            (
                "rp name with a bidi override",
                Box::new(|v| v["rp"]["name"] = json!("Bank\u{202E}lapyap")),
            ),
            (
                "user name too long",
                Box::new(|v| v["user"]["name"] = json!("u".repeat(513))),
            ),
            (
                "display name too long",
                Box::new(|v| v["user"]["displayName"] = json!("d".repeat(513))),
            ),
            (
                "user name with a control character",
                Box::new(|v| v["user"]["name"] = json!("a\u{0007}")),
            ),
            (
                "display name with a zero-width joiner",
                Box::new(|v| v["user"]["displayName"] = json!("a\u{200D}b")),
            ),
            (
                "rp id fallback name with a newline",
                Box::new(|v| v["rp"] = json!({ "id": "a\nb" })),
            ),
        ];
        for (label, mutate) in cases {
            let mut v = create_json(&[-7], None);
            mutate(&mut v);
            assert_eq!(create(&v), Err(PasskeyError::InvalidRequest), "{label}");
        }
        // The bounds themselves are accepted.
        let mut v = create_json(&[-7], None);
        v["challenge"] = json!(b64url::encode(&[1u8; 1024]));
        v["user"]["id"] = json!(b64url::encode(&[1u8; 64]));
        v["rp"]["name"] = json!("n".repeat(256));
        v["user"]["name"] = json!("u".repeat(512));
        v["user"]["displayName"] = json!("\u{1F3E6}".repeat(512));
        assert!(create(&v).is_ok());
        let mut v = create_json(&[-7], None);
        v["challenge"] = json!(b64url::encode(&[1u8; 16]));
        v["user"]["id"] = json!(b64url::encode(&[1u8]));
        assert!(create(&v).is_ok());
    }

    #[test]
    fn oversized_request_json_is_refused_before_parsing() {
        let mut v = create_json(&[-7], None);
        v["padding"] = json!("x".repeat(MAX_REQUEST_JSON_BYTES));
        assert_eq!(create(&v), Err(PasskeyError::InvalidRequest));
        let mut g = get_json(&[]);
        g["padding"] = json!("x".repeat(MAX_REQUEST_JSON_BYTES));
        assert_eq!(
            parse_get_request(&g.to_string()),
            Err(PasskeyError::InvalidRequest)
        );
    }

    #[test]
    fn deeply_nested_json_is_an_error_not_a_panic() {
        let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
        assert_eq!(
            parse_create_request(&deep),
            Err(PasskeyError::InvalidRequest)
        );
        assert_eq!(parse_get_request(&deep), Err(PasskeyError::InvalidRequest));
    }

    #[test]
    fn get_extracts_rp_and_allow_list() {
        let ctx = parse_get_request(&get_json(&[&[5, 6, 7]]).to_string()).unwrap();
        assert_eq!(ctx.rp_id.as_deref(), Some("example.com"));
        assert_eq!(ctx.challenge, challenge16());
        assert_eq!(ctx.allow_credential_ids, vec![vec![5, 6, 7]]);

        let mut v = get_json(&[]);
        v.as_object_mut().unwrap().remove("allowCredentials");
        let ctx = parse_get_request(&v.to_string()).unwrap();
        assert!(ctx.allow_credential_ids.is_empty());
        v.as_object_mut().unwrap().remove("rpId");
        assert_eq!(parse_get_request(&v.to_string()).unwrap().rp_id, None);
        v.as_object_mut().unwrap().remove("challenge");
        assert_eq!(
            parse_get_request(&v.to_string()),
            Err(PasskeyError::InvalidRequest)
        );
    }

    #[test]
    fn get_ignores_chrome_injected_transports() {
        // Chrome adds a `transports` array of its own; parsing must not depend on it.
        let v = json!({
            "challenge": b64url::encode(&challenge16()),
            "rpId": "example.com",
            "allowCredentials": [{
                "type": "public-key",
                "id": b64url::encode(&[5, 6, 7]),
                "transports": ["usb", "nfc", "ble", "hybrid", "internal"],
            }],
        });
        let ctx = parse_get_request(&v.to_string()).unwrap();
        assert_eq!(ctx.allow_credential_ids, vec![vec![5, 6, 7]]);
    }

    #[test]
    fn get_never_widens_a_restricted_allow_list() {
        // Dropping unusable entries would turn "only these" into "any credential".
        for allow in [
            json!([{ "type": "public-key", "id": "%%%" }]),
            json!([{ "type": "public-key", "id": "" }]),
            json!([{ "type": "public-key" }]),
            json!([{ "type": "future-type", "id": "AQ" }]),
            json!("AQ"),
            json!(null),
        ] {
            let mut v = get_json(&[]);
            v["allowCredentials"] = allow.clone();
            assert_eq!(
                parse_get_request(&v.to_string()),
                Err(PasskeyError::InvalidRequest),
                "{allow}"
            );
        }
        // Other types next to a usable id are skipped.
        let mut v = get_json(&[]);
        v["allowCredentials"] = json!([
            { "type": "future-type", "id": "AQ" },
            { "type": "public-key", "id": "Ag" },
        ]);
        let ctx = parse_get_request(&v.to_string()).unwrap();
        assert_eq!(ctx.allow_credential_ids, vec![vec![2]]);
    }

    #[test]
    fn get_enforces_size_caps() {
        let ids: Vec<Vec<u8>> = (0..=MAX_CREDENTIAL_LIST_ITEMS)
            .map(|i| vec![i as u8])
            .collect();
        let refs: Vec<&[u8]> = ids.iter().map(Vec::as_slice).collect();
        assert_eq!(
            parse_get_request(&get_json(&refs).to_string()),
            Err(PasskeyError::InvalidRequest)
        );
        assert!(
            parse_get_request(&get_json(&refs[..MAX_CREDENTIAL_LIST_ITEMS]).to_string()).is_ok()
        );
        let long = [9u8; MAX_CREDENTIAL_ID_BYTES + 1];
        assert_eq!(
            parse_get_request(&get_json(&[&long]).to_string()),
            Err(PasskeyError::InvalidRequest)
        );
        assert!(
            parse_get_request(&get_json(&[&long[..MAX_CREDENTIAL_ID_BYTES]]).to_string()).is_ok()
        );
        let mut v = get_json(&[]);
        v["challenge"] = json!(b64url::encode(&[1u8; 15]));
        assert_eq!(
            parse_get_request(&v.to_string()),
            Err(PasskeyError::InvalidRequest)
        );
        let mut v = get_json(&[]);
        v["rpId"] = json!("a".repeat(254));
        assert_eq!(
            parse_get_request(&v.to_string()),
            Err(PasskeyError::InvalidRequest)
        );
    }

    #[test]
    fn chrome_shaped_fixtures_parse() {
        let create = parse_create_request(include_str!(
            "../../tests/fixtures/passkey/chrome_create_request.json"
        ))
        .unwrap();
        assert_eq!(create.rp_id.as_deref(), Some("example.com"));
        assert_eq!(create.rp_name, "Example");
        assert_eq!(create.user_name, "alice@example.com");
        assert_eq!(create.user_display_name, "Alice");
        assert_eq!(create.challenge.len(), 32);
        assert_eq!(create.user_id.len(), 16);
        assert_eq!(create.exclude_credential_ids.len(), 1);
        assert_eq!(create.exclude_credential_ids[0].len(), 32);
        assert_eq!(create.attestation, "none");

        let get = parse_get_request(include_str!(
            "../../tests/fixtures/passkey/chrome_get_request.json"
        ))
        .unwrap();
        assert_eq!(get.rp_id.as_deref(), Some("example.com"));
        assert_eq!(get.challenge.len(), 32);
        assert_eq!(get.allow_credential_ids.len(), 1);
        assert_eq!(
            get.allow_credential_ids[0],
            create.exclude_credential_ids[0]
        );
    }
}
