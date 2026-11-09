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
pub use format::limits::MAX_GROUP_DEPTH;
pub use format::outer_header::{Cipher, Compression};
pub use model::*;

mod ops;

mod file;
pub use file::{write_atomic, FileIo, FileMeta, FileStamp, PlainFileIo, SaveOutcome, VaultFile};
#[cfg(feature = "test-util")]
pub mod test_util;
