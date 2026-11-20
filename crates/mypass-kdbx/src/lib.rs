//! KDBX 4 engine for MyPass (format, model, database API, vault file). Filled in by the later
//! tasks of stage 1.

pub mod error;
mod format;

pub use error::{KdbxError, Result};

/// Name of this crate, used by the smoke test and by `mypass-core`'s link check.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_links() {
        assert_eq!(CRATE_NAME, "mypass-kdbx");
    }
}

mod database;
mod model;
pub use database::{Database, SaveSettings};
pub use format::kdf::{Argon2Variant, KdfParams};
pub use format::limits::{KdfCost, KdfLimits, MAX_GROUP_DEPTH, MAX_VAULT_FILE_BYTES};
pub use format::outer_header::{Cipher, Compression};
pub use model::*;

mod ops;

mod file;
pub use file::{
    authenticate_retained_copy, displaced_copies, inspect_retained_copies,
    recorded_retained_copies, remove_retained_copy, set_retained_copy_listener, write_atomic,
    AuthenticatedRetainedRemoval, DisplacedCopy, DisplacedCopyKind, FileIo, FileMeta, FileStamp,
    OpenedFile, PlainFileIo, ReplaceOutcome, RetainedCopyCheck, RetainedCopyListener,
    RetainedIdentity, RetainedInspection, RetainedProvenance, RetainedRefusal, RetainedRemoval,
    SaveOutcome, VaultFile, RETAINED_IN_FLIGHT_GRACE,
};
#[cfg(feature = "test-util")]
pub mod test_util;
