//! Origin and relying-party-id validation for passkey ceremonies.
//!
//! Nothing signs for an origin that did not pass this validation: the signing API takes only a
//! [`ValidatedOrigin`] (inside a `ValidatedCreate` or `ValidatedGet` from
//! [`parse_create`](super::request::parse_create) / [`parse_get`](super::request::parse_get)),
//! and that type has no public constructor.
//!
//! ```
//! use mypass_core::passkey::{parse_get, ValidatedOrigin};
//! let json = r#"{"challenge":"AAECAwQFBgcICQoLDA0ODw","rpId":"EXAMPLE.com",
//!     "extensions":{"remoteDesktopClientOverride":
//!         {"origin":"https://login.example.com","sameOriginWithAncestors":true}}}"#;
//! let get = parse_get("https://login.example.com", json).unwrap();
//! let origin: &ValidatedOrigin = get.origin();
//! assert_eq!(origin.as_str(), "https://login.example.com");
//! assert_eq!(get.rp_id(), "example.com");
//! ```
//!
//! A `ValidatedOrigin` cannot be built from a string by hand:
//!
//! ```compile_fail,E0451
//! use mypass_core::passkey::ValidatedOrigin;
//! let forged = ValidatedOrigin { origin: "https://evil.example".into(), rp_id: "bank.com".into() };
//! ```
//!
//! ```compile_fail,E0277
//! use mypass_core::passkey::ValidatedOrigin;
//! let forged = ValidatedOrigin::from(String::from("https://evil.example"));
//! ```
//!
//! Nor can a bound request (its fields are private); reading its parts is fine:
//!
//! ```
//! use mypass_core::passkey::{GetContext, ValidatedGet, ValidatedOrigin};
//! fn parts(get: &ValidatedGet) -> (GetContext, ValidatedOrigin) {
//!     (get.request().clone(), get.origin().clone())
//! }
//! ```
//!
//! ```compile_fail,E0451
//! use mypass_core::passkey::{GetContext, ValidatedGet, ValidatedOrigin};
//! fn forge(request: GetContext, origin: ValidatedOrigin) -> ValidatedGet {
//!     ValidatedGet { request, origin }
//! }
//! ```

use super::PasskeyError;
use crate::psl::{canonical_host, registrable_domain};
use serde_json::{Map, Value};
use url::{Host, Url};

/// An origin that passed origin and rpId validation, with the canonical rpId it was authorised for.
///
/// The origin keeps its port: it goes into `clientDataJSON.origin`, while the port never takes
/// part in rpId matching. The rpId is the lowercase ACE form, which is what gets hashed into
/// `rpIdHash` and stored with the credential.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValidatedOrigin {
    origin: String,
    rp_id: String,
}

impl ValidatedOrigin {
    /// The origin as serialised (`https://host[:port]`), exactly as the page's browser sent it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.origin
    }

    /// The canonical (lowercase ACE) rpId that was authorised for this origin.
    #[must_use]
    pub fn rp_id(&self) -> &str {
        &self.rp_id
    }
}

