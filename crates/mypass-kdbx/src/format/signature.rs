//! KDBX signature and version words.

use super::reader::Reader;
use crate::error::{KdbxError, Result};

/// First magic word of every KeePass file.
const SIGNATURE_1: u32 = 0x9AA2_D903;
/// Second magic word of KDBX 2.x/3.x/4.x files.
const SIGNATURE_2: u32 = 0xB54B_FB67;
/// Length of the magic words plus the minor and major version words.
const SIGNATURE_LEN: usize = 12;
/// Only KDBX major version 4 is read.
const SUPPORTED_MAJOR: u16 = 4;
/// Historical major-1 word is also classified as NotKdbx, even with KDBX magic.
const KDB1_MAJOR: u16 = 1;

/// The file format version from the signature block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Version {
    pub major: u16,
    pub minor: u16,
}

fn magic_bytes() -> [u8; 8] {
    let mut magic = [0u8; 8];
    let (first, second) = magic.split_at_mut(4);
    first.copy_from_slice(&SIGNATURE_1.to_le_bytes());
    second.copy_from_slice(&SIGNATURE_2.to_le_bytes());
    magic
}

/// Reads and validates the 12-byte signature block.
///
/// * Bytes that do not match the KDBX magic (including KDB1) give `NotKdbx`.
/// * Input shorter than 12 bytes gives `NotKdbx` when the bytes present already differ from the
///   magic, otherwise `Corrupt("truncated signature")` (a KDBX file that was cut short).
/// * Major 2 and 3 give `UnsupportedVersion`; so does any major other than 4 (e.g. a future 5).
///   Any minor of major 4 is accepted: minors only add optional features.
pub(crate) fn parse_signature(r: &mut Reader<'_>) -> Result<Version> {
    if r.remaining() < SIGNATURE_LEN {
        let present = r.bytes(r.remaining())?;
        let matches_magic = present.iter().zip(magic_bytes()).all(|(a, b)| *a == b);
        return Err(if matches_magic {
            KdbxError::Corrupt("truncated signature".into())
        } else {
            KdbxError::NotKdbx
        });
    }
    if r.u32_le()? != SIGNATURE_1 || r.u32_le()? != SIGNATURE_2 {
        return Err(KdbxError::NotKdbx);
    }
    let minor = r.u16_le()?;
    let major = r.u16_le()?;
    match major {
        SUPPORTED_MAJOR => Ok(Version { major, minor }),
        KDB1_MAJOR => Err(KdbxError::NotKdbx),
        _ => Err(KdbxError::UnsupportedVersion { major, minor }),
    }
}

/// Appends the 12-byte signature block for `version`.
pub(crate) fn write_signature(out: &mut Vec<u8>, version: Version) {
    out.extend_from_slice(&SIGNATURE_1.to_le_bytes());
    out.extend_from_slice(&SIGNATURE_2.to_le_bytes());
    out.extend_from_slice(&version.minor.to_le_bytes());
    out.extend_from_slice(&version.major.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::KdbxError;

    fn parse(bytes: &[u8]) -> crate::error::Result<Version> {
        parse_signature(&mut Reader::new(bytes))
    }

    const MAGIC: [u8; 8] = [0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5];

    fn with_version(minor: u16, major: u16) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&minor.to_le_bytes());
        v.extend_from_slice(&major.to_le_bytes());
        v
    }

    #[test]
    fn parses_kdbx4_signature() {
        let bytes = [
            0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5, 0x00, 0x00, 0x04, 0x00,
        ];
        let mut r = Reader::new(&bytes);
        assert_eq!(
            parse_signature(&mut r).unwrap(),
            Version { major: 4, minor: 0 }
        );
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn any_kdbx4_minor_is_accepted() {
        for minor in [0u16, 1, 2, 0xFFFF] {
            assert_eq!(
                parse(&with_version(minor, 4)).unwrap(),
                Version { major: 4, minor }
            );
        }
    }

    #[test]
    fn kdbx3_is_unsupported_version() {
        assert_eq!(
            parse(&with_version(0, 3)),
            Err(KdbxError::UnsupportedVersion { major: 3, minor: 0 })
        );
        assert_eq!(
            parse(&with_version(1, 2)),
            Err(KdbxError::UnsupportedVersion { major: 2, minor: 1 })
        );
    }

    #[test]
    fn future_major_is_unsupported_version() {
        assert_eq!(
            parse(&with_version(0, 5)),
            Err(KdbxError::UnsupportedVersion { major: 5, minor: 0 })
        );
    }

    #[test]
    fn bad_signature_is_not_kdbx() {
        assert_eq!(parse(&[0u8; 12]), Err(KdbxError::NotKdbx));
        assert_eq!(parse(b"<?xml version=\"1.0\"?>"), Err(KdbxError::NotKdbx));
        let mut bad_second = with_version(0, 4);
        bad_second[4] ^= 0xFF;
        assert_eq!(parse(&bad_second), Err(KdbxError::NotKdbx));
    }

    #[test]
    fn kdb1_signature_is_not_kdbx() {
        assert_eq!(parse(&with_version(0, 1)), Err(KdbxError::NotKdbx));
        let bytes = [
            0x03, 0xD9, 0xA2, 0x9A, 0x65, 0xFB, 0x4B, 0xB5, 0x00, 0x00, 0x01, 0x00,
        ];
        assert_eq!(parse(&bytes), Err(KdbxError::NotKdbx));
        // Even when the KDB1 file is cut short, the second magic word already differs.
        assert_eq!(parse(&bytes[..8]), Err(KdbxError::NotKdbx));
    }

    #[test]
    fn truncated_signature_is_corrupt() {
        let full = with_version(0, 4);
        assert_eq!(
            parse(&full[..11]),
            Err(KdbxError::Corrupt("truncated signature".into()))
        );
        // Every proper prefix of a valid signature is a truncated KDBX file, including empty input.
        for len in 0..12 {
            assert_eq!(
                parse(&full[..len]),
                Err(KdbxError::Corrupt("truncated signature".into())),
                "prefix of {len} bytes"
            );
        }
    }

    #[test]
    fn short_input_that_mismatches_the_magic_is_not_kdbx() {
        assert_eq!(parse(&[0x00]), Err(KdbxError::NotKdbx));
        assert_eq!(parse(b"hello"), Err(KdbxError::NotKdbx));
        assert_eq!(
            parse(&MAGIC[..4]).unwrap_err(),
            KdbxError::Corrupt("truncated signature".into())
        );
        let mut wrong_tail = MAGIC[..6].to_vec();
        wrong_tail[5] = 0x00;
        assert_eq!(parse(&wrong_tail), Err(KdbxError::NotKdbx));
    }

    #[test]
    fn write_then_parse_round_trips_and_matches_the_wire_format() {
        let mut out = Vec::new();
        write_signature(&mut out, Version { major: 4, minor: 1 });
        assert_eq!(out, with_version(1, 4));
        assert_eq!(parse(&out).unwrap(), Version { major: 4, minor: 1 });
    }
}
