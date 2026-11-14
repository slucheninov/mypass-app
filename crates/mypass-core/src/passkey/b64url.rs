//! Base64url without padding for passkey wire values and stored `KPEX_PASSKEY_*` fields.

use data_encoding::{Encoding, BASE64URL_NOPAD};
use std::sync::OnceLock;

/// Encode as URL-safe base64 with no padding.
pub fn encode(b: &[u8]) -> String {
    BASE64URL_NOPAD.encode(b)
}

fn lenient() -> Option<&'static Encoding> {
    static ENC: OnceLock<Option<Encoding>> = OnceLock::new();
    ENC.get_or_init(|| {
        let mut spec = BASE64URL_NOPAD.specification();
        // Other tools write non-canonical trailing bits; reading them must not fail.
        spec.check_trailing_bits = false;
        spec.encoding().ok()
    })
    .as_ref()
}

/// Decode base64url, padded or not. For interop with values written by KeePassXC and other tools
/// the standard alphabet (`+` and `/`) and non-zero trailing bits are tolerated too. Whitespace,
/// padding in the middle or a length that no byte string encodes gives `None`.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let trimmed = s.trim_end_matches('=');
    let pad = s.len() - trimmed.len();
    // Padding, when present, must complete a group of four.
    if pad > 2 || (pad > 0 && !s.len().is_multiple_of(4)) {
        return None;
    }
    let url: String = trimmed
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    lenient()?.decode(url.as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64url_round_trips_all_lengths_0_to_70() {
        for n in 0..=70usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode(&encode(&data)), Some(data), "len {n}");
        }
    }

    #[test]
    fn b64url_decode_accepts_padded_unpadded_and_standard_alphabet_rejects_whitespace() {
        assert_eq!(decode("-_8"), Some(vec![0xfb, 0xff]));
        assert_eq!(decode("-_8="), Some(vec![0xfb, 0xff]));
        assert_eq!(decode("+/8="), Some(vec![0xfb, 0xff]));
        assert_eq!(decode("+/8"), Some(vec![0xfb, 0xff]));
        assert_eq!(decode("YQ"), Some(b"a".to_vec()));
        assert_eq!(decode("YQ=="), Some(b"a".to_vec()));
        // Non-canonical trailing bits are tolerated.
        assert_eq!(decode("YR"), Some(b"a".to_vec()));
        assert_eq!(decode(""), Some(vec![]));
        for bad in [
            "Y Q", "YQ\n", " YQ", "Y=Q", "YQ=", "YQ===", "Y", "!!", "YQ==YQ",
        ] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn b64url_encode_never_pads() {
        for n in 0..=8usize {
            let s = encode(&vec![0xffu8; n]);
            assert!(!s.contains('=') && !s.contains('+') && !s.contains('/'));
        }
    }
}
