//! KDBX 4 engine for MyPass (format, model, database API, vault file). Filled in by the later
//! tasks of stage 1; this stub only proves that the crate builds and is linkable.

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
