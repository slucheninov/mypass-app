//! Gzip payload framing with a bounded decompression reader.
use std::io::{Read, Write};

use flate2::{bufread::GzDecoder, write::GzEncoder, Compression};
use zeroize::Zeroizing;

use super::limits::MAX_DECOMPRESSED_LEN;
use crate::error::{KdbxError, Result};

pub(crate) fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    // GzEncoder's default header has mtime zero (no clock or filename metadata).
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|_| KdbxError::Corrupt("gzip payload".into()))?;
    encoder
        .finish()
        .map_err(|_| KdbxError::Corrupt("gzip payload".into()))
}

pub(crate) fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    gunzip_with_limit(data, MAX_DECOMPRESSED_LEN)
}

fn gunzip_with_limit(data: &[u8], limit: u64) -> Result<Vec<u8>> {
    // Read just one byte beyond the bound to distinguish exactly-at-limit success.
    // Never trust the gzip trailer's attacker-controlled uncompressed size.
    let mut reader = GzDecoder::new(data).take(limit.saturating_add(1));
    // Best-effort erasure: Vec growth may leave earlier allocations unwiped.
    // Preallocating the 1 GiB cap for every vault would itself amplify hostile input.
    let mut output = Zeroizing::new(Vec::new());
    reader
        .read_to_end(&mut output)
        .map_err(|_| KdbxError::Corrupt("gzip payload".into()))?;
    if output.len() as u64 > limit {
        return Err(KdbxError::Corrupt("decompressed payload too large".into()));
    }
    if !reader.into_inner().into_inner().is_empty() {
        return Err(KdbxError::Corrupt("trailing gzip data".into()));
    }
    Ok(std::mem::take(&mut *output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_has_magic_1f8b() {
        let compressed = gzip(b"Hello, KeePassXC!").unwrap();
        assert_eq!(&compressed[..2], &[0x1f, 0x8b]);
        assert_eq!(&compressed[8..10], &[0, 255]);
        assert_eq!(&compressed[4..8], &[0; 4]); // deterministic mtime
        assert_eq!(compressed, gzip(b"Hello, KeePassXC!").unwrap());
    }

    #[test]
    fn gunzip_reads_known_gzip_bytes() {
        // Generated ONCE with system `gzip -n -c`, never shell out in tests.
        const HEX: &str = "1f8b08000000000000030bca2fcd4bd12d29ca2c5028c928ca2f4dcf00d2a90ac595c525a9b90ae95540f1a4ccbcc4a24a1d85bcfc1285acd2e21285fcd22285fcf23c85e4fc94543d2e00902d24a942000000";
        let bytes: Vec<u8> = HEX
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(
            gunzip(&bytes).unwrap(),
            b"Round-trip through the system gzip binary, not just our own code.\n"
        );
    }

    #[test]
    fn gzip_round_trip_binary_all_byte_values() {
        let data: Vec<u8> = (0..=255).collect();
        for input in [data.as_slice(), &[]] {
            assert_eq!(gunzip(&gzip(input).unwrap()).unwrap(), input);
        }
    }

    #[test]
    fn gunzip_rejects_garbage() {
        for input in [b"not gzip".as_slice(), &[], &[0x1f, 0x8b]] {
            assert_eq!(
                gunzip(input),
                Err(KdbxError::Corrupt("gzip payload".into()))
            );
        }
    }

    #[test]
    fn gunzip_rejects_decompression_bomb() {
        let compressed = gzip(&vec![0; 2 * 1024 * 1024]).unwrap();
        assert_eq!(
            gunzip_with_limit(&compressed, 1024),
            Err(KdbxError::Corrupt("decompressed payload too large".into()))
        );
    }

    #[test]
    fn gunzip_limit_boundary() {
        assert_eq!(
            gunzip_with_limit(&gzip(&vec![0x42; 1024]).unwrap(), 1024).unwrap(),
            vec![0x42; 1024]
        );
        assert_eq!(
            gunzip_with_limit(&gzip(&vec![0x42; 1025]).unwrap(), 1024),
            Err(KdbxError::Corrupt("decompressed payload too large".into()))
        );
    }

    #[test]
    fn gunzip_rejects_truncated_and_bad_checksum() {
        let compressed = gzip(b"payload").unwrap();
        assert_eq!(
            gunzip(&compressed[..compressed.len() - 1]),
            Err(KdbxError::Corrupt("gzip payload".into()))
        );
        let mut damaged = compressed;
        let crc = damaged.len() - 8;
        damaged[crc] ^= 1;
        assert_eq!(
            gunzip(&damaged),
            Err(KdbxError::Corrupt("gzip payload".into()))
        );
    }
}

#[cfg(test)]
mod trailing_tests {
    use super::*;
    #[test]
    fn trailing_bytes_and_second_member_are_corrupt() {
        let first = gzip(b"first").unwrap();
        assert_eq!(gunzip(&first).unwrap(), b"first");
        for suffix in [vec![0], gzip(b"second").unwrap()] {
            let mut data = first.clone();
            data.extend(suffix);
            assert!(matches!(gunzip(&data), Err(KdbxError::Corrupt(_))));
        }
    }
}