/// Validates the origin of a passkey ceremony instead of trusting whatever the extension sent as
/// `origin_url`: on its own that is just a string the message carried, and the ceremony stamps it
/// verbatim into `clientDataJSON.origin`, which is exactly the value a relying party trusts when
/// verifying the signature we hand back. Two independent checks, both required, plus the
/// hardening below.
///
/// Check 1 compares `origin_url` against `extensions.remoteDesktopClientOverride.origin` inside
/// the same request payload (the parsed `root`), failing closed when the field is absent. Chrome
/// injects that field; the extension's `background.js` (`originFromRequestDetails()`) only reads
/// it and sends it on as `origin_url`. This check is weak on its own: a fully compromised
/// extension controls both copies and can simply lie in agreement (and today the extension takes
/// `origin_url` from that very field, so the two agree by construction). What it actually buys is cheap: it stops
/// a *partially* wrong message (a bug or a rewrite of only one of the two fields in transit) and
/// keeps the two copies from silently drifting apart, which would otherwise mask the exact class
/// of bug this hardening exists to catch.
///
/// Check 2 is the real gate: it enforces the WebAuthn RP ID invariant that an origin may only
/// assert relying-party ids equal to its own host or a registrable domain of it. Without this
/// check the app would happily sign an assertion binding an attacker-chosen origin to an
/// attacker-chosen relying party, e.g. a page at `evil.example` claiming `rpId: "bank.com"`,
/// which is precisely the cross-origin confusion WebAuthn's RP ID scoping exists to prevent. The
/// Public Suffix List is what stops an rpId from climbing past the registrable domain: an origin
/// at `evil.co.uk` must not be able to claim `rpId: "co.uk"` just because `"evil.co.uk"` ends
/// with `".co.uk"`; `"co.uk"` is a public suffix, not a domain anyone registers under.
///
/// Hardening over the Swift app:
/// - `remoteDesktopClientOverride.sameOriginWithAncestors` must be exactly `true`; absent, `false`
///   or any other value is `CrossOrigin`. A ceremony from a cross-origin iframe would otherwise be
///   signed with `clientDataJSON.crossOrigin: false` (the proxy payload carries no top origin to
///   report), and a relying party that refuses framed ceremonies would be told it was top level;
/// - the origin must be a bare `https` origin (no userinfo, path, query or fragment; its
///   serialisation equals the input) whose host is a domain, not an IP literal, with no trailing
///   dot. There is no `localhost` exception (neither `http` nor `https`, nor `*.localhost`):
///   credentials are offered only on `https` pages (AGENTS.md), whatever Chrome considers a
///   secure context;
/// - host and rpId are compared in canonical lowercase ACE form (IDNA), so case and Unicode
///   spellings cannot split one site in two, and a Unicode-spelled public suffix cannot be
///   claimed through its ACE form;
/// - the rpId must have a registrable domain of its own, so it is never a public suffix (this
///   also rules out `localhost`);
/// - an rpId with a port, path, userinfo, whitespace, a trailing dot or empty labels is refused.
///
/// `rp_id` is the rpId the request named, or `None` when it omitted it; WebAuthn then uses the
/// origin's host.
///
/// The size caps on `origin_url` and the JSON are enforced by the callers before parsing.
///
/// # Errors
/// `OriginMismatch` when check 1 fails; `CrossOrigin` when the request does not come from a
/// frame same-origin with all its ancestors; `RpIdNotAuthorized` for every other refusal.
pub(crate) fn validate_origin_in(
    root: &Map<String, Value>,
    origin_url: &str,
    rp_id: Option<&str>,
) -> Result<ValidatedOrigin, PasskeyError> {
    let client_override = root
        .get("extensions")
        .and_then(|e| e.get("remoteDesktopClientOverride"));

    // Check 1: the wire origin must match the payload's own origin, byte for byte.
    let payload_origin = client_override
        .and_then(|o| o.get("origin"))
        .and_then(Value::as_str)
        .ok_or(PasskeyError::OriginMismatch)?;
    if payload_origin != origin_url {
        return Err(PasskeyError::OriginMismatch);
    }

    // Only top-level or same-origin-framed ceremonies: we cannot report crossOrigin truthfully.
    let same_origin_with_ancestors = client_override
        .and_then(|o| o.get("sameOriginWithAncestors"))
        .and_then(Value::as_bool);
    if same_origin_with_ancestors != Some(true) {
        return Err(PasskeyError::CrossOrigin);
    }

    // Check 2: a bare https origin on a domain, and an rpId it may claim.
    let host = origin_host(origin_url).ok_or(PasskeyError::RpIdNotAuthorized)?;
    let rp = match rp_id {
        Some(requested) => canonical_rp_id(requested).ok_or(PasskeyError::RpIdNotAuthorized)?,
        None => host.clone(),
    };
    if !rp_id_is_authorised(&host, &rp) {
        return Err(PasskeyError::RpIdNotAuthorized);
    }
    Ok(ValidatedOrigin {
        origin: origin_url.to_owned(),
        rp_id: rp,
    })
}

