//! KDBX inner header fields preceding the XML payload.
use std::fmt;
use zeroize::Zeroizing;

use super::reader::Reader;
use crate::error::{KdbxError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InnerStreamKind {
    Salsa20,
    ChaCha20,
}

#[derive(Clone, PartialEq, Eq)]
pub struct InnerBinary {
    pub data: Vec<u8>,
    pub protected: bool,
}

impl fmt::Debug for InnerBinary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Even unprotected attachments may contain secrets.
        f.debug_struct("InnerBinary")
            .field("data_len", &self.data.len())
            .field("protected", &self.protected)
            .finish()
    }
}

pub struct InnerHeader {
    pub stream: InnerStreamKind,
    pub stream_key: Zeroizing<Vec<u8>>,
    pub binaries: Vec<InnerBinary>,
}

impl fmt::Debug for InnerHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InnerHeader")
            .field("stream", &self.stream)
            .field("stream_key_len", &self.stream_key.len())
            .field("binaries", &self.binaries)
            .finish()
    }
}

impl InnerHeader {
    pub(crate) fn parse(data: &[u8]) -> Result<(Self, usize)> {
        let mut reader = Reader::new(data);
        let mut stream = None;
        let mut stream_key = None;
        let mut binaries = Vec::new();
        loop {
            // Each iteration consumes at least five bytes. Reader validates the
            // entire field against remaining input before any copy/allocation.
            let id = reader.u8()?;
            let len =
                usize::try_from(reader.u32_le()?).map_err(|_| corrupt("inner field length"))?;
            let value = reader.bytes(len)?;
            match id {
                0 => {
                    if !value.is_empty() {
                        return Err(corrupt("inner end field must be empty"));
                    }
                    break;
                }
                1 => {
                    if value.len() != 4 {
                        return Err(corrupt("inner stream id length"));
                    }
                    stream = Some(match Reader::new(value).u32_le()? {
                        2 => InnerStreamKind::Salsa20,
                        3 => InnerStreamKind::ChaCha20,
                        _ => return Err(KdbxError::Unsupported("inner stream id".into())),
                    });
                }
                2 => {
                    if value.is_empty() {
                        return Err(corrupt("empty inner stream key"));
                    }
                    stream_key = Some(Zeroizing::new(value.to_vec()));
                }
                3 => {
                    let (flags, content) = value
                        .split_first()
                        .ok_or_else(|| corrupt("missing inner binary flag"))?;
                    binaries.push(InnerBinary {
                        data: content.to_vec(),
                        protected: flags & 1 != 0,
                    });
                }
                _ => {} // Unknown fields are skipped without copying.
            }
        }
        Ok((
            Self {
                stream: stream.ok_or_else(|| corrupt("missing inner stream id"))?,
                stream_key: stream_key.ok_or_else(|| corrupt("missing inner stream key"))?,
                binaries,
            },
            reader.pos(),
        ))
    }

    #[allow(dead_code)] // remove in Task 9
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let id: u32 = match self.stream {
            InnerStreamKind::Salsa20 => 2,
            InnerStreamKind::ChaCha20 => 3,
        };
        write_field_prefix(&mut out, 1, 4);
        out.extend_from_slice(&id.to_le_bytes());
        write_field_prefix(&mut out, 2, self.stream_key.len());
        out.extend_from_slice(&self.stream_key);
        for binary in &self.binaries {
            write_field_prefix(&mut out, 3, binary.data.len().saturating_add(1));
            out.push(u8::from(binary.protected));
            out.extend_from_slice(&binary.data);
        }
        write_field_prefix(&mut out, 0, 0);
        out
    }
}

fn corrupt(message: &str) -> KdbxError {
    KdbxError::Corrupt(message.into())
}

