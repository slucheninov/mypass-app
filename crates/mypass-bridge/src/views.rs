//! Wire helpers shared by the handlers: entry ids on the wire and the mapping of internal errors
//! to protocol responses.
//!
//! The mappings are exhaustive (no wildcard arm) and every message is a fixed string: no internal
//! error text, path, secret or store detail ever reaches the extension.

#![deny(clippy::wildcard_enum_match_arm)]

use mypass_session::session::SessionError;
use uuid::Uuid;

use crate::pairing::PairingError;
use crate::protocol::Response;

/// The id of an entry as the extension sees it: uppercase, hyphenated (the Swift `uuidString`).
#[must_use]
pub fn wire_id(id: Uuid) -> String {
    id.hyphenated()
        .encode_upper(&mut Uuid::encode_buffer())
        .to_owned()
}

/// Parses an id sent by the extension: hyphenated or simple form, either case.
#[must_use]
pub fn parse_wire_id(s: &str) -> Option<Uuid> {
    Uuid::parse_str(s.trim()).ok()
}

/// `Response::Error` with a fixed message.
#[must_use]
pub fn error(message: &str) -> Response {
    Response::Error {
        message: message.to_owned(),
    }
}

/// The protocol answer for a session failure. A locked or absent vault is `Locked` (the extension
/// then offers an unlock); everything else is a fixed, detail-free message.
#[must_use]
pub fn session_error_response(e: &SessionError) -> Response {
    match e {
        SessionError::Locked | SessionError::NoVault => Response::Locked,
        SessionError::Quitting => error("MyPass is quitting"),
        SessionError::NotFound => error("entry not found"),
        SessionError::InvalidInput(_) => error("invalid request"),
        SessionError::LimitExceeded(_) => error("limit exceeded"),
        SessionError::ChangedOnDisk => error("the vault file changed, try again"),
        SessionError::WrongPassword
        | SessionError::AlreadyExists
        | SessionError::Corrupt(_)
        | SessionError::Unsupported(_)
        | SessionError::Io(_)
        | SessionError::GeneratorRules(_) => error("the vault could not be used"),
    }
}

/// The protocol answer for a pairing store failure.
#[must_use]
pub fn pairing_error_response(e: &PairingError) -> Response {
    match e {
        PairingError::TooMany => error("too many paired browsers"),
        PairingError::Store(_) | PairingError::Corrupt => error("pairing is unavailable"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn wire_id_is_uppercase_hyphenated_and_parse_accepts_simple_and_lowercase() {
        let id = Uuid::parse_str("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0").unwrap();
        let wire = wire_id(id);
        assert_eq!(wire, "0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0");
        assert_eq!(parse_wire_id(&wire), Some(id));
        assert_eq!(
            parse_wire_id("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"),
            Some(id)
        );
        assert_eq!(parse_wire_id("0f1e2d3c4b5a69788796a5b4c3d2e1f0"), Some(id));
        assert_eq!(parse_wire_id("not an id"), None);
        assert_eq!(parse_wire_id(""), None);
    }

    #[test]
    fn session_errors_map_without_internal_detail() {
        assert_eq!(
            session_error_response(&SessionError::Locked),
            Response::Locked
        );
        assert_eq!(
            session_error_response(&SessionError::NoVault),
            Response::Locked
        );
        let leaky = SessionError::Io("/Users/x/secret-vault.kdbx: permission denied".into());
        let Response::Error { message } = session_error_response(&leaky) else {
            panic!("expected Error");
        };
        assert!(!message.contains("secret-vault"));
        let store = PairingError::Store("keychain said hunter2".into());
        let Response::Error { message } = pairing_error_response(&store) else {
            panic!("expected Error");
        };
        assert!(!message.contains("hunter2"));
    }
}
