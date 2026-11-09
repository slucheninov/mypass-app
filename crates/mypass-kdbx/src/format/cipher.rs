//! KDBX outer ciphers: AES-256-CBC with PKCS7 and IETF ChaCha20 at counter zero.
use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use chacha20::cipher::StreamCipher;
use zeroize::Zeroizing;

use super::outer_header::Cipher;
use crate::error::{KdbxError, Result};

fn outer_cipher_error() -> KdbxError {
    KdbxError::Corrupt("outer cipher".into())
}

#[allow(dead_code)] // remove in Task 9
pub(crate) fn encrypt(cipher: Cipher, key: &[u8; 32], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if iv.len() != cipher.iv_len() {
        return Err(outer_cipher_error());
    }
    match cipher {
        Cipher::Aes256Cbc => {
            let padded_len = data
                .len()
                .checked_add(16 - data.len() % 16)
                .ok_or_else(outer_cipher_error)?;
            // Wipe the intermediate plaintext and padding even on an error path.
            let mut buffer = Zeroizing::new(Vec::with_capacity(padded_len));
            buffer.extend_from_slice(data);
            buffer.resize(padded_len, 0);
            let encrypted = cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv)
                .map_err(|_| outer_cipher_error())?
                .encrypt_padded::<Pkcs7>(&mut buffer, data.len())
                .map_err(|_| outer_cipher_error())?;
            Ok(encrypted.to_vec())
        }
        Cipher::ChaCha20 => apply_chacha20(key, iv, data),
    }
}

pub(crate) fn decrypt(cipher: Cipher, key: &[u8; 32], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if iv.len() != cipher.iv_len() {
        return Err(outer_cipher_error());
    }
    match cipher {
        Cipher::Aes256Cbc => {
            if data.is_empty() || !data.len().is_multiple_of(16) {
                return Err(outer_cipher_error());
            }
            // Decrypt in a buffer bounded by ciphertext length. RustCrypto validates
            // every PKCS7 padding byte; wipe padded plaintext on success and failure.
            let mut buffer = Zeroizing::new(data.to_vec());
            let plaintext = cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
                .map_err(|_| outer_cipher_error())?
                .decrypt_padded::<Pkcs7>(&mut buffer)
                .map_err(|_| outer_cipher_error())?;
            Ok(plaintext.to_vec())
        }
        Cipher::ChaCha20 => apply_chacha20(key, iv, data),
    }
}