#[allow(dead_code)] // remove in Task 9
fn write_field_prefix(out: &mut Vec<u8>, id: u8, len: usize) {
    out.push(id);
    // Parsed fields fit u32 by construction; match the outer-header serializer
    // for caller-constructed fields beyond the wire format's representable size.
    out.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_wire_field(out: &mut Vec<u8>, id: u8, value: &[u8]) {
        out.push(id);
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
    }
    fn valid_wire() -> Vec<u8> {
        // Independent framing: ChaCha20, a nonempty key, two binaries and end.
        vec![
            1, 4, 0, 0, 0, 3, 0, 0, 0, 2, 3, 0, 0, 0, 0x11, 0x22, 0x33, 3, 3, 0, 0, 0, 0, 0xaa,
            0xbb, 3, 2, 0, 0, 0, 1, 0xcc, 0, 0, 0, 0, 0,
        ]
    }
    fn assert_corrupt_wire(bytes: &[u8]) {
        assert!(matches!(
            InnerHeader::parse(bytes),
            Err(KdbxError::Corrupt(_))
        ));
    }
    #[test]
    fn serialize_then_parse_round_trip() {
        let header = InnerHeader {
            stream: InnerStreamKind::ChaCha20,
            stream_key: Zeroizing::new(vec![0x11, 0x22, 0x33]),
            binaries: vec![
                InnerBinary {
                    data: vec![0xaa, 0xbb],
                    protected: false,
                },
                InnerBinary {
                    data: vec![0xcc],
                    protected: true,
                },
            ],
        };
        assert_eq!(header.serialize(), valid_wire());
        let (parsed, offset) = InnerHeader::parse(&valid_wire()).unwrap();
        assert_eq!(parsed.stream, header.stream);
        assert_eq!(parsed.stream_key, header.stream_key);
        assert!(parsed.binaries == header.binaries);
        assert_eq!(offset, valid_wire().len());
    }
    #[test]
    fn missing_stream_key_is_corrupt() {
        // All other requirements, including a valid end field, are present.
        assert_corrupt_wire(&[1, 4, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    #[test]
    fn missing_stream_id_is_corrupt() {
        assert_corrupt_wire(&[2, 1, 0, 0, 0, 0x11, 0, 0, 0, 0, 0]);
    }
    #[test]
    fn unknown_stream_id_is_unsupported() {
        let mut bytes = valid_wire();
        bytes[5..9].copy_from_slice(&99u32.to_le_bytes());
        assert!(matches!(
            InnerHeader::parse(&bytes),
            Err(KdbxError::Unsupported(_))
        ));
    }
    #[test]
    fn arcfour_is_unsupported() {
        let mut bytes = valid_wire();
        bytes[5..9].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            InnerHeader::parse(&bytes),
            Err(KdbxError::Unsupported(_))
        ));
    }
    #[test]
    fn truncated_is_corrupt() {
        let bytes = valid_wire();
        assert!(
            InnerHeader::parse(&bytes).is_ok(),
            "complete fixture must be valid"
        );
        for end in 0..bytes.len() {
            assert!(
                matches!(
                    InnerHeader::parse(&bytes[..end]),
                    Err(KdbxError::Corrupt(_))
                ),
                "prefix {end}"
            );
        }
    }
    #[test]
    fn parse_stops_at_end_field_leaving_xml_untouched() {
        let mut bytes = valid_wire();
        let header_len = bytes.len();
        bytes.extend_from_slice(b"<KeePassFile></KeePassFile>");
        let (_, offset) = InnerHeader::parse(&bytes).unwrap();
        assert_eq!(offset, header_len);
        assert_eq!(&bytes[offset..], b"<KeePassFile></KeePassFile>");
    }
    #[test]
    fn unknown_field_ignored() {
        let mut bytes = Vec::new();
        append_wire_field(&mut bytes, 42, &[0xaa, 0xbb, 0xcc]);
        bytes.extend(valid_wire());
        let (parsed, offset) = InnerHeader::parse(&bytes).unwrap();
        assert_eq!(offset, bytes.len());
        assert_eq!(&*parsed.stream_key, &[0x11, 0x22, 0x33]);
        assert_eq!(parsed.binaries.len(), 2);
    }
    #[test]
    fn malformed_fields_are_corrupt() {
        for (id, payload) in [
            (1, vec![]),
            (1, vec![3; 3]),
            (1, vec![3, 0, 0, 0, 0]),
            (2, vec![]),
            (3, vec![]),
            (0, vec![0]),
        ] {
            let mut bytes = Vec::new();
            append_wire_field(&mut bytes, id, &payload);
            bytes.extend(valid_wire());
            assert_corrupt_wire(&bytes);
        }
        // Reject hostile u32 length before any allocation, including unknown fields.
        assert_corrupt_wire(&[42, 255, 255, 255, 255]);
    }
    #[test]
    fn salsa_id_and_binary_flag_bit_are_preserved() {
        let mut bytes = valid_wire();
        bytes[5] = 2;
        bytes[22] = 0xfe;
        let (parsed, _) = InnerHeader::parse(&bytes).unwrap();
        assert_eq!(parsed.stream, InnerStreamKind::Salsa20);
        assert!(!parsed.binaries[0].protected);
        assert!(parsed.binaries[1].protected);
        assert_eq!(parsed.serialize()[5], 2);
    }
    #[test]
    fn debug_redacts_keys_and_all_attachment_data() {
        let header = InnerHeader {
            stream: InnerStreamKind::ChaCha20,
            stream_key: Zeroizing::new(b"SENTINEL-STREAM-KEY".to_vec()),
            binaries: vec![
                InnerBinary {
                    data: b"SENTINEL-PLAIN-ATTACHMENT".to_vec(),
                    protected: false,
                },
                InnerBinary {
                    data: b"SENTINEL-PROTECTED-ATTACHMENT".to_vec(),
                    protected: true,
                },
            ],
        };
        let debug = format!("{header:?}");
        assert!(debug.contains("stream_key_len: 19"));
        for secret in [
            &header.stream_key[..],
            &header.binaries[0].data,
            &header.binaries[1].data,
        ] {
            assert!(!debug.contains(std::str::from_utf8(secret).unwrap()));
            assert!(!debug.contains(&format!("{secret:?}")));
        }
        for binary in &header.binaries {
            let debug = format!("{binary:?}");
            assert!(debug.contains(&format!("data_len: {}", binary.data.len())));
            assert!(debug.contains(&format!("protected: {}", binary.protected)));
            assert!(!debug.contains(&format!("{:?}", binary.data)));
        }
    }
    #[test]
    fn empty_attachment_payloads_are_valid_with_a_flag() {
        let mut bytes = Vec::new();
        append_wire_field(&mut bytes, 3, &[1]);
        bytes.extend(valid_wire());
        let (header, _) = InnerHeader::parse(&bytes).unwrap();
        assert!(header.binaries[0].data.is_empty());
        assert!(header.binaries[0].protected);
    }
}
