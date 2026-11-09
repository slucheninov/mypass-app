//! RFC 6238 TOTP generation and otpauth/base32 parsing.

use hmac::{EagerHash, Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

/// Hash algorithm used for the TOTP HMAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotpAlgorithm {
    /// HMAC-SHA1 (the otpauth default).
    Sha1,
    /// HMAC-SHA256.
    Sha256,
    /// HMAC-SHA512.
    Sha512,
}

/// Invalid generation parameters; messages never contain secret data.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TotpError {
    /// The period must be nonzero.
    #[error("TOTP period must be nonzero")]
    InvalidPeriod,
    /// The digit count must be between one and ten.
    #[error("TOTP digits must be between 1 and 10")]
    InvalidDigits,
    /// The cryptographic backend rejected the key or digest shape.
    #[error("TOTP HMAC computation failed")]
    InvalidHmac,
}

/// Parsed TOTP parameters with a zeroizing secret and redacted debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct TotpConfig {
    /// Decoded secret bytes, erased on drop.
    pub secret: Zeroizing<Vec<u8>>,
    /// Output width (parsing accepts 6 through 10).
    pub digits: u32,
    /// Time step in seconds (parsing accepts 1 through 3600).
    pub period: u32,
    /// HMAC hash algorithm.
    pub algorithm: TotpAlgorithm,
}

impl std::fmt::Debug for TotpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TotpConfig")
            .field("secret_len", &self.secret.len())
            .field("digits", &self.digits)
            .field("period", &self.period)
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

/// Generate a zero-padded RFC 6238 code at an injected Unix timestamp.
///
/// Rejects zero periods and digits outside 1..=10. Unlike parsing, this lower-level
/// API permits fewer than six digits and periods greater than an hour.
pub fn totp_code(
    secret: &[u8],
    unix_secs: u64,
    period: u32,
    digits: u32,
    algorithm: TotpAlgorithm,
) -> Result<String, TotpError> {
    if period == 0 {
        return Err(TotpError::InvalidPeriod);
    }
    if !(1..=10).contains(&digits) {
        return Err(TotpError::InvalidDigits);
    }
    let counter = (unix_secs / u64::from(period)).to_be_bytes();
    let digest = match algorithm {
        TotpAlgorithm::Sha1 => hmac_digest::<Sha1>(secret, &counter)?,
        TotpAlgorithm::Sha256 => hmac_digest::<Sha256>(secret, &counter)?,
        TotpAlgorithm::Sha512 => hmac_digest::<Sha512>(secret, &counter)?,
    };
    let offset = usize::from(digest.last().ok_or(TotpError::InvalidHmac)? & 0x0f);
    let bytes: [u8; 4] = digest
        .get(offset..offset + 4)
        .ok_or(TotpError::InvalidHmac)?
        .try_into()
        .map_err(|_| TotpError::InvalidHmac)?;
    let binary = u32::from_be_bytes(bytes) & 0x7fff_ffff;
    let code = u64::from(binary) % 10_u64.pow(digits);
    Ok(format!("{code:0width$}", width = digits as usize))
}

fn hmac_digest<D: EagerHash>(
    secret: &[u8],
    counter: &[u8],
) -> Result<Zeroizing<Vec<u8>>, TotpError> {
    let mut mac = Hmac::<D>::new_from_slice(secret).map_err(|_| TotpError::InvalidHmac)?;
    mac.update(counter);
    // The returned digest buffer is wiped; RustCrypto's internal hasher state is
    // subject to the dependency's own cleanup guarantees.
    let mut digest = mac.finalize().into_bytes();
    let result = Zeroizing::new(digest.to_vec());
    use zeroize::Zeroize;
    digest.as_mut_slice().zeroize();
    Ok(result)
}

/// Decode base32, ignoring ASCII spaces, hyphens, padding and letter case.
///
/// Like Swift, unused trailing bits (including an incomplete final symbol group)
/// are discarded rather than checked for canonical zero bits. Invalid alphabet
/// characters and empty decoded results are rejected. The caller owns and must
/// protect the returned bytes; temporary normalized and decoded buffers are wiped.
pub fn base32_decode(input: &str) -> Option<Vec<u8>> {
    let mut normalized = Zeroizing::new(
        input
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '='))
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>(),
    );
    let output_len = normalized.len().checked_mul(5)? / 8;
    if output_len == 0 {
        return None;
    }
    // Complete a block with zero symbols, then discard bytes from incomplete bits.
    // This lets the dependency validate the alphabet while matching Swift's
    // permissive length/trailing-bit behavior, without a hand-written codec.
    while normalized.len() % 8 != 0 {
        normalized.push('A');
    }
    let encoding = data_encoding::BASE32_NOPAD;
    let mut decoded = Zeroizing::new(vec![0; encoding.decode_len(normalized.len()).ok()?]);
    encoding
        .decode_mut(normalized.as_bytes(), &mut decoded)
        .ok()?;
    decoded.truncate(output_len);
    Some(std::mem::take(&mut *decoded))
}

