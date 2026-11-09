//! Stateful KDBX inner random streams.
use chacha20::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha256, Sha512};
use zeroize::{Zeroize, Zeroizing};

use super::inner_header::InnerStreamKind;
use crate::error::{KdbxError, Result};

// Deliberately no Debug implementation: cipher state contains secret material.
pub struct InnerStream {
    cipher: CipherState,
    failed: bool,
}

enum CipherState {
    ChaCha20(chacha20::ChaCha20),
    Salsa20(salsa20::Salsa20),
}

impl InnerStream {
    pub fn new(kind: InnerStreamKind, key: &[u8]) -> Result<Self> {
        if key.is_empty() {
            return Err(KdbxError::Corrupt("empty inner stream key".into()));
        }
        let cipher = match kind {
            InnerStreamKind::ChaCha20 => {
                let hashed: Zeroizing<[u8; 64]> = Zeroizing::new(Sha512::digest(key).into());
                // IETF ChaCha20: 32-byte key, 12-byte nonce, counter zero.
                let derived_key = hashed.get(..32).ok_or_else(stream_error)?;
                let nonce = hashed.get(32..44).ok_or_else(stream_error)?;
                CipherState::ChaCha20(
                    chacha20::ChaCha20::new_from_slices(derived_key, nonce)
                        .map_err(|_| stream_error())?,
                )
            }
            InnerStreamKind::Salsa20 => {
                let hashed: Zeroizing<[u8; 32]> = Zeroizing::new(Sha256::digest(key).into());
                let nonce = [0xe8, 0x30, 0x09, 0x4b, 0x97, 0x20, 0x5d, 0x2a];
                CipherState::Salsa20(
                    salsa20::Salsa20::new_from_slices(&*hashed, &nonce)
                        .map_err(|_| stream_error())?,
                )
            }
        };
        Ok(Self {
            cipher,
            failed: false,
        })
    }

    /// XOR with the next keystream bytes, preserving state across calls.
    ///
    /// The infallible interface fails closed at counter exhaustion: this buffer
    /// and every subsequent buffer are zeroed. ChaCha20 exhaustion requires about
    /// 2^38 bytes, far beyond the 1 GiB payload limit; Salsa20's limit is larger.
    /// Never use a panicking API or leave plaintext unchanged on failure.
    pub(crate) fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn apply(&mut self, data: &mut [u8]) {
        if !self.failed {
            let result = match &mut self.cipher {
                CipherState::ChaCha20(cipher) => cipher.try_apply_keystream(data),
                CipherState::Salsa20(cipher) => cipher.try_apply_keystream(data),
            };
            self.failed = result.is_err();
        }
        if self.failed {
            data.zeroize();
        }
    }
}

