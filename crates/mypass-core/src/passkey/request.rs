//! Parsing of the WebAuthn request JSON that Chrome hands to the extension's
//! `webAuthenticationProxy` (`requestDetailsJson`), for `create` and `get`.
//!
//! The page controls every value in it, so each field is bounded before any work: the strings
//! land in entry titles, user names and the pending ceremony table. Anything over a cap or with
//! characters the UI must not show is `InvalidRequest`; nothing over a cap is stored or echoed.

use super::origin::{validate_origin_in, ValidatedOrigin};
use super::{b64url, PasskeyError};
use crate::text::{is_clean_line, MAX_TITLE_CHARS, MAX_URL_CHARS, MAX_USERNAME_CHARS};
use serde_json::{Map, Value};
use std::fmt;

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

/// A parsed `navigator.credentials.create()` request, without its relying party id.
///
/// The page's raw `rp.id` is deliberately not kept: the only rpId is the one bound to the
/// request in [`ValidatedCreate::rp_id`]. `Debug` prints sizes only (the challenge, user handle
/// and names are page data and personal data, never log material).
#[derive(Clone, PartialEq, Eq)]
pub struct CreateContext {
    /// `rp.name`, falling back to the validated rpId.
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

impl fmt::Debug for CreateContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateContext")
            .field("challenge_len", &self.challenge.len())
            .field("user_id_len", &self.user_id.len())
            .field("exclude_credentials", &self.exclude_credential_ids.len())
            .field("attestation", &self.attestation)
            .finish_non_exhaustive()
    }
}

/// A parsed `navigator.credentials.get()` request, without its relying party id (see
/// [`CreateContext`]). `Debug` prints sizes only.
#[derive(Clone, PartialEq, Eq)]
pub struct GetContext {
    /// The challenge bytes.
    pub challenge: Vec<u8>,
    /// Decoded ids of the `public-key` entries of `allowCredentials`; empty means any credential
    /// of the relying party.
    pub allow_credential_ids: Vec<Vec<u8>>,
}

impl fmt::Debug for GetContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GetContext")
            .field("challenge_len", &self.challenge.len())
            .field("allow_credentials", &self.allow_credential_ids.len())
            .finish()
    }
}

/// A create request bound to the origin it was validated for. The only way to get one is
/// [`parse_create`], which parses the JSON once and validates the origin and rpId against it.
///
/// Hashing (`rpIdHash`), storing (`KPEX_PASSKEY_RELYING_PARTY`) and comparing relying party ids
/// use only [`ValidatedCreate::rp_id`], the canonical lowercase ACE rpId that was authorised for
/// the origin; the page's raw `rp.id` is not reachable from here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValidatedCreate {
    request: CreateContext,
    origin: ValidatedOrigin,
}

impl ValidatedCreate {
    /// The parsed request fields.
    #[must_use]
    pub fn request(&self) -> &CreateContext {
        &self.request
    }
    /// The validated origin (`clientDataJSON.origin`) and its authorised rpId.
    #[must_use]
    pub fn origin(&self) -> &ValidatedOrigin {
        &self.origin
    }
    /// The canonical rpId authorised for the origin: the only rpId to hash, store or compare.
    #[must_use]
    pub fn rp_id(&self) -> &str {
        self.origin.rp_id()
    }
}

/// A get request bound to the origin it was validated for; see [`ValidatedCreate`]. Hashing,
/// storing and comparing relying party ids use only [`ValidatedGet::rp_id`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValidatedGet {
    request: GetContext,
    origin: ValidatedOrigin,
}

impl ValidatedGet {
    /// The parsed request fields.
    #[must_use]
    pub fn request(&self) -> &GetContext {
        &self.request
    }
    /// The validated origin (`clientDataJSON.origin`) and its authorised rpId.
    #[must_use]
    pub fn origin(&self) -> &ValidatedOrigin {
        &self.origin
    }
    /// The canonical rpId authorised for the origin: the only rpId to hash, store or compare.
    #[must_use]
    pub fn rp_id(&self) -> &str {
        self.origin.rp_id()
    }
}