/// The canonical host of a bare `https` origin on a domain, or `None`.
fn origin_host(origin_url: &str) -> Option<String> {
    let url = Url::parse(origin_url).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        // The serialisation equals the input: no Unicode or upper-case spelling, no default
        // port, no trailing slash; the origin goes into clientDataJSON exactly as given.
        || url.origin().ascii_serialization() != origin_url
    {
        return None;
    }
    let domain = match url.host()? {
        Host::Domain(domain) => domain,
        Host::Ipv4(_) | Host::Ipv6(_) => return None,
    };
    if domain.ends_with('.') {
        return None;
    }
    let host = canonical_host(domain)?;
    // No localhost exception, not even over https: `localhost` and `*.localhost` (RFC 6761)
    // resolve to this machine, so any local process could stand in for the relying party.
    if host == "localhost" || host.ends_with(".localhost") {
        return None;
    }
    Some(host)
}

/// The canonical (lowercase ACE) form of a requested rpId, or `None` when it is not a plain
/// domain name.
fn canonical_rp_id(rp_id: &str) -> Option<String> {
    if rp_id.is_empty()
        || rp_id.ends_with('.')
        || rp_id.chars().any(|c| {
            matches!(c, ':' | '/' | '@' | '\\' | '?' | '#' | '%' | '[' | ']')
                || c.is_whitespace()
                || c.is_control()
        })
    {
        return None;
    }
    let canonical = canonical_host(rp_id)?;
    // ACE output is letters, digits, hyphens, underscores and dots; anything else is not a host.
    canonical
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .then_some(canonical)
}