fn stream_error() -> KdbxError {
    KdbxError::Corrupt("inner stream initialization".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decode_hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    fn check_split(kind: InnerStreamKind, key: &[u8], len: usize, split: usize) {
        let mut full = vec![0; len];
        let mut parts = full.clone();
        InnerStream::new(kind, key).unwrap().apply(&mut full);
        let mut stream = InnerStream::new(kind, key).unwrap();
        stream.apply(&mut parts[..split]);
        stream.apply(&mut []);
        stream.apply(&mut parts[split..]);
        assert_eq!(full, parts);
    }
    #[test]
    fn chacha20_stream_deterministic_and_sequential() {
        check_split(InnerStreamKind::ChaCha20, &[0x55; 64], 64, 32);
        check_split(InnerStreamKind::ChaCha20, &[0x55; 64], 137, 7);
    }
    #[test]
    fn salsa20_stream_deterministic_and_sequential() {
        check_split(InnerStreamKind::Salsa20, &[0x66; 32], 80, 10);
    }
    #[test]
    fn salsa20_nontrivial_output() {
        let mut bytes = [0; 32];
        InnerStream::new(InnerStreamKind::Salsa20, &[0x66; 32])
            .unwrap()
            .apply(&mut bytes);
        assert_ne!(bytes, [0; 32]);
    }
    #[test]
    fn xor_round_trip() {
        for kind in [InnerStreamKind::ChaCha20, InnerStreamKind::Salsa20] {
            let plain = b"secret-password-value";
            let mut bytes = plain.to_vec();
            InnerStream::new(kind, &[0x77; 64])
                .unwrap()
                .apply(&mut bytes);
            assert_ne!(bytes, plain);
            InnerStream::new(kind, &[0x77; 64])
                .unwrap()
                .apply(&mut bytes);
            assert_eq!(bytes, plain);
        }
    }
    // Literal oracle: libsodium crypto_stream_chacha20_ietf, counter zero,
    // SHA512(bytes 00..3f), key bytes 0..32 and nonce bytes 32..44.
    // The supplied Swift test computes its expectation; it contains no literal vector.
    #[test]
    fn chacha20_matches_independently_computed_keystream() {
        let key: Vec<u8> = (0..64).collect();
        let mut bytes = [0; 64];
        InnerStream::new(InnerStreamKind::ChaCha20, &key)
            .unwrap()
            .apply(&mut bytes);
        assert_eq!(
            bytes.as_slice(),
            decode_hex(concat!(
                "8ce8bc610ac05ff2e3dd88b49a1404c2844f148037027476b83d58f5609adf65",
                "f8fa87d8b5f5287f6cfdbdb69adea2f942e4acf5a9ad9e860623477a73a24c4e"
            ))
        );
    }
    // Literal oracle: libsodium crypto_stream_salsa20 with SHA256(bytes 00..3f)
    // = fdeab9acf3710362bd2658cdc9a29e8f9c757fcf9811603a8c447cd1d9151108
    // and KeePass nonce e830094b97205d2a, counter zero.
    #[test]
    fn salsa20_matches_independently_computed_keystream() {
        let key: Vec<u8> = (0..64).collect();
        let mut bytes = [0; 64];
        InnerStream::new(InnerStreamKind::Salsa20, &key)
            .unwrap()
            .apply(&mut bytes);
        assert_eq!(
            bytes.as_slice(),
            decode_hex(concat!(
                "0f49b2bd1392f303c02eef124cd0fda0d1855645eb16470cb5aadc7698fb7bede",
                "27ee7b99216b0be573bb128a66cdcb16e916294edda057d95ce8a3623c45367"
            ))
        );
    }
    #[test]
    fn short_key_for_salsa_is_accepted() {
        let mut bytes = [0; 16];
        InnerStream::new(InnerStreamKind::Salsa20, &[1])
            .unwrap()
            .apply(&mut bytes);
        assert_ne!(bytes, [0; 16]);
    }
    #[test]
    fn empty_key_is_corrupt_for_both_kinds() {
        for kind in [InnerStreamKind::ChaCha20, InnerStreamKind::Salsa20] {
            assert!(matches!(
                InnerStream::new(kind, &[]),
                Err(KdbxError::Corrupt(_))
            ));
        }
    }
    #[test]
    fn exhausted_stream_zeroes_data_and_stays_failed_closed() {
        use chacha20::cipher::StreamCipherSeek;
        for kind in [InnerStreamKind::ChaCha20, InnerStreamKind::Salsa20] {
            let mut stream = InnerStream::new(kind, b"counter-exhaustion-test").unwrap();
            match &mut stream.cipher {
                CipherState::ChaCha20(cipher) => {
                    cipher.try_seek(u64::from(u32::MAX - 1) * 64).unwrap()
                }
                CipherState::Salsa20(cipher) => {
                    cipher.try_seek(u128::from(u64::MAX - 1) * 64).unwrap()
                }
            }
            let mut bytes = [0x41; 65];
            stream.apply(&mut bytes);
            assert_eq!(bytes, [0; 65]);
            // Even if a smaller call could otherwise fit, failure is latched.
            let mut later = [0x42; 1];
            stream.apply(&mut later);
            assert_eq!(later, [0; 1]);
            assert!(stream.failed);
        }
    }
}
