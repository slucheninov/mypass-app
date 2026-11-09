//! KDBX 4 engine for MyPass (format, model, database API, vault file). Filled in by the later
//! tasks of stage 1.

pub mod error;
// Nothing consumes the format layer until the database API lands.
#[allow(dead_code)] // remove in Task 9
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
