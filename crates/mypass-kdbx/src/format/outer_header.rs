//! KDBX 4 outer header fields and their canonical wire order.

use super::{
    reader::Reader,
    signature::{parse_signature, write_signature, Version},
    variant_dict::VariantDictionary,
};
use crate::error::{KdbxError, Result};

const AES_UUID: [u8; 16] = [
    0x31, 0xc1, 0xf2, 0xe6, 0xbf, 0x71, 0x43, 0x50, 0xbe, 0x58, 0x05, 0x21, 0x6a, 0xfc, 0x5a, 0xff,
];
const CHACHA_UUID: [u8; 16] = [
    0xd6, 0x03, 0x8a, 0x2b, 0x8b, 0x6f, 0x4c, 0xb5, 0xa5, 0x24, 0x33, 0x9a, 0x31, 0xdb, 0xb5, 0x9a,
];
const END_MARKER: [u8; 4] = [13, 10, 13, 10];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cipher {
    Aes256Cbc,
    ChaCha20,
}

impl Cipher {
    pub fn iv_len(self) -> usize {
        match self {
            Self::Aes256Cbc => 16,
            Self::ChaCha20 => 12,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OuterHeader {
    pub version: Version,
    pub cipher: Cipher,
    pub compression: Compression,
    pub master_seed: Vec<u8>,
    pub encryption_iv: Vec<u8>,
    pub kdf_parameters: VariantDictionary,
    pub public_custom_data: Option<VariantDictionary>,
}

fn missing(name: &str) -> KdbxError {
    KdbxError::Corrupt(format!("missing {name}"))
}

fn fixed<const N: usize>(value: &[u8], name: &str) -> Result<[u8; N]> {
    value
        .try_into()
        .map_err(|_| KdbxError::Corrupt(format!("invalid {name} length")))
}

impl OuterHeader {
    /// The returned offset includes the signature and end field, but excludes the stored hashes
    /// and payload. Authenticate the original input prefix, never a reserialized header: unknown
    /// fields and field ordering are part of the authenticated bytes too.
    pub(crate) fn parse(data: &[u8]) -> Result<(OuterHeader, usize)> {
        let mut reader = Reader::new(data);
        let version = parse_signature(&mut reader)?;
        let (mut cipher, mut compression, mut master_seed, mut encryption_iv) =
            (None, None, None, None);
        let (mut kdf_parameters, mut public_custom_data) = (None, None);
        loop {
            let id = reader.u8()?;
            let len = usize::try_from(reader.u32_le()?)
                .map_err(|_| KdbxError::Corrupt("header field length".into()))?;
            // Check the untrusted length before copying or parsing anything. Unknown fields are
            // skipped without allocation, including obsolete KDBX3 fields.
            let value = reader.bytes(len)?;
            match id {
                0 => {
                    if value != END_MARKER {
                        return Err(KdbxError::Corrupt("end-of-header marker".into()));
                    }
                    break;
                }
                2 => {
                    cipher = Some(match fixed::<16>(value, "CipherID")? {
                        AES_UUID => Cipher::Aes256Cbc,
                        CHACHA_UUID => Cipher::ChaCha20,
                        _ => return Err(KdbxError::Unsupported("cipher UUID".into())),
                    })
                }
                3 => {
                    compression = Some(
                        match u32::from_le_bytes(fixed(value, "CompressionFlags")?) {
                            0 => Compression::None,
                            1 => Compression::Gzip,
                            _ => return Err(KdbxError::Unsupported("compression flags".into())),
                        },
                    )
                }
                4 => master_seed = Some(fixed::<32>(value, "MasterSeed")?.to_vec()),
                7 => encryption_iv = Some(value),
                11 => kdf_parameters = Some(VariantDictionary::parse(value)?),
                12 => public_custom_data = Some(VariantDictionary::parse(value)?),
                _ => {}
            }
        }
        let cipher = cipher.ok_or_else(|| missing("CipherID"))?;
        let compression = compression.ok_or_else(|| missing("CompressionFlags"))?;
        let master_seed = master_seed.ok_or_else(|| missing("MasterSeed"))?;
        let encryption_iv = encryption_iv.ok_or_else(|| missing("EncryptionIV"))?;
        let kdf_parameters = kdf_parameters.ok_or_else(|| missing("KdfParameters"))?;
        if encryption_iv.len() != cipher.iv_len() {
            return Err(KdbxError::Corrupt("invalid EncryptionIV length".into()));
        }
        Ok((
            Self {
                version,
                cipher,
                compression,
                master_seed,
                encryption_iv: encryption_iv.to_vec(),
                kdf_parameters,
                public_custom_data,
            },
            reader.pos(),
        ))
    }

    /// Writes the signature and fields in canonical order. As with VariantDictionary, field
    /// lengths above u32::MAX saturate; callers produce only representable headers.
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_signature(&mut out, self.version);
        let uuid = match self.cipher {
            Cipher::Aes256Cbc => &AES_UUID,
            Cipher::ChaCha20 => &CHACHA_UUID,
        };
        let compression: u32 = match self.compression {
            Compression::None => 0,
            Compression::Gzip => 1,
        };
        field(&mut out, 2, uuid);
        field(&mut out, 3, &compression.to_le_bytes());
        field(&mut out, 4, &self.master_seed);
        field(&mut out, 7, &self.encryption_iv);
        field(&mut out, 11, &self.kdf_parameters.serialize());
        if let Some(custom) = &self.public_custom_data {
            field(&mut out, 12, &custom.serialize());
        }
        field(&mut out, 0, &END_MARKER);
        out
    }
}

fn field(out: &mut Vec<u8>, id: u8, value: &[u8]) {
    out.push(id);
    out.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::KdbxError,
        format::{signature::write_signature, variant_dict::VariantValue},
    };

    pub(super) const AES: [u8; 16] = [
        0x31, 0xc1, 0xf2, 0xe6, 0xbf, 0x71, 0x43, 0x50, 0xbe, 0x58, 0x05, 0x21, 0x6a, 0xfc, 0x5a,
        0xff,
    ];
    const CHACHA: [u8; 16] = [
        0xd6, 0x03, 0x8a, 0x2b, 0x8b, 0x6f, 0x4c, 0xb5, 0xa5, 0x24, 0x33, 0x9a, 0x31, 0xdb, 0xb5,
        0x9a,
    ];
    fn sample() -> OuterHeader {
        let mut kdf = VariantDictionary::default();
        kdf.set("$UUID", VariantValue::Bytes(vec![7; 16]));
        OuterHeader {
            version: Version { major: 4, minor: 1 },
            cipher: Cipher::Aes256Cbc,
            compression: Compression::Gzip,
            master_seed: vec![1; 32],
            encryption_iv: vec![2; 16],
            kdf_parameters: kdf,
            public_custom_data: None,
        }
    }
    fn field(out: &mut Vec<u8>, id: u8, value: &[u8]) {
        out.push(id);
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
    }
    // Build wire bytes independently of OuterHeader::serialize, so parser tests do not depend on it.
    pub(super) fn wire(
        omit: Option<u8>,
        cipher: &[u8],
        iv: &[u8],
        extra: &[(u8, Vec<u8>)],
    ) -> Vec<u8> {
        let h = sample();
        let mut out = Vec::new();
        write_signature(&mut out, h.version);
        for (id, value) in [
            (2, cipher.to_vec()),
            (3, 1u32.to_le_bytes().to_vec()),
            (4, h.master_seed),
            (7, iv.to_vec()),
            (11, h.kdf_parameters.serialize()),
        ] {
            if omit != Some(id) {
                field(&mut out, id, &value);
            }
        }
        for (id, value) in extra {
            field(&mut out, *id, value);
        }
        field(&mut out, 0, &[13, 10, 13, 10]);
        out
    }
    #[test]
    fn minimal_header_round_trips() {
        let h = sample();
        let bytes = wire(None, &AES, &[2; 16], &[]);
        assert_eq!(h.serialize(), bytes);
        assert_eq!(OuterHeader::parse(&bytes).unwrap(), (h, bytes.len()));
    }
    #[test]
    fn chacha20_recognized_with_12_byte_iv() {
        let bytes = wire(None, &CHACHA, &[2; 12], &[]);
        let (h, _) = OuterHeader::parse(&bytes).unwrap();
        assert_eq!(h.cipher, Cipher::ChaCha20);
        assert_eq!(h.cipher.iv_len(), 12);
        assert_eq!(h.serialize(), bytes);
    }
    #[test]
    fn unknown_field_is_ignored() {
        let bytes = wire(None, &AES, &[2; 16], &[(250, vec![1, 2, 3])]);
        assert_eq!(OuterHeader::parse(&bytes).unwrap().0, sample());
    }
    #[test]
    fn kdbx3_only_fields_ignored() {
        let extra: Vec<_> = [5, 6, 8, 9, 10]
            .into_iter()
            .map(|id| (id, vec![9]))
            .collect();
        assert_eq!(
            OuterHeader::parse(&wire(None, &AES, &[2; 16], &extra))
                .unwrap()
                .0,
            sample()
        );
    }
    #[test]
    fn public_custom_data_parsed() {
        let mut custom = VariantDictionary::default();
        custom.set("test", VariantValue::String("value".into()));
        let bytes = wire(None, &AES, &[2; 16], &[(12, custom.serialize())]);
        let (h, _) = OuterHeader::parse(&bytes).unwrap();
        assert_eq!(h.public_custom_data, Some(custom));
        assert_eq!(h.serialize(), bytes);
    }
    #[test]
    fn truncated_header_is_corrupt() {
        let bytes = wire(None, &AES, &[2; 16], &[]);
        for len in 0..bytes.len() {
            assert!(
                matches!(
                    OuterHeader::parse(&bytes[..len]),
                    Err(KdbxError::Corrupt(_))
                ),
                "prefix {len}"
            );
        }
    }
    #[test]
    fn missing_master_seed_names_field() {
        assert!(
            matches!(OuterHeader::parse(&wire(Some(4), &AES, &[2;16], &[])), Err(KdbxError::Corrupt(s)) if s.contains("MasterSeed"))
        );
    }
    #[test]
    fn missing_kdf_parameters_names_field() {
        assert!(
            matches!(OuterHeader::parse(&wire(Some(11), &AES, &[2;16], &[])), Err(KdbxError::Corrupt(s)) if s.contains("KdfParameters"))
        );
    }
    #[test]
    fn bad_iv_length_is_corrupt() {
        for (cipher, iv) in [(&AES, vec![0; 12]), (&CHACHA, vec![0; 16])] {
            assert!(matches!(
                OuterHeader::parse(&wire(None, cipher, &iv, &[])),
                Err(KdbxError::Corrupt(_))
            ));
        }
    }
    #[test]
    fn unknown_cipher_is_unsupported() {
        assert!(matches!(
            OuterHeader::parse(&wire(None, &[0; 16], &[2; 16], &[])),
            Err(KdbxError::Unsupported(_))
        ));
    }
}

#[cfg(test)]
mod edge_tests {
    use super::tests::{wire, AES};
    use super::*;
    use crate::error::KdbxError;
    #[test]
    fn offset_excludes_authentication_and_payload() {
        let mut bytes = wire(None, &AES, &[2; 16], &[]);
        let expected = bytes.len();
        bytes.extend_from_slice(&[0xaa; 100]);
        assert_eq!(OuterHeader::parse(&bytes).unwrap().1, expected);
    }
    #[test]
    fn missing_cipher_compression_and_iv_name_field() {
        for (id, name) in [
            (2, "CipherID"),
            (3, "CompressionFlags"),
            (7, "EncryptionIV"),
        ] {
            assert!(
                matches!(OuterHeader::parse(&wire(Some(id), &AES, &[2;16], &[])), Err(KdbxError::Corrupt(s)) if s.contains(name))
            );
        }
    }
    #[test]
    fn malformed_field_lengths_and_end_marker_are_corrupt() {
        for (id, value) in [
            (2, vec![0; 15]),
            (3, vec![0; 3]),
            (4, vec![0; 31]),
            (11, vec![0]),
            (12, vec![0]),
            (0, vec![0; 4]),
        ] {
            let bytes = wire(Some(id), &AES, &[2; 16], &[(id, value)]);
            assert!(
                matches!(OuterHeader::parse(&bytes), Err(KdbxError::Corrupt(_))),
                "field {id}"
            );
        }
        let mut bytes = wire(None, &AES, &[2; 16], &[]);
        bytes[13..17].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            OuterHeader::parse(&bytes),
            Err(KdbxError::Corrupt(_))
        ));
    }
    #[test]
    fn uncompressed_header_and_minor_versions_are_preserved() {
        for minor in [0, 1, 2, u16::MAX] {
            let mut bytes = wire(Some(3), &AES, &[2; 16], &[(3, 0u32.to_le_bytes().to_vec())]);
            bytes[8..10].copy_from_slice(&minor.to_le_bytes());
            let (h, _) = OuterHeader::parse(&bytes).unwrap();
            assert_eq!(h.compression, Compression::None);
            assert_eq!(h.version.minor, minor);
            assert_eq!(OuterHeader::parse(&h.serialize()).unwrap().0, h);
        }
    }
    #[test]
    fn unsupported_compression_is_reported() {
        let bytes = wire(Some(3), &AES, &[2; 16], &[(3, 2u32.to_le_bytes().to_vec())]);
        assert!(matches!(
            OuterHeader::parse(&bytes),
            Err(KdbxError::Unsupported(_))
        ));
    }
}
