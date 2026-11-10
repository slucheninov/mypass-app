//! The single error type of the KDBX engine.
//!
//! Variants are deliberately coarse and carry no secret material: messages describe the
//! structural problem ("truncated header field"), never the bytes that were being parsed.

/// Everything that can go wrong while opening, editing or saving a vault.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KdbxError {
    /// The operating system could not supply cryptographic randomness.
    #[error("OS randomness unavailable")]
    Random,
    /// The input does not start with the KDBX magic (or is KDB1).
    #[error("not a KDBX file")]
    NotKdbx,
    /// A KDBX file whose major version this engine does not read (KDBX 2 and 3).
    #[error("unsupported KDBX version {major}.{minor}")]
    UnsupportedVersion { major: u16, minor: u16 },
    /// The composite key did not authenticate the header (HMAC mismatch).
    #[error("wrong password")]
    WrongPassword,
    /// The file is structurally damaged or violates a hostile-input limit.
    #[error("corrupt vault: {0}")]
    Corrupt(String),
    /// A valid KDBX feature that this engine deliberately does not implement.
    #[error("unsupported feature: {0}")]
    Unsupported(String),
    /// KDF parameters that are missing, malformed or outside the accepted limits.
    #[error("invalid KDF parameters: {0}")]
    InvalidKdfParameters(String),
    /// A file-system failure, flattened so the error stays `Clone + Eq`.
    #[error("i/o error ({kind:?}): {message}")]
    Io {
        kind: std::io::ErrorKind,
        message: String,
    },
    /// The caller asked for something the data model forbids.
    #[error("invalid operation: {0}")]
    InvalidOperation(String),
}

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, KdbxError>;