/// Encode bytes as padded uppercase RFC 4648 base32 (empty bytes yield an empty string).
pub fn base32_encode(data: &[u8]) -> String {
    data_encoding::BASE32.encode(data)
}

impl TotpConfig {
    /// Parse a raw base32 secret or an `otpauth://totp/` URL, defaulting to 6/30/SHA1.
    ///
    /// The first occurrence of each query parameter wins, including `secret`.
    /// Algorithms are case-insensitive. Unknown algorithms are rejected instead
    /// of Swift's SHA1 fallback, because silently selecting another hash gives
    /// incorrect codes. Parsing deliberately bounds digits to 6..=10 and period
    /// to 1..=3600. Query names are case-sensitive; form decoding treats `+`
    /// as a space (ignored in secrets). Fractional periods and empty algorithm
    /// values are rejected rather than rounded/defaulted as in Swift;
    /// public fields are validated again during generation.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        let mut config = Self {
            secret: Zeroizing::new(Vec::new()),
            digits: 6,
            period: 30,
            algorithm: TotpAlgorithm::Sha1,
        };
        if !input.contains("://") {
            config.secret = Zeroizing::new(base32_decode(input)?);
            return Some(config);
        }
        // Parse the URL structure without copying its secret query into Url's
        // non-zeroizing storage. Labels are opaque and may contain spaces.
        let (head, query) = input.split_once('?')?;
        let url = url::Url::parse(head).ok()?;
        if url.scheme() != "otpauth" || !url.host_str()?.eq_ignore_ascii_case("totp") {
            return None;
        }
        let mut secret = None;
        let mut digits = None;
        let mut period = None;
        let mut algorithm = None;
        let query = query.split('#').next()?;
        for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
            let value = Zeroizing::new(value.into_owned());
            let slot = match name.as_ref() {
                "secret" => &mut secret,
                "digits" => &mut digits,
                "period" => &mut period,
                "algorithm" => &mut algorithm,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(value);
            }
        }
        config.secret = Zeroizing::new(base32_decode(secret.as_deref()?)?);
        if let Some(value) = digits {
            config.digits = value.parse().ok()?;
        }
        if let Some(value) = period {
            config.period = value.parse().ok()?;
        }
        if !(6..=10).contains(&config.digits) || !(1..=3600).contains(&config.period) {
            return None;
        }
        if let Some(value) = algorithm {
            config.algorithm = if value.eq_ignore_ascii_case("SHA1") {
                TotpAlgorithm::Sha1
            } else if value.eq_ignore_ascii_case("SHA256") {
                TotpAlgorithm::Sha256
            } else if value.eq_ignore_ascii_case("SHA512") {
                TotpAlgorithm::Sha512
            } else {
                return None;
            };
        }
        Some(config)
    }

    /// Generate using this config; invalid publicly modified fields return an error.
    pub fn code_at(&self, unix_secs: u64) -> Result<String, TotpError> {
        totp_code(
            &self.secret,
            unix_secs,
            self.period,
            self.digits,
            self.algorithm,
        )
    }

    /// Seconds until the next time step, in 1..=period; zero for a zero period.
    pub fn seconds_remaining(&self, unix_secs: u64) -> u32 {
        if self.period == 0 {
            return 0;
        }
        self.period - (unix_secs % u64::from(self.period)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SHA1_KEY: &[u8] = b"12345678901234567890";
    const SHA256_KEY: &[u8] = b"12345678901234567890123456789012";
    const SHA512_KEY: &[u8] = b"1234567890123456789012345678901234567890123456789012345678901234";

    #[test]
    fn long_keys_and_maximum_timestamp() {
        // Independently checked with Python stdlib hmac/hashlib; key = 200 bytes of 0xab.
        let key = vec![0xab; 200];
        for (algorithm, expected) in [
            (TotpAlgorithm::Sha1, "50878979"),
            (TotpAlgorithm::Sha256, "58667971"),
            (TotpAlgorithm::Sha512, "70197661"),
        ] {
            assert_eq!(
                totp_code(&key, 1234567890, 30, 8, algorithm).unwrap(),
                expected
            );
        }
        let config = TotpConfig::parse("MY").unwrap();
        assert_eq!(config.code_at(u64::MAX).unwrap().len(), 6);
    }
    #[test]
    fn query_parameter_policies_and_pretty_debug() {
        let config = TotpConfig::parse("otpauth://totp/x?secret=MY&digits=8&digits=9&period=60&period=90&algorithm=SHA256&algorithm=SHA512&DIGITS=10").unwrap();
        assert_eq!(
            (config.digits, config.period, config.algorithm),
            (8, 60, TotpAlgorithm::Sha256)
        );
        assert!(TotpConfig::parse("otpauth://totp/x?SECRET=MY").is_none());
        for query in ["secret=MY&period=1.5", "secret=MY&algorithm="] {
            assert!(TotpConfig::parse(&format!("otpauth://totp/x?{query}")).is_none());
        }
        assert_eq!(
            TotpConfig::parse("otpauth://totp/x?secret=M+Y")
                .unwrap()
                .secret
                .as_slice(),
            b"f"
        );
        let debug = format!("{config:#?}");
        for expected in ["secret_len: 1", "digits: 8", "period: 60", "Sha256"] {
            assert!(debug.contains(expected));
        }
        assert!(!debug.contains("[102]"));
    }
    #[test]
    fn cleanup_uppercase_totp_host() {
        assert_eq!(
            TotpConfig::parse("otpauth://TOTP/test?secret=MY"),
            TotpConfig::parse("otpauth://totp/test?secret=MY")
        );
    }
    #[test]
    fn rfc6238_vectors() {
        for (key, algorithm, time, expected) in [
            (SHA1_KEY, TotpAlgorithm::Sha1, 59, "94287082"),
            (SHA1_KEY, TotpAlgorithm::Sha1, 1111111109, "07081804"),
            (SHA1_KEY, TotpAlgorithm::Sha1, 1111111111, "14050471"),
            (SHA1_KEY, TotpAlgorithm::Sha1, 1234567890, "89005924"),
            (SHA1_KEY, TotpAlgorithm::Sha1, 2000000000, "69279037"),
            (SHA256_KEY, TotpAlgorithm::Sha256, 59, "46119246"),
            (SHA256_KEY, TotpAlgorithm::Sha256, 1111111109, "68084774"),
            (SHA512_KEY, TotpAlgorithm::Sha512, 59, "90693936"),
            (SHA512_KEY, TotpAlgorithm::Sha512, 1111111109, "25091201"),
        ] {
            assert_eq!(totp_code(key, time, 30, 8, algorithm).unwrap(), expected);
        }
        // Independent Python stdlib HMAC/SHA256 oracle with the 20-byte key.
        assert_eq!(
            totp_code(SHA1_KEY, 59, 30, 8, TotpAlgorithm::Sha256).unwrap(),
            "32247374"
        );
    }

    #[test]
    fn default_six_digit_truncation() {
        assert_eq!(
            totp_code(SHA1_KEY, 59, 30, 6, TotpAlgorithm::Sha1).unwrap(),
            "287082"
        );
    }
    #[test]
    fn leading_zeros_preserved() {
        assert_eq!(
            totp_code(SHA1_KEY, 1111111109, 30, 6, TotpAlgorithm::Sha1).unwrap(),
            "081804"
        );
    }
    #[test]
    fn generation_validates_period_and_digits() {
        assert_eq!(
            totp_code(SHA1_KEY, 59, 0, 6, TotpAlgorithm::Sha1),
            Err(TotpError::InvalidPeriod)
        );
        for digits in [0, 11, u32::MAX] {
            assert_eq!(
                totp_code(SHA1_KEY, 59, 30, digits, TotpAlgorithm::Sha1),
                Err(TotpError::InvalidDigits)
            );
        }
        assert_eq!(
            totp_code(SHA1_KEY, 59, 30, 1, TotpAlgorithm::Sha1).unwrap(),
            "2"
        );
        assert_eq!(
            totp_code(SHA1_KEY, 59, 30, 10, TotpAlgorithm::Sha1).unwrap(),
            "1094287082"
        );
    }
    #[test]
    fn base32_round_trip_and_known() {
        for (encoded, plain) in [
            ("MY======", b"f".as_slice()),
            ("MZXQ====", b"fo"),
            ("MZXW6===", b"foo"),
            ("MZXW6YQ=", b"foob"),
            ("MZXW6YTB", b"fooba"),
            ("MZXW6YTBOI======", b"foobar"),
        ] {
            assert_eq!(base32_decode(encoded).unwrap(), plain);
            assert_eq!(base32_encode(plain), encoded);
        }
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_decode(""), None);
    }
    #[test]
    fn base32_tolerates_lowercase_spaces_hyphens_and_trailing_bits() {
        assert_eq!(base32_decode(" m-z xw6=== ").unwrap(), b"foo");
        assert_eq!(base32_decode("MZ").unwrap(), b"f");
        // Swift discards incomplete final bits, even an extra single symbol.
        assert_eq!(base32_decode("MYA").unwrap(), b"f");
    }
    #[test]
    fn base32_invalid_character_is_none() {
        for input in ["!", "M0", "M1", "M8", "M9", "A", " -= ", "\t\n"] {
            assert_eq!(base32_decode(input), None);
        }
    }
    #[test]
    fn parse_otpauth_full() {
        let config=TotpConfig::parse("otpauth://totp/ACME:alice@example.com?secret=JBSWY3DPEHPK3PXP&issuer=ACME&digits=8&period=60&algorithm=SHA256").unwrap();
        assert_eq!(&*config.secret, b"Hello!\xde\xad\xbe\xef");
        assert_eq!(
            (config.digits, config.period, config.algorithm),
            (8, 60, TotpAlgorithm::Sha256)
        );
    }
    #[test]
    fn parse_otpauth_defaults_and_raw_secret() {
        let config = TotpConfig::parse("otpauth://totp/Label?secret=JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(
            (config.digits, config.period, config.algorithm),
            (6, 30, TotpAlgorithm::Sha1)
        );
        assert_eq!(TotpConfig::parse("jbsw y3dp ehpk 3pxp"), Some(config));
    }
    #[test]
    fn parse_percent_encoded_secret_label_spaces_and_case_insensitive_algorithms() {
        for (name, algorithm) in [
            ("sHa1", TotpAlgorithm::Sha1),
            ("sha256", TotpAlgorithm::Sha256),
            ("sHa512", TotpAlgorithm::Sha512),
        ] {
            let config=TotpConfig::parse(&format!("otpauth://totp/Label:with spaces<>?secret=%6d%7a%78%77%36%3d%3d%3d&algorithm={name}")).unwrap();
            assert_eq!(&*config.secret, b"foo");
            assert_eq!(config.algorithm, algorithm);
        }
    }
    #[test]
    fn parse_first_secret_parameter_wins() {
        let config = TotpConfig::parse("otpauth://totp/x?secret=MZXW6===&secret=MY======").unwrap();
        assert_eq!(&*config.secret, b"foo");
        assert!(TotpConfig::parse("otpauth://totp/x?secret=&secret=MY======").is_none());
    }
    #[test]
    fn parse_rejects_invalid_config_and_unknown_algorithm() {
        for suffix in [
            "",
            "secret=",
            "secret=!!!",
            "secret=MY&digits=0",
            "secret=MY&digits=5",
            "secret=MY&digits=11",
            "secret=MY&digits=no",
            "secret=MY&period=0",
            "secret=MY&period=3601",
            "secret=MY&period=-1",
            "secret=MY&period=no",
            "secret=MY&algorithm=SHA-1",
            "secret=MY&algorithm=unknown",
        ] {
            assert!(
                TotpConfig::parse(&format!("otpauth://totp/x?{suffix}")).is_none(),
                "{suffix}"
            );
        }
        for input in [
            "otpauth://hotp/x?secret=MY",
            "https://totp/x?secret=MY",
            "",
            "   ",
        ] {
            assert!(TotpConfig::parse(input).is_none());
        }
        for suffix in ["digits=6&period=1", "digits=10&period=3600"] {
            assert!(TotpConfig::parse(&format!("otpauth://totp/x?secret=MY&{suffix}")).is_some());
        }
    }
    #[test]
    fn config_code_uses_all_parameters_and_validates_public_fields() {
        let mut config=TotpConfig::parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA====&digits=8&period=60&algorithm=SHA256").unwrap();
        assert_eq!(config.code_at(119).unwrap(), "46119246");
        config.digits = 0;
        assert_eq!(config.code_at(119), Err(TotpError::InvalidDigits));
        config.digits = 6;
        config.period = 0;
        assert_eq!(config.code_at(119), Err(TotpError::InvalidPeriod));
        assert_eq!(config.seconds_remaining(119), 0);
    }
    #[test]
    fn seconds_remaining_boundaries() {
        let config = TotpConfig::parse("MY").unwrap();
        for (t, want) in [
            (0, 30),
            (29, 1),
            (30, 30),
            (45, 15),
            (59, 1),
            (u64::MAX, 15),
        ] {
            assert_eq!(config.seconds_remaining(t), want);
        }
    }
    #[test]
    fn debug_does_not_leak_secret() {
        let config = TotpConfig::parse("MZXW6YTBOI======").unwrap();
        let debug = format!("{config:?}");
        assert!(debug.contains("secret_len: 6"));
        for forbidden in ["foobar", "MZXW6YTBOI", "102, 111"] {
            assert!(!debug.contains(forbidden));
        }
        assert_eq!(config.clone(), config);
    }
}
