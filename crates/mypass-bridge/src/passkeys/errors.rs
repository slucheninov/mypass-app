//! The WebAuthn errors the extension rethrows into the page: a completion with no response and
//! a `DOMException` name and message.

#![deny(clippy::wildcard_enum_match_arm)]

use mypass_core::passkey::PasskeyError;

use crate::protocol::{PasskeyErrorDetails, Response};

/// `NotAllowedError`: the name for every refusal the page must not learn the reason of.
pub(crate) const NOT_ALLOWED: &str = "NotAllowedError";
/// `InvalidStateError`: the authenticator already holds a credential it must not overwrite.
pub(crate) const INVALID_STATE: &str = "InvalidStateError";
/// `SecurityError`: the origin or the relying party id is not acceptable.
pub(crate) const SECURITY: &str = "SecurityError";
/// `NotSupportedError`: no requested algorithm is supported.
pub(crate) const NOT_SUPPORTED: &str = "NotSupportedError";

/// The page error (`DOMException` name, fixed message) for a refused request. Exhaustive, so a
/// new `PasskeyError` is a compile error until it is mapped.
///
/// - Origin and rpId failures are `SecurityError` (WebAuthn's answer to an rpId that is not a
///   registrable suffix of the origin), with the Swift app's messages.
/// - A request from a frame that is not same-origin with its ancestors is `NotAllowedError`: that
///   is what WebAuthn answers when the frame lacks the `publickey-credentials-*` permission, and
///   MyPass never serves a framed ceremony.
/// - `CredentialNotAllowed` (the key is not in `allowCredentials`) is `NotAllowedError`, as for
///   any credential the authenticator cannot use.
/// - Everything else is `NotAllowedError` "Invalid passkey request".
pub(crate) fn page_error(e: &PasskeyError) -> (&'static str, &'static str) {
    match e {
        PasskeyError::UnsupportedAlgorithm => (NOT_SUPPORTED, "MyPass passkeys require ES256"),
        PasskeyError::OriginMismatch => (SECURITY, "Passkey request origin could not be verified"),
        PasskeyError::RpIdNotAuthorized => (
            SECURITY,
            "This site is not authorized for the requested passkey relying party",
        ),
        PasskeyError::CrossOrigin => (
            NOT_ALLOWED,
            "Passkeys are not available in embedded cross-origin frames",
        ),
        PasskeyError::CredentialNotAllowed => {
            (NOT_ALLOWED, "The passkey is not one the site asked for")
        }
        PasskeyError::InvalidRequest | PasskeyError::Crypto => {
            (NOT_ALLOWED, "Invalid passkey request")
        }
    }
}

fn details(name: &str, message: &str) -> Option<PasskeyErrorDetails> {
    Some(PasskeyErrorDetails {
        name: name.to_owned(),
        message: message.to_owned(),
    })
}

/// A failed create ceremony (`passkeyCreateComplete` with `responseJson` absent).
pub(crate) fn create_error(id: i64, name: &str, message: &str) -> Response {
    Response::PasskeyCreateComplete {
        request_id: id,
        response_json: None,
        error: details(name, message),
    }
}

/// A failed get ceremony (`passkeyGetComplete` with `responseJson` absent).
pub(crate) fn get_error(id: i64, name: &str, message: &str) -> Response {
    Response::PasskeyGetComplete {
        request_id: id,
        response_json: None,
        error: details(name, message),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_serialise_without_response_json_and_are_not_served() {
        let c = create_error(3, NOT_ALLOWED, "No pending request");
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({"type": "passkeyCreateComplete", "requestId": 3,
                   "error": {"name": "NotAllowedError", "message": "No pending request"}})
        );
        let g = get_error(-1, "SecurityError", "x");
        assert_eq!(
            serde_json::to_value(&g).unwrap(),
            json!({"type": "passkeyGetComplete", "requestId": -1,
                   "error": {"name": "SecurityError", "message": "x"}})
        );
        assert!(!c.was_served());
        assert!(!g.was_served());
    }

    #[test]
    fn page_errors_follow_webauthn_names() {
        for (e, name) in [
            (PasskeyError::UnsupportedAlgorithm, "NotSupportedError"),
            (PasskeyError::OriginMismatch, "SecurityError"),
            (PasskeyError::RpIdNotAuthorized, "SecurityError"),
            (PasskeyError::CrossOrigin, "NotAllowedError"),
            (PasskeyError::CredentialNotAllowed, "NotAllowedError"),
            (PasskeyError::InvalidRequest, "NotAllowedError"),
            (PasskeyError::Crypto, "NotAllowedError"),
        ] {
            let (got, message) = page_error(&e);
            assert_eq!(got, name, "{e:?}");
            assert!(!message.is_empty());
        }
        assert_eq!(
            page_error(&PasskeyError::InvalidRequest).1,
            "Invalid passkey request"
        );
    }
}