/// Parse a create request and validate its origin and rpId, parsing the JSON once. Ports
/// `PasskeyCeremony.parseCreateRequest` followed by `validateOrigin`.
///
/// # Errors
/// From the request: `UnsupportedAlgorithm` when `pubKeyCredParams` is non-empty and offers no
/// ES256 `public-key` entry; `InvalidRequest` for everything else that is missing, malformed or
/// over a cap. Then from the origin check: `OriginMismatch`, `CrossOrigin`, `RpIdNotAuthorized`
/// (see [`super::origin`]).
pub fn parse_create(origin_url: &str, json: &str) -> Result<ValidatedCreate, PasskeyError> {
    check_origin_len(origin_url)?;
    let root = parse_object(json)?;
    let challenge = challenge(&root)?;

    let rp = root
        .get("rp")
        .and_then(Value::as_object)
        .ok_or(PasskeyError::InvalidRequest)?;
    let rp_id = optional_rp_id(rp.get("id"))?;
    let rp_name = optional_string(rp.get("name"))?;
    if rp_name
        .as_deref()
        .is_some_and(|name| !is_clean_line(name, MAX_TITLE_CHARS))
    {
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
    let user_name = optional_string(user.get("name"))?.unwrap_or_default();
    let user_display_name =
        optional_string(user.get("displayName"))?.unwrap_or_else(|| user_name.clone());
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

    let origin = validate_origin_in(&root, origin_url, rp_id.as_deref())?;
    let rp_name = rp_name.unwrap_or_else(|| origin.rp_id().to_owned());
    Ok(ValidatedCreate {
        request: CreateContext {
            rp_name,
            challenge,
            user_id,
            user_name,
            user_display_name,
            exclude_credential_ids,
            attestation,
        },
        origin,
    })
}

/// Parse a get request and validate its origin and rpId, parsing the JSON once. Ports
/// `PasskeyCeremony.parseGetRequest` followed by `validateOrigin`.
///
/// `allowCredentials` is held to the same descriptor rules as `excludeCredentials`: a malformed
/// list fails the request, and so does a non-empty list none of whose entries is a `public-key`
/// credential. The Swift app dropped undecodable entries, which could turn a list naming only
/// unusable ids into an empty one, and an empty list means "any credential of the relying party".
///
/// # Errors
/// `InvalidRequest` when anything is missing, malformed or over a cap; then the origin errors of
/// [`parse_create`].
pub fn parse_get(origin_url: &str, json: &str) -> Result<ValidatedGet, PasskeyError> {
    check_origin_len(origin_url)?;
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
    let origin = validate_origin_in(&root, origin_url, rp_id.as_deref())?;
    Ok(ValidatedGet {
        request: GetContext {
            challenge,
            allow_credential_ids,
        },
        origin,
    })
}

/// The origin URL is page-reachable input too: refuse an oversized one before any work.
fn check_origin_len(origin_url: &str) -> Result<(), PasskeyError> {
    if origin_url.chars().count() > MAX_URL_CHARS {
        Err(PasskeyError::InvalidRequest)
    } else {
        Ok(())
    }
}

/// An absent member is `None`; a present one must be a string.
fn optional_string(value: Option<&Value>) -> Result<Option<String>, PasskeyError> {
    match value {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(PasskeyError::InvalidRequest),
    }
}

/// The request JSON as an object, refusing oversized input before parsing it.
fn parse_object(json: &str) -> Result<Map<String, Value>, PasskeyError> {
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

    const ORIGIN: &str = "https://example.com";

    /// Stamps the Chrome origin extension into an object payload (other input is left as is).
    fn stamp(json: &str) -> String {
        match serde_json::from_str::<Value>(json) {
            Ok(Value::Object(mut root)) => {
                root.insert(
                    "extensions".to_owned(),
                    json!({ "remoteDesktopClientOverride":
                        { "origin": ORIGIN, "sameOriginWithAncestors": true } }),
                );
                Value::Object(root).to_string()
            }
            _ => json.to_owned(),
        }
    }

    fn parse_create_request(json: &str) -> Result<ValidatedCreate, PasskeyError> {
        parse_create(ORIGIN, &stamp(json))
    }

    fn parse_get_request(json: &str) -> Result<ValidatedGet, PasskeyError> {
        parse_get(ORIGIN, &stamp(json))
    }

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

    fn create(v: &Value) -> Result<ValidatedCreate, PasskeyError> {
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
        assert_eq!(ctx.rp_id(), "example.com");
        assert_eq!(ctx.request().rp_name, "Example");
        assert_eq!(ctx.request().user_name, "alice");
        assert_eq!(ctx.request().user_display_name, "Alice");
        assert_eq!(ctx.request().challenge, challenge16());
        assert_eq!(ctx.request().user_id, vec![1, 2, 3, 4]);
        assert_eq!(ctx.request().attestation, "none");
        assert!(ctx.request().exclude_credential_ids.is_empty());
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
            ctx.request().exclude_credential_ids,
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
        assert_eq!(ctx.request().attestation, "none");
        assert_eq!(
            ctx.request().rp_name,
            "example.com",
            "rp name falls back to rp id"
        );
        assert_eq!(
            ctx.request().user_display_name,
            "alice",
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
            assert_eq!(ctx.request().attestation, want, "{given}");
        }
        let mut v = create_json(&[-7], None);
        v["attestation"] = json!(5);
        assert_eq!(create(&v).unwrap().request().attestation, "none");
    }

    #[test]
    fn create_without_rp_id_binds_the_origin_host() {
        let mut v = create_json(&[-7], None);
        v["rp"] = json!({ "name": "Example" });
        let ctx = create(&v).unwrap();
        assert_eq!(ctx.rp_id(), "example.com", "the origin host");
        assert_eq!(ctx.request().rp_name, "Example");
        v["rp"] = json!({});
        assert_eq!(create(&v).unwrap().request().rp_name, "example.com");
        v["rp"] = json!({ "id": 5 });
        assert_eq!(create(&v), Err(PasskeyError::InvalidRequest));
        // An rpId the origin rules refuse never becomes a fallback name.
        v["rp"] = json!({ "id": "a\nb" });
        assert_eq!(create(&v), Err(PasskeyError::RpIdNotAuthorized));
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
                "rp name not a string",
                Box::new(|v| v["rp"]["name"] = json!(5)),
            ),
            (
                "user name not a string",
                Box::new(|v| v["user"]["name"] = json!(null)),
            ),
            (
                "display name not a string",
                Box::new(|v| v["user"]["displayName"] = json!(["Alice"])),
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
        assert_eq!(ctx.rp_id(), "example.com");
        assert_eq!(ctx.request().challenge, challenge16());
        assert_eq!(ctx.request().allow_credential_ids, vec![vec![5, 6, 7]]);

        let mut v = get_json(&[]);
        v.as_object_mut().unwrap().remove("allowCredentials");
        let ctx = parse_get_request(&v.to_string()).unwrap();
        assert!(ctx.request().allow_credential_ids.is_empty());
        v.as_object_mut().unwrap().remove("rpId");
        assert_eq!(
            parse_get_request(&v.to_string()).unwrap().rp_id(),
            "example.com"
        );
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
        assert_eq!(ctx.request().allow_credential_ids, vec![vec![5, 6, 7]]);
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
        assert_eq!(ctx.request().allow_credential_ids, vec![vec![2]]);
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
    fn debug_prints_sizes_not_page_data() {
        let create = create(&create_json(&[-7], None)).unwrap();
        let get = parse_get_request(&get_json(&[&[5, 6, 7]]).to_string()).unwrap();
        for text in [format!("{create:?}"), format!("{get:?}")] {
            for leak in [
                "alice",
                "Alice",
                "Example\"",
                "AAECAwQFBgcICQoLDA0ODw",
                "[0, 1, 2",
            ] {
                assert!(!text.contains(leak), "{leak} in {text}");
            }
            assert!(text.contains("example.com"), "{text}");
        }
    }

    #[test]
    fn chrome_shaped_fixtures_parse() {
        let create = parse_create_request(include_str!(
            "../../tests/fixtures/passkey/chrome_create_request.json"
        ))
        .unwrap();
        assert_eq!(create.rp_id(), "example.com");
        assert_eq!(create.request().rp_name, "Example");
        assert_eq!(create.request().user_name, "alice@example.com");
        assert_eq!(create.request().user_display_name, "Alice");
        assert_eq!(create.request().challenge.len(), 32);
        assert_eq!(create.request().user_id.len(), 16);
        assert_eq!(create.request().exclude_credential_ids.len(), 1);
        assert_eq!(create.request().exclude_credential_ids[0].len(), 32);
        assert_eq!(create.request().attestation, "none");

        let get = parse_get_request(include_str!(
            "../../tests/fixtures/passkey/chrome_get_request.json"
        ))
        .unwrap();
        assert_eq!(get.rp_id(), "example.com");
        assert_eq!(get.request().challenge.len(), 32);
        assert_eq!(get.request().allow_credential_ids.len(), 1);
        assert_eq!(
            get.request().allow_credential_ids[0],
            create.request().exclude_credential_ids[0]
        );
    }
}
