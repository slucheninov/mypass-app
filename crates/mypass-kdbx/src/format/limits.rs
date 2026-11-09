//! Hostile-input limits, as named constants.
//!
//! A vault file is untrusted input: a crafted header can ask for terabytes of Argon2 memory, a
//! billion-iteration KDF or a decompression bomb. Every bound lives here so that the parsers,
//! the KDF code and the writer all agree on one definition. Parsers return `Err` when a value is
//! outside its range; they never clamp silently and never panic.

/// Smallest accepted Argon2 memory cost, in bytes (8 KiB). KDBX stores the `M` parameter in bytes.
pub(crate) const ARGON2_MEMORY_MIN_BYTES: u64 = 8 * 1024;
/// Largest accepted Argon2 memory cost, in bytes (4 GiB).
pub(crate) const ARGON2_MEMORY_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Smallest accepted Argon2 iteration count (the `I` parameter).
pub(crate) const ARGON2_ITERATIONS_MIN: u64 = 1;
/// Largest accepted Argon2 iteration count.
pub(crate) const ARGON2_ITERATIONS_MAX: u64 = 10_000;
/// Smallest accepted Argon2 parallelism (the `P` parameter, lanes).
pub(crate) const ARGON2_PARALLELISM_MIN: u32 = 1;
/// Largest accepted Argon2 parallelism.
pub(crate) const ARGON2_PARALLELISM_MAX: u32 = 255;
/// Shortest accepted Argon2 salt, in bytes (the Argon2 specification's lower bound).
pub(crate) const ARGON2_SALT_MIN_LEN: usize = 8;

/// Smallest accepted AES-KDF round count.
pub(crate) const AES_KDF_ROUNDS_MIN: u64 = 1;
/// Largest accepted AES-KDF round count.
pub(crate) const AES_KDF_ROUNDS_MAX: u64 = 1_000_000_000;

/// Largest accepted length of one HMAC-protected payload block, in bytes (2^30).
pub(crate) const MAX_BLOCK_LEN: usize = 1 << 30;
/// Largest accepted decompressed payload, in bytes (1 GiB); the guard against gzip bombs.
pub(crate) const MAX_DECOMPRESSED_LEN: u64 = 1 << 30;
/// Deepest accepted XML element nesting.
pub(crate) const MAX_XML_DEPTH: usize = 100;

/// Maximum serializable group chain depth, including the root group.
/// Reserves KeePassFile/Root and Entry/History/Entry/String/Value XML nesting.
pub const MAX_GROUP_DEPTH: usize = MAX_XML_DEPTH - 7;
