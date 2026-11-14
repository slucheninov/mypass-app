//! Passkey building blocks: base64url, a minimal CBOR encoder, the authenticator AAGUID and the
//! KeePassXC-compatible field names with the one `has_passkey` predicate.

/// Unpadded base64url encoding and a lenient decoder.
pub mod b64url;
// Consumed by the ceremony module (next task); only the tests use it until then.
#[allow(dead_code)]
pub(crate) mod cbor;
/// Origin and relying-party-id validation; `ValidatedOrigin` gates every signature.
pub mod origin;
/// The `KPEX_PASSKEY_*` field names and the single `has_passkey` predicate.
pub mod record;

/// WebAuthn create and get request parsing with size and text caps.
pub mod request;

pub use origin::{validate_origin, ValidatedOrigin};
pub use record::has_passkey;
pub use request::{parse_create_request, parse_get_request, CreateContext, GetContext};

/// The AAGUID MyPass reports in the attested credential data of every passkey it creates
/// (`94803471-f8b4-48f9-98ff-fc4ac1a92af8`). It is a fixed random v4 UUID owned by MyPass, the
/// same value the earlier Swift app used, so relying parties see one authenticator model across
/// both apps.
pub const AAGUID: [u8; 16] = [
    0x94, 0x80, 0x34, 0x71, 0xf8, 0xb4, 0x48, 0xf9, 0x98, 0xff, 0xfc, 0x4a, 0xc1, 0xa9, 0x2a, 0xf8,
];

/// Why a passkey operation was refused. The messages are fixed text and never echo input.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PasskeyError {
    /// The request was malformed or missing a required value.
    #[error("invalid passkey request")]
    InvalidRequest,
    /// None of the requested public-key algorithms is supported.
    #[error("no supported passkey algorithm")]
    UnsupportedAlgorithm,
    /// The caller origin is not acceptable.
    #[error("passkey origin mismatch")]
    OriginMismatch,
    /// The relying party id is not valid for the caller origin.
    #[error("relying party id not authorized")]
    RpIdNotAuthorized,
    /// A cryptographic operation failed.
    #[error("passkey cryptographic failure")]
    Crypto,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aaguid_bytes_equal_the_swift_constant_and_uuid_string() {
        assert_eq!(
            uuid::Uuid::from_bytes(AAGUID).to_string(),
            "94803471-f8b4-48f9-98ff-fc4ac1a92af8"
        );
    }

    #[test]
    fn passkey_error_display_contains_no_input() {
        for e in [
            PasskeyError::InvalidRequest,
            PasskeyError::UnsupportedAlgorithm,
            PasskeyError::OriginMismatch,
            PasskeyError::RpIdNotAuthorized,
            PasskeyError::Crypto,
        ] {
            let text = e.to_string();
            assert!(!text.is_empty() && !text.contains('{') && !text.contains("://"));
        }
    }
}
