//! Authenticated KDBX 4 payload blocks.

use crate::error::{KdbxError, Result};
use crate::format::{limits::MAX_BLOCK_LEN, reader::Reader};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;

pub(crate) const DEFAULT_BLOCK_SIZE: usize = 1 << 20;

pub(crate) fn read(data: &[u8], hmac_key_base: &[u8; 64]) -> Result<Vec<u8>> {
    let mut reader = Reader::new(data);
    let mut plain = Vec::new();
    let mut index = 0_u64;
    loop {
        // Check framing before reading, including the mandatory authenticated terminator.
        if reader.remaining() < 36 {
            return Err(KdbxError::Corrupt("truncated".into()));
        }
        let stored_mac = reader.bytes(32)?;
        let length = reader.u32_le()?;
        if u64::from(length) > MAX_BLOCK_LEN as u64 {
            return Err(KdbxError::Corrupt("block length exceeds limit".into()));
        }
        if u64::from(length) > reader.remaining() as u64 {
            return Err(KdbxError::Corrupt("truncated".into()));
        }
        // The checks above bound the conversion, slicing and output growth by actual input.
        let payload = reader.bytes(length as usize)?;
        block_mac(index, length, payload, hmac_key_base)
            .verify_slice(stored_mac)
            .map_err(|_| KdbxError::Corrupt(format!("block {index} HMAC")))?;
        if length == 0 {
            if reader.remaining() != 0 {
                return Err(KdbxError::Corrupt("trailing block data".into()));
            }
            return Ok(plain);
        }
        plain.extend_from_slice(payload);
        index = index
            .checked_add(1)
            .ok_or_else(|| KdbxError::Corrupt("block index overflow".into()))?;
    }
}

/// A zero block size uses the default; oversized sizes are capped to the reader's block limit.
pub(crate) fn write(plain: &[u8], hmac_key_base: &[u8; 64], block_size: usize) -> Vec<u8> {
    let block_size = effective_block_size(block_size);
    let mut stream = Vec::new();
    let mut index = 0_u64;
    for payload in plain.chunks(block_size) {
        append_block(&mut stream, index, payload, hmac_key_base);
        // A slice can contain at most isize::MAX bytes, and each chunk is nonempty, so a
        // u64 index cannot wrap on supported targets. Explicit wrapping also never panics.
        index = index.wrapping_add(1);
    }
    append_block(&mut stream, index, &[], hmac_key_base);
    stream
}

fn effective_block_size(block_size: usize) -> usize {
    if block_size == 0 {
        DEFAULT_BLOCK_SIZE
    } else {
        block_size.min(MAX_BLOCK_LEN)
    }
}

fn block_mac(index: u64, length: u32, payload: &[u8], base: &[u8; 64]) -> Hmac<Sha256> {
    let mut hash = Sha512::new();
    Digest::update(&mut hash, index.to_le_bytes());
    Digest::update(&mut hash, base);
    let key = Zeroizing::new(<[u8; 64]>::from(hash.finalize()));
    // SHA-512 yields exactly the HMAC-SHA256 key/block size; initialization is infallible.
    let mut mac = Hmac::<Sha256>::new((&*key).into());
    Mac::update(&mut mac, &index.to_le_bytes());
    Mac::update(&mut mac, &length.to_le_bytes());
    Mac::update(&mut mac, payload);
    mac
}

