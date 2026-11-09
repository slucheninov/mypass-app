//! Key-independent corruption detection and password-dependent header authentication.

use crate::error::{KdbxError, Result};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256, Sha512};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub(crate) fn header_sha256(raw_header: &[u8]) -> [u8; 32] {
    Sha256::digest(raw_header).into()
}

pub(crate) fn header_hmac(raw_header: &[u8], hmac_key_base: &[u8; 64]) -> [u8; 32] {
    let mut hash = Sha512::new();
    Digest::update(&mut hash, u64::MAX.to_le_bytes());
    Digest::update(&mut hash, hmac_key_base);
    let key = Zeroizing::new(<[u8; 64]>::from(hash.finalize()));
    // SHA-512 produces exactly the SHA-256 HMAC block/key size, so initialization is infallible.
    let mut mac = Hmac::<Sha256>::new((&*key).into());
    Mac::update(&mut mac, raw_header);
    mac.finalize().into_bytes().into()
}

pub(crate) fn verify_sha256(raw_header: &[u8], stored: &[u8]) -> Result<()> {
    if bool::from(header_sha256(raw_header).as_slice().ct_eq(stored)) {
        Ok(())
    } else {
        Err(KdbxError::Corrupt("header checksum".into()))
    }
}

pub(crate) fn verify_hmac(
    raw_header: &[u8],
    hmac_key_base: &[u8; 64],
    stored: &[u8],
) -> Result<()> {
    if bool::from(
        header_hmac(raw_header, hmac_key_base)
            .as_slice()
            .ct_eq(stored),
    ) {
        Ok(())
    } else {
        Err(KdbxError::WrongPassword)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::KdbxError,
        format::signature::{write_signature, Version},
    };

    #[test]
    fn sha256_matches_manual_computation() {
        assert_eq!(
            header_sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
        assert_eq!(verify_sha256(b"abc", &header_sha256(b"abc")), Ok(()));
    }
    #[test]
    fn hmac_is_key_dependent_and_deterministic() {
        let a = header_hmac(b"header", &[1; 64]);
        assert_eq!(a, header_hmac(b"header", &[1; 64]));
        assert_ne!(a, header_hmac(b"header", &[2; 64]));
        assert_ne!(a, header_hmac(b"changed", &[1; 64]));
        assert_eq!(verify_hmac(b"header", &[1; 64], &a), Ok(()));
    }
    #[test]
    fn wrong_password_is_distinct_from_corruption() {
        let mut header = Vec::new();
        write_signature(&mut header, Version { major: 4, minor: 1 });
        header.extend_from_slice(&[0, 4, 0, 0, 0, 13, 10, 13, 10]);
        let hmac = header_hmac(&header, &[1; 64]);
        assert_eq!(
            verify_hmac(&header, &[2; 64], &hmac),
            Err(KdbxError::WrongPassword)
        );
        let sha = header_sha256(&header);
        header[0] ^= 1;
        assert_eq!(
            verify_sha256(&header, &sha),
            Err(KdbxError::Corrupt("header checksum".into()))
        );
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::error::KdbxError;
    #[test]
    fn hmac_matches_independent_python_hashlib_vector() {
        // Python stdlib: HMAC-SHA256(SHA512(u64::MAX LE || [1;64]), b"header").
        assert_eq!(
            header_hmac(b"header", &[1; 64]),
            [
                0x24, 0xf5, 0xed, 0x4e, 0x02, 0xf7, 0x66, 0x40, 0x07, 0x6a, 0xfb, 0xc6, 0x54, 0xed,
                0xbf, 0xcb, 0xee, 0x26, 0xc1, 0xb2, 0x16, 0x75, 0xa4, 0x04, 0x0c, 0x7c, 0x94, 0xaf,
                0x11, 0x2c, 0xac, 0x48
            ]
        );
    }
    #[test]
    fn wrong_digest_lengths_and_each_changed_byte_are_rejected() {
        let sha = header_sha256(b"header");
        let hmac = header_hmac(b"header", &[1; 64]);
        for len in [0, 1, 31, 33, 64] {
            assert_eq!(
                verify_sha256(b"header", &vec![0; len]),
                Err(KdbxError::Corrupt("header checksum".into()))
            );
            assert_eq!(
                verify_hmac(b"header", &[1; 64], &vec![0; len]),
                Err(KdbxError::WrongPassword)
            );
        }
        for i in 0..32 {
            let mut bad_sha = sha;
            bad_sha[i] ^= 1;
            assert!(matches!(
                verify_sha256(b"header", &bad_sha),
                Err(KdbxError::Corrupt(_))
            ));
            let mut bad_hmac = hmac;
            bad_hmac[i] ^= 1;
            assert_eq!(
                verify_hmac(b"header", &[1; 64], &bad_hmac),
                Err(KdbxError::WrongPassword)
            );
        }
    }
}