/// The WebAuthn rule over canonical names: the rpId is the host itself, or a parent of the host
/// that is the host's registrable domain or lies below it. The rpId is never a public suffix.
fn rp_id_is_authorised(host: &str, rp: &str) -> bool {
    if registrable_domain(rp).is_none() {
        return false;
    }
    if rp == host {
        return true;
    }
    let Some(parent_of) = host.strip_suffix(rp) else {
        return false;
    };
    if !parent_of.ends_with('.') {
        return false;
    }
    match registrable_domain(host) {
        Some(registrable) => {
            rp == registrable
                || rp
                    .strip_suffix(registrable.as_str())
                    .is_some_and(|prefix| prefix.ends_with('.'))
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    // Test-only support code: assertions may unwrap known fixtures.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::passkey::request::{parse_create, parse_get, MAX_REQUEST_JSON_BYTES};
    use crate::text::MAX_URL_CHARS;

    /// A request payload whose `extensions.remoteDesktopClientOverride.origin` is `origin`, sent
    /// from a top-level frame; `None` omits the whole `extensions` block, as in a payload Chrome
    /// never stamped.
    fn origin_json(origin: Option<&str>) -> String {
        match origin {
            None => "{}".to_owned(),
            Some(o) => serde_json::json!({
                "extensions": {
                    "remoteDesktopClientOverride": { "origin": o, "sameOriginWithAncestors": true }
                }
            })
            .to_string(),
        }
    }

    /// Runs the origin rules alone over an already parsed payload.
    fn validate_origin(
        origin_url: &str,
        json: &str,
        rp_id: Option<&str>,
    ) -> Result<ValidatedOrigin, PasskeyError> {
        match serde_json::from_str::<Value>(json) {
            Ok(Value::Object(root)) => validate_origin_in(&root, origin_url, rp_id),
            _ => Err(PasskeyError::InvalidRequest),
        }
    }

    /// A get request for `rp_id` whose payload names `payload_origin`.
    fn get_json(payload_origin: &str, rp_id: Option<&str>) -> serde_json::Value {
        let mut v: serde_json::Value =
            serde_json::from_str(&origin_json(Some(payload_origin))).unwrap();
        v["challenge"] = serde_json::json!("AAECAwQFBgcICQoLDA0ODw");
        if let Some(rp) = rp_id {
            v["rpId"] = serde_json::json!(rp);
        }
        v
    }

    fn check(origin: &str, rp_id: &str) -> Result<ValidatedOrigin, PasskeyError> {
        validate_origin(origin, &origin_json(Some(origin)), Some(rp_id))
    }

    fn refused(origin: &str, rp_id: &str) {
        assert_eq!(
            check(origin, rp_id),
            Err(PasskeyError::RpIdNotAuthorized),
            "{origin} / {rp_id}"
        );
    }

    #[test]
    fn origin_exact_rp_id_match() {
        let v = check("https://example.com", "example.com").unwrap();
        assert_eq!(v.as_str(), "https://example.com");
        assert_eq!(v.rp_id(), "example.com");
    }

    #[test]
    fn origin_with_port_keeps_the_port_out_of_rp_id_matching() {
        let v = check("https://example.com:8443", "example.com").unwrap();
        assert_eq!(v.as_str(), "https://example.com:8443");
        assert_eq!(v.rp_id(), "example.com");
    }

    #[test]
    fn origin_localhost_http_with_port_is_rejected() {
        // R3: https only, no localhost exception (the Swift app accepted this one).
        refused("http://localhost:8765", "localhost");
        refused("https://localhost:8765", "localhost");
        refused("https://localhost", "localhost");
        refused("http://app.localhost", "app.localhost");
        refused("https://app.localhost", "app.localhost");
    }

    #[test]
    fn origin_rp_id_on_parent_registrable_domain() {
        let v = check("https://login.example.com", "example.com").unwrap();
        assert_eq!(v.rp_id(), "example.com");
        assert!(check("https://a.b.example.com", "b.example.com").is_ok());
    }

    #[test]
    fn omitted_rp_id_defaults_to_the_origin_host() {
        let origin = "https://Login.Example.com";
        // An upper-case origin does not round-trip, so use the canonical spelling.
        assert!(validate_origin(origin, &origin_json(Some(origin)), None).is_err());
        let origin = "https://login.example.com";
        let v = validate_origin(origin, &origin_json(Some(origin)), None).unwrap();
        assert_eq!(v.rp_id(), "login.example.com");
        // A host that is a public suffix has no rpId to default to.
        let origin = "https://github.io";
        assert_eq!(
            validate_origin(origin, &origin_json(Some(origin)), None),
            Err(PasskeyError::RpIdNotAuthorized)
        );
    }

    #[test]
    fn own_rp_id_under_unicode_private_suffix() {
        // The bundled PSL spells the suffix in Unicode; origins and rpIds arrive as ACE.
        let origin = "https://login.bank.xn--55qx5d.hk";
        for rp_id in ["bank.xn--55qx5d.hk", "login.bank.xn--55qx5d.hk"] {
            assert_eq!(check(origin, rp_id).unwrap().rp_id(), rp_id);
        }
    }

    #[test]
    fn uppercase_rp_id_and_origin_host_are_canonicalised() {
        let v = check("https://login.example.com", "EXAMPLE.Com").unwrap();
        assert_eq!(v.rp_id(), "example.com");
        // The URL parser lowercases the host, so an upper-case origin is not a canonical
        // serialisation and is refused rather than silently rewritten into clientDataJSON.
        refused("https://EXAMPLE.com", "example.com");
    }

    #[test]
    fn unicode_rp_id_matches_ace_origin() {
        let v = check("https://xn--bcher-kva.example", "b\u{fc}cher.example").unwrap();
        assert_eq!(v.rp_id(), "xn--bcher-kva.example");
        assert!(check("https://xn--bcher-kva.example", "xn--bcher-kva.example").is_ok());
    }

    #[test]
    fn wire_origin_disagreeing_with_payload_is_origin_mismatch() {
        let json = origin_json(Some("https://example.com"));
        for wire in [
            "https://attacker.example",
            "https://example.com/",
            "https://Example.com",
            "https://example.com ",
        ] {
            assert_eq!(
                validate_origin(wire, &json, Some("example.com")),
                Err(PasskeyError::OriginMismatch),
                "{wire}"
            );
        }
    }

    #[test]
    fn missing_remote_desktop_client_override_fails_closed() {
        let o = "https://example.com";
        for json in [
            origin_json(None),
            r#"{"extensions":{}}"#.to_owned(),
            r#"{"extensions":{"remoteDesktopClientOverride":{}}}"#.to_owned(),
            r#"{"extensions":{"remoteDesktopClientOverride":{"origin":5}}}"#.to_owned(),
            r#"{"extensions":{"remoteDesktopClientOverride":"https://example.com"}}"#.to_owned(),
        ] {
            assert_eq!(
                validate_origin(o, &json, Some("example.com")),
                Err(PasskeyError::OriginMismatch),
                "{json}"
            );
        }
    }

    #[test]
    fn cross_origin_or_unknown_frame_context_is_refused() {
        let o = "https://example.com";
        for same_origin in [
            Some(serde_json::json!(false)),
            None,
            Some(serde_json::json!("true")),
            Some(serde_json::json!(1)),
            Some(serde_json::Value::Null),
        ] {
            let mut v = get_json(o, Some("example.com"));
            let rdco = &mut v["extensions"]["remoteDesktopClientOverride"];
            match &same_origin {
                Some(value) => rdco["sameOriginWithAncestors"] = value.clone(),
                None => {
                    rdco.as_object_mut()
                        .unwrap()
                        .remove("sameOriginWithAncestors");
                }
            }
            assert_eq!(
                parse_get(o, &v.to_string()),
                Err(PasskeyError::CrossOrigin),
                "{same_origin:?}"
            );
        }
        assert!(parse_get(o, &get_json(o, Some("example.com")).to_string()).is_ok());
    }

    #[test]
    fn unrelated_rp_id() {
        refused("https://example.com", "evil.com");
        refused("https://evil.example", "bank.com");
    }

    #[test]
    fn rp_id_climbing_past_public_suffix() {
        refused("https://evil.co.uk", "co.uk");
        refused("https://evil.co.uk", "uk");
        refused("https://user.github.io", "github.io");
    }

    #[test]
    fn rp_id_equal_to_unicode_suffix_of_ace_origin() {
        refused("https://evil.xn--55qx5d.hk", "xn--55qx5d.hk");
        refused("https://evil.xn--55qx5d.hk", "\u{516c}\u{53f8}.hk");
    }

    #[test]
    fn sibling_registrable_domain_under_unicode_suffix() {
        refused("https://evil.xn--55qx5d.hk", "bank.xn--55qx5d.hk");
    }

    #[test]
    fn idn_lookalike_origins_never_match_ascii_rp_id() {
        // Cyrillic small a (U+0430) followed by "pple.com", in ACE.
        refused("https://xn--pple-43d.com", "apple.com");
        // The same lookalike in Unicode form, mixed script, and full-width Latin a: none is a
        // canonical origin serialisation, so none reaches the rpId rule.
        refused("https://\u{430}pple.com", "apple.com");
        refused("https://\u{430}\u{440}ple.com", "apple.com");
        refused("https://\u{ff41}pple.com", "apple.com");
        // And the other way round: a lookalike rpId on the real origin.
        refused("https://apple.com", "\u{430}pple.com");
        refused("https://apple.com", "xn--pple-43d.com");
    }

    #[test]
    fn suffix_confusion_notexample_com() {
        refused("https://notexample.com", "example.com");
        refused("https://example.com.evil.com", "example.com");
    }

    #[test]
    fn rp_id_longer_than_host() {
        refused("https://example.com", "login.example.com");
    }

    #[test]
    fn http_non_localhost_is_rejected() {
        refused("http://example.com", "example.com");
        refused("http://example.com:443", "example.com");
    }

    #[test]
    fn non_web_schemes_rejected() {
        refused("file:///etc/passwd", "example.com");
        refused(
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop",
            "abcdefghijklmnopabcdefghijklmnop",
        );
        refused("data:text/html,hi", "example.com");
        refused("wss://example.com", "example.com");
        refused("example.com", "example.com");
    }

    #[test]
    fn userinfo_path_query_fragment_in_origin_rejected() {
        refused("https://example.com@evil.com", "example.com");
        refused("https://example.com@evil.com", "evil.com");
        refused("https://user:pw@example.com", "example.com");
        refused("https://example.com/x", "example.com");
        refused("https://example.com/", "example.com");
        refused("https://example.com?x", "example.com");
        refused("https://example.com#x", "example.com");
        refused("https://example.com:443", "example.com");
    }

    #[test]
    fn trailing_dot_host_rejected() {
        refused("https://example.com.", "example.com");
        refused("https://example.com.", "example.com.");
        refused("https://example.com", "example.com.");
    }

    #[test]
    fn ip_literal_hosts_rejected() {
        refused("https://127.0.0.1", "127.0.0.1");
        refused("https://[::1]", "[::1]");
        refused("https://[::1]", "::1");
        refused("https://192.168.1.10", "1.10");
    }

    #[test]
    fn rp_id_with_port_slash_or_empty_rejected() {
        for rp_id in [
            "",
            "example.com:443",
            "example.com/",
            "https://example.com",
            "user@example.com",
            "example .com",
            " example.com",
            "example..com",
            ".example.com",
            "exa%6dple.com",
            "example.com\n",
        ] {
            refused("https://example.com", rp_id);
        }
    }

    #[test]
    fn oversized_inputs_are_invalid_requests() {
        let long = format!("https://{}.com", "a".repeat(MAX_URL_CHARS));
        assert_eq!(
            parse_get(&long, &get_json(&long, None).to_string()),
            Err(PasskeyError::InvalidRequest)
        );
        let o = "https://example.com";
        let mut v = get_json(o, Some("example.com"));
        v["padding"] = serde_json::json!("x".repeat(MAX_REQUEST_JSON_BYTES));
        assert_eq!(
            parse_get(o, &v.to_string()),
            Err(PasskeyError::InvalidRequest)
        );
        for garbage in ["not json", "[]"] {
            assert_eq!(parse_get(o, garbage), Err(PasskeyError::InvalidRequest));
        }
    }

    #[test]
    fn bound_request_carries_only_the_canonical_rp_id() {
        let o = "https://login.example.com";
        let get = parse_get(o, &get_json(o, Some("EXAMPLE.Com")).to_string()).unwrap();
        assert_eq!(get.rp_id(), "example.com");
        assert_eq!(get.origin().rp_id(), "example.com");
        assert_eq!(get.origin().as_str(), o);
        // An omitted rpId binds the origin host.
        let get = parse_get(o, &get_json(o, None).to_string()).unwrap();
        assert_eq!(get.rp_id(), "login.example.com");
        // The origin rules run on the same parse: a refused rpId refuses the request.
        assert_eq!(
            parse_get(o, &get_json(o, Some("evil.com")).to_string()),
            Err(PasskeyError::RpIdNotAuthorized)
        );
        assert_eq!(
            parse_get(
                "https://evil.com",
                &get_json(o, Some("example.com")).to_string()
            ),
            Err(PasskeyError::OriginMismatch)
        );
    }

    #[test]
    fn chrome_shaped_fixtures_validate() {
        let create = include_str!("../../tests/fixtures/passkey/chrome_create_request.json");
        let get = include_str!("../../tests/fixtures/passkey/chrome_get_request.json");
        let create = parse_create("https://example.com", create).unwrap();
        assert_eq!(create.rp_id(), "example.com");
        assert_eq!(create.origin().as_str(), "https://example.com");
        let get = parse_get("https://example.com", get).unwrap();
        assert_eq!(get.rp_id(), "example.com");
    }
}
