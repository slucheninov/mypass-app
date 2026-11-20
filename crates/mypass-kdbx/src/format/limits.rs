//! Hostile-input limits, as named constants.
//!
//! A vault file is untrusted input: a crafted header can ask for terabytes of Argon2 memory, a
//! billion-iteration KDF or a decompression bomb. Every bound lives here so that the parsers,
//! the KDF code and the writer all agree on one definition. Parsers return `Err` when a value is
//! outside its range; they never clamp silently and never panic.

use super::kdf::KdfParams;
use crate::{KdbxError, Result};

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

/// Default automatic-open ceilings. Argon2 CPU work scales with memory times iterations.
const REMOTE_ARGON2_MEMORY_MAX_BYTES: u64 = 1024 * 1024 * 1024;
const REMOTE_ARGON2_WORK_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const REMOTE_AES_KDF_ROUNDS_MAX: u64 = 300_000_000;

/// Additional cost ceilings for an automatic vault open. The local format limits always apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfLimits(Option<KdfBudget>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KdfBudget {
    argon2_memory: u64,
    argon2_work: u64,
    aes_rounds: u64,
}

impl KdfLimits {
    /// Use the standard format limits for an explicit open.
    pub const LOCAL: Self = Self(None);

    /// Use the automatic-open ceilings, raised only by the corresponding cost of the
    /// already authenticated vault. Each KDF family has its own independent budget.
    pub fn remote(authenticated: Option<&KdfParams>) -> Self {
        let mut budget = KdfBudget {
            argon2_memory: REMOTE_ARGON2_MEMORY_MAX_BYTES,
            argon2_work: REMOTE_ARGON2_WORK_MAX_BYTES,
            aes_rounds: REMOTE_AES_KDF_ROUNDS_MAX,
        };
        match authenticated {
            Some(KdfParams::Argon2 {
                memory_bytes,
                iterations,
                ..
            }) => {
                budget.argon2_memory = budget.argon2_memory.max(*memory_bytes);
                budget.argon2_work = budget
                    .argon2_work
                    .max(memory_bytes.saturating_mul(*iterations));
            }
            Some(KdfParams::AesKdf { rounds, .. }) => {
                budget.aes_rounds = budget.aes_rounds.max(*rounds);
            }
            None => {}
        }
        Self(Some(budget))
    }

    pub(crate) fn check(self, kdf: &KdfParams) -> Result<()> {
        let Some(budget) = self.0 else {
            return Ok(());
        };
        let parameter = match kdf {
            KdfParams::Argon2 { memory_bytes, .. } if *memory_bytes > budget.argon2_memory => {
                "Argon2 memory"
            }
            KdfParams::Argon2 {
                memory_bytes,
                iterations,
                ..
            } if memory_bytes.saturating_mul(*iterations) > budget.argon2_work => "Argon2 work",
            KdfParams::AesKdf { rounds, .. } if *rounds > budget.aes_rounds => "AES-KDF rounds",
            _ => return Ok(()),
        };
        Err(KdbxError::RemoteKdfLimitExceeded(parameter))
    }
}

/// Largest accepted length of one HMAC-protected payload block, in bytes (2^30).
pub(crate) const MAX_BLOCK_LEN: usize = 1 << 30;
/// Largest accepted decompressed payload, in bytes (1 GiB); the guard against gzip bombs.
pub(crate) const MAX_DECOMPRESSED_LEN: u64 = 1 << 30;
/// Largest vault file read from storage that other software writes (a sync folder, iCloud Drive, a
/// migration target), in bytes: the decompressed cap plus 64 MiB for headers, block framing and
/// padding, so no file the parsers would accept is refused. The read itself is bounded
/// (`FileIo::read_limited`), so a huge, growing or endless file is cut off before it can exhaust
/// memory.
pub const MAX_VAULT_FILE_BYTES: u64 = MAX_DECOMPRESSED_LEN + 64 * 1024 * 1024;
/// Deepest accepted XML element nesting.
pub(crate) const MAX_XML_DEPTH: usize = 100;

/// Maximum serializable group chain depth, including the root group.
/// Deliberately stricter than opening shallow-content chains at depths 94..97:
/// future entries/history must remain serializable at every accepted group.
/// Reserves KeePassFile/Root and Entry/History/Entry/String/Value XML nesting.
pub const MAX_GROUP_DEPTH: usize = MAX_XML_DEPTH - 7;
