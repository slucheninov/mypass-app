//! Everything that consumes the KDBX model: TOTP, password generator and rules, Public Suffix List
//! matching, entry matcher and merge engine. TOTP is available now; later stage-1 tasks add
//! the remaining consumers.

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

/// Time-based one-time passwords and secret parsing.
pub mod totp;
pub use totp::{base32_decode, base32_encode, totp_code, TotpAlgorithm, TotpConfig, TotpError};