fn apply_chacha20(key: &[u8; 32], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    // `ChaCha20` is the IETF 12-byte-nonce variant, initialized at counter 0.
    // try_apply_keystream reports counter exhaustion instead of panicking.
    let mut cipher =
        chacha20::ChaCha20::new_from_slices(key, iv).map_err(|_| outer_cipher_error())?;
    let mut buffer = Zeroizing::new(data.to_vec());
    cipher
        .try_apply_keystream(&mut buffer)
        .map_err(|_| outer_cipher_error())?;
    Ok(std::mem::take(&mut *buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};

    fn hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    fn assert_corrupt(result: Result<Vec<u8>>) {
        assert_eq!(result, Err(KdbxError::Corrupt("outer cipher".into())));
    }

    #[test]
    fn aes_cbc_nist_sp800_38a_vector() {
        // NIST SP 800-38A F.2.5; only the first block precedes PKCS7 padding.
        let key: [u8; 32] = hex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4")
            .try_into()
            .unwrap();
        let iv = hex("000102030405060708090a0b0c0d0e0f");
        let plaintext = hex("6bc1bee22e409f96e93d7e117393172a");
        let encrypted = encrypt(Cipher::Aes256Cbc, &key, &iv, &plaintext).unwrap();
        assert_eq!(&encrypted[..16], hex("f58c4c04d6e5f1ba779eabfb5f7bfbd6"));
    }

    #[test]
    fn aes_cbc_round_trip_all_lengths_0_to_65() {
        for len in 0..=65 {
            let plaintext = vec![0x42; len];
            let encrypted =
                encrypt(Cipher::Aes256Cbc, &[0x5a; 32], &[0xa5; 16], &plaintext).unwrap();
            assert_eq!(encrypted.len(), (len / 16 + 1) * 16);
            assert_eq!(
                decrypt(Cipher::Aes256Cbc, &[0x5a; 32], &[0xa5; 16], &encrypted).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn aes_decrypt_non_block_multiple_is_corrupt() {
        for len in [0, 1, 15, 17, 31] {
            assert_corrupt(decrypt(
                Cipher::Aes256Cbc,
                &[0; 32],
                &[0; 16],
                &vec![0; len],
            ));
        }
    }

    #[test]
    fn aes_decrypt_bad_padding_is_corrupt() {
        // One padded block (02 02). CBC IV flips alter exactly the chosen plaintext
        // byte, leaving a deterministic malformed suffix instead of random padding.
        let encrypted = encrypt(Cipher::Aes256Cbc, &[0; 32], &[0; 16], &[0x42; 14]).unwrap();
        for (position, flip) in [(15, 2), (15, 19), (14, 1)] {
            let mut iv = [0; 16];
            iv[position] ^= flip; // zero length, length > 16, inconsistent bytes
            assert_corrupt(decrypt(Cipher::Aes256Cbc, &[0; 32], &iv, &encrypted));
        }
    }

    #[test]
    fn cipher_rejects_wrong_iv_lengths() {
        for cipher in [Cipher::Aes256Cbc, Cipher::ChaCha20] {
            for len in [0, 8, cipher.iv_len() - 1, cipher.iv_len() + 1, 24] {
                assert_corrupt(encrypt(cipher, &[0; 32], &vec![0; len], &[0; 16]));
                assert_corrupt(decrypt(cipher, &[0; 32], &vec![0; len], &[0; 16]));
            }
        }
    }

    #[test]
    fn chacha20_rfc8439_block_vector() {
        // RFC 8439 section 2.3.2 uses counter 1, unlike KDBX's counter 0.
        let key: [u8; 32] = hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
            .try_into()
            .unwrap();
        let nonce = hex("000000090000004a00000000");
        let expected = hex(concat!(
            "10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4e",
            "d2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e"
        ));
        let mut cipher = chacha20::ChaCha20::new_from_slices(&key, &nonce).unwrap();
        cipher.seek(64u64);
        let mut block = [0; 64];
        cipher.apply_keystream(&mut block);
        assert_eq!(block.as_slice(), expected);
        // Reach that same independent vector through production's counter-zero API.
        let encrypted = encrypt(Cipher::ChaCha20, &key, &nonce, &[0; 128]).unwrap();
        assert_eq!(&encrypted[64..], expected);
    }

    #[test]
    fn chacha20_counter_zero_vector() {
        let expected = hex(concat!(
            "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7",
            "da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586"
        ));
        assert_eq!(
            encrypt(Cipher::ChaCha20, &[0; 32], &[0; 12], &[0; 64]).unwrap(),
            expected
        );
        assert_eq!(
            decrypt(Cipher::ChaCha20, &[0; 32], &[0; 12], &expected).unwrap(),
            vec![0; 64]
        );
    }

    #[test]
    fn chacha20_round_trip() {
        for len in [0, 1, 63, 64, 65, 1024] {
            let plaintext = vec![0x42; len];
            let encrypted =
                encrypt(Cipher::ChaCha20, &[0x11; 32], &[0x22; 12], &plaintext).unwrap();
            assert_eq!(encrypted.len(), len);
            assert_eq!(
                decrypt(Cipher::ChaCha20, &[0x11; 32], &[0x22; 12], &encrypted).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn chacha20_different_nonce_changes_ciphertext() {
        assert_ne!(
            encrypt(Cipher::ChaCha20, &[0x33; 32], &[1; 12], &[0; 64]).unwrap(),
            encrypt(Cipher::ChaCha20, &[0x33; 32], &[2; 12], &[0; 64]).unwrap()
        );
    }
}
