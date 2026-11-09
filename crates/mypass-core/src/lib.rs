//! Everything that consumes the KDBX model: TOTP, password generator and rules, Public Suffix List
//! matching, entry matcher and merge engine. Filled in by the later tasks of stage 1; this stub only
//! proves that the crate builds and links against `mypass-kdbx`.

/// Name of this crate, used by the smoke test.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_links() {
        assert_eq!(CRATE_NAME, "mypass-core");
        assert_eq!(mypass_kdbx::CRATE_NAME, "mypass-kdbx");
    }
}