fn append_block(stream: &mut Vec<u8>, index: u64, payload: &[u8], base: &[u8; 64]) {
    // Only called with chunks capped to MAX_BLOCK_LEN (2^30), or the empty terminator.
    let length = payload.len() as u32;
    stream.extend_from_slice(
        &block_mac(index, length, payload, base)
            .finalize()
            .into_bytes(),
    );
    stream.extend_from_slice(&length.to_le_bytes());
    stream.extend_from_slice(payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::{Digest, Sha256, Sha512};

    #[test]
    fn clamp_and_terminator_authentication_order() {
        assert_eq!(effective_block_size(0), DEFAULT_BLOCK_SIZE);
        assert_eq!(effective_block_size(1), 1);
        assert_eq!(effective_block_size(usize::MAX), MAX_BLOCK_LEN);
        let mut data = write(b"x", &[0; 64], 1);
        data[37] ^= 1;
        data.push(0);
        assert_eq!(
            read(&data, &[0; 64]),
            Err(KdbxError::Corrupt("block 1 HMAC".into()))
        );
    }
    #[test]
    fn literal_block_mac_known_answer() {
        // Python stdlib hashlib/hmac, index 0, all-zero base, one-byte payload.
        let actual = write(b"x", &[0; 64], 1);
        let expected = "121a996cae5055030800887dcfe257bc1fdc3eea197d2e55de1dbb915f50eae8";
        let bytes: Vec<u8> = (0..64)
            .step_by(2)
            .map(|i| u8::from_str_radix(&expected[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(&actual[..32], bytes);
    }
    // Independent oracle: raw index/length bytes and RustCrypto calls only; no production helpers.
    fn oracle_block(index: u64, payload: &[u8], base: &[u8; 64]) -> Vec<u8> {
        let mut key_input = index.to_le_bytes().to_vec();
        key_input.extend_from_slice(base);
        let key = Sha512::digest(&key_input);
        let mut message = index.to_le_bytes().to_vec();
        message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        message.extend_from_slice(payload);
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key).unwrap();
        mac.update(&message);
        let mut block = mac.finalize().into_bytes().to_vec();
        block.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        block.extend_from_slice(payload);
        block
    }

    fn oracle_stream(payloads: &[&[u8]], base: &[u8; 64]) -> Vec<u8> {
        let mut stream = Vec::new();
        for (index, payload) in payloads.iter().enumerate() {
            stream.extend(oracle_block(index as u64, payload, base));
        }
        stream.extend(oracle_block(payloads.len() as u64, b"", base));
        stream
    }

    fn assert_corrupt(stream: &[u8], base: &[u8; 64], message: &str) {
        assert_eq!(read(stream, base), Err(KdbxError::Corrupt(message.into())));
    }

    #[test]
    fn single_block_plus_terminator() {
        let base = [0x42; 64];
        let stream = oracle_stream(&[b"hello kdbx4"], &base);
        assert_eq!(read(&stream, &base).unwrap(), b"hello kdbx4");
        assert_eq!(write(b"hello kdbx4", &base, 1024), stream);
    }

    #[test]
    fn multiple_blocks_concatenate() {
        let base = [0x77; 64];
        let stream = oracle_stream(&[b"0123", b"4567", b"89"], &base);
        assert_eq!(read(&stream, &base).unwrap(), b"0123456789");
        assert_eq!(write(b"0123456789", &base, 4), stream);
    }

    #[test]
    fn write_then_read_round_trip() {
        let payload: Vec<u8> = (0..5000).map(|n| (n % 256) as u8).collect();
        assert_eq!(
            read(&write(&payload, &[0x77; 64], 1024), &[0x77; 64]).unwrap(),
            payload
        );
    }

    #[test]
    fn empty_payload_produces_only_terminator() {
        let base = [0x88; 64];
        let stream = write(b"", &base, 1024);
        assert_eq!(stream, oracle_block(0, b"", &base));
        assert_eq!(stream.len(), 36);
        assert_eq!(read(&stream, &base).unwrap(), b"");
    }

    #[test]
    fn default_block_size_is_one_mebibyte() {
        assert_eq!(DEFAULT_BLOCK_SIZE, 1 << 20);
    }

    #[test]
    fn default_block_size_round_trips() {
        let payload = vec![0x53; (5 * (1 << 20)) / 2];
        let stream = write(&payload, &[0x11; 64], DEFAULT_BLOCK_SIZE);
        assert_eq!(stream.len(), payload.len() + 4 * 36);
        assert_eq!(
            u32::from_le_bytes(stream[32..36].try_into().unwrap()),
            1 << 20
        );
        assert_eq!(read(&stream, &[0x11; 64]).unwrap(), payload);
        assert_eq!(
            stream,
            oracle_stream(
                &[
                    &payload[..1 << 20],
                    &payload[1 << 20..2 << 20],
                    &payload[2 << 20..]
                ],
                &[0x11; 64]
            )
        );
    }

    #[test]
    fn corrupted_data_is_corrupt() {
        let base = [0x42; 64];
        let mut stream = oracle_stream(&[b"original"], &base);
        stream[36] ^= 0xff; // Data only: all framing and stored MAC bytes remain intact.
        assert_corrupt(&stream, &base, "block 0 HMAC");
    }

    #[test]
    fn corrupted_mac_is_corrupt() {
        let base = [0x11; 64];
        let mut stream = oracle_stream(&[b"payload"], &base);
        stream[0] ^= 0xff; // MAC only: the payload and lengths remain intact.
        assert_corrupt(&stream, &base, "block 0 HMAC");
    }

    #[test]
    fn truncated_stream_is_corrupt() {
        let base = [0x99; 64];
        let stream = oracle_stream(&[b"first", b"second"], &base);
        for end in 0..stream.len() {
            assert_corrupt(&stream[..end], &base, "truncated");
        }
    }

    #[test]
    fn empty_stream_is_corrupt() {
        assert_corrupt(b"", &[0x33; 64], "truncated");
    }

    #[test]
    fn block_stream_tamper_is_corrupt_not_wrong_password() {
        let base = [1; 64];
        let stream = oracle_stream(&[b"payload"], &base);
        assert!(matches!(
            read(&stream, &[2; 64]),
            Err(KdbxError::Corrupt(_))
        ));
    }

    #[test]
    fn declared_length_past_end_is_corrupt() {
        let mut stream = vec![0; 32];
        stream.extend_from_slice(&5_u32.to_le_bytes());
        stream.extend_from_slice(b"four");
        assert_corrupt(&stream, &[0; 64], "truncated");
    }

    #[test]
    fn length_cap_has_a_distinct_error_and_accepts_exact_boundary() {
        assert_eq!(crate::format::limits::MAX_BLOCK_LEN, 1 << 30);
        for (length, message) in [
            (1_u32 << 30, "truncated"),
            ((1_u32 << 30) + 1, "block length exceeds limit"),
        ] {
            let mut stream = vec![0; 32];
            stream.extend_from_slice(&length.to_le_bytes());
            assert_corrupt(&stream, &[0; 64], message);
        }
    }

    #[test]
    fn terminator_mac_is_verified_at_its_own_index() {
        let base = [0x88; 64];
        let mut stream = oracle_stream(&[b"data"], &base);
        stream[40] ^= 1;
        assert_corrupt(&stream, &base, "block 1 HMAC");
        let wrong_index = [oracle_block(0, b"data", &base), oracle_block(0, b"", &base)].concat();
        assert_corrupt(&wrong_index, &base, "block 1 HMAC");
    }

    #[test]
    fn trailing_data_after_terminator_is_corrupt() {
        let base = [0x77; 64];
        for tail in [
            vec![0],
            oracle_block(2, b"extra", &base),
            oracle_block(2, b"", &base),
        ] {
            let mut stream = oracle_stream(&[b"data"], &base);
            stream.extend(tail);
            assert_corrupt(&stream, &base, "trailing block data");
        }
    }

    #[test]
    fn reordered_blocks_fail_authentication() {
        let base = [0x77; 64];
        let stream = [
            oracle_block(1, b"second", &base),
            oracle_block(0, b"first", &base),
            oracle_block(2, b"", &base),
        ]
        .concat();
        assert_corrupt(&stream, &base, "block 0 HMAC");
    }

    #[test]
    fn zero_block_size_uses_default() {
        let payload = vec![0x55; DEFAULT_BLOCK_SIZE + 1];
        assert_eq!(
            write(&payload, &[3; 64], 0),
            oracle_stream(
                &[
                    &payload[..DEFAULT_BLOCK_SIZE],
                    &payload[DEFAULT_BLOCK_SIZE..]
                ],
                &[3; 64]
            )
        );
    }

    #[test]
    fn oversized_block_size_still_writes_small_payload() {
        assert_eq!(
            write(b"small", &[4; 64], usize::MAX),
            oracle_stream(&[b"small"], &[4; 64])
        );
    }
}

#[cfg(test)]
mod authentication_order_tests {
    use super::*;
    #[test]
    fn tampered_terminator_mac_precedes_trailing_data_error() {
        let base = [7; 64];
        let mut wire = write(b"x", &base, 0);
        let terminator = wire.len() - 36;
        wire[terminator] ^= 1;
        wire.push(0);
        assert_eq!(
            read(&wire, &base),
            Err(KdbxError::Corrupt("block 1 HMAC".into()))
        );
    }
}
