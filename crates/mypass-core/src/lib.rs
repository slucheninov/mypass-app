//! Everything that consumes the KDBX model: TOTP, password generator and rules, Public Suffix List
//! matching, entry matcher and merge engine.

/// Name of this crate, used by the smoke test.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Time-based one-time passwords and secret parsing.
pub mod totp;
pub use totp::{base32_decode, base32_encode, totp_code, TotpAlgorithm, TotpConfig, TotpError};

/// Apple's password-rules grammar and structured rules.
pub mod password_rules;
/// Host lookup over the bundled Apple password-rules catalog.
pub mod rules_catalog;
pub use password_rules::{
    parse_password_rules, CharacterClass, PasswordRules, PasswordRulesParseError,
};
pub use rules_catalog::{bundled_rules_for_host, PasswordRulesIndex};

/// Cryptographic password generation.
pub mod generator;
/// Rule-derived toggle state.
pub mod generator_toggles;
pub use generator::{generate, generate_os, GeneratorError, GeneratorOptions};

/// Public suffix matching and canonical ACE hosts.
pub mod psl;

/// Vault entry URL and matching-mode accessors.
pub mod entry_ext;
/// URL-based credential selection.
pub mod matcher;

/// Pure UUID-keyed two-way vault merge.
pub mod merge;

#[cfg(test)]
mod tests {
    use super::CRATE_NAME;

    #[test]
    fn crate_links() {
        assert_eq!(CRATE_NAME, "mypass-core");
        assert_eq!(mypass_kdbx::CRATE_NAME, "mypass-kdbx");
    }
}
