//! KDBX VariantDictionary: the typed key/value container used for KDF parameters.

use super::reader::Reader;
use crate::error::{KdbxError, Result};

/// Version word written by KeePass: major byte 1, minor byte 0. Only the high byte is checked on
/// read, so a later minor revision still parses.
const VERSION: u16 = 0x0100;
const VERSION_MAJOR_MASK: u16 = 0xFF00;

const TYPE_END: u8 = 0x00;
const TYPE_U32: u8 = 0x04;
const TYPE_U64: u8 = 0x05;
const TYPE_BOOL: u8 = 0x08;
const TYPE_I32: u8 = 0x0C;
const TYPE_I64: u8 = 0x0D;
const TYPE_STRING: u8 = 0x18;
const TYPE_BYTES: u8 = 0x42;

/// One typed value of a [`VariantDictionary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantValue {
    U32(u32),
    U64(u64),
    I32(i32),
    I64(i64),
    Bool(bool),
    String(String),
    Bytes(Vec<u8>),
}

/// Ordered key/value list, as stored in the KDF parameters header field. Order is kept so that a
/// dictionary re-serialises the way it was read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VariantDictionary {
    pub items: Vec<(String, VariantValue)>,
}

fn corrupt(what: &str) -> KdbxError {
    KdbxError::Corrupt(format!("variant dictionary: {what}"))
}

fn fixed<const N: usize>(value: &[u8], what: &str) -> Result<[u8; N]> {
    <[u8; N]>::try_from(value).map_err(|_| corrupt(what))
}

fn decode_value(type_byte: u8, value: &[u8]) -> Result<VariantValue> {
    Ok(match type_byte {
        TYPE_U32 => VariantValue::U32(u32::from_le_bytes(fixed(value, "u32 length")?)),
        TYPE_U64 => VariantValue::U64(u64::from_le_bytes(fixed(value, "u64 length")?)),
        TYPE_I32 => VariantValue::I32(i32::from_le_bytes(fixed(value, "i32 length")?)),
        TYPE_I64 => VariantValue::I64(i64::from_le_bytes(fixed(value, "i64 length")?)),
        TYPE_BOOL => {
            let [b] = fixed::<1>(value, "bool length")?;
            VariantValue::Bool(b != 0)
        }
        TYPE_STRING => VariantValue::String(
            std::str::from_utf8(value)
                .map_err(|_| corrupt("string is not valid UTF-8"))?
                .to_owned(),
        ),
        TYPE_BYTES => VariantValue::Bytes(value.to_vec()),
        _ => return Err(corrupt("unknown value type")),
    })
}

impl VariantDictionary {
    /// Parses a serialised dictionary. Bytes after the terminator are ignored, as KeePass does.
    pub(crate) fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        if r.u16_le()? & VERSION_MAJOR_MASK != VERSION {
            return Err(corrupt("unsupported version"));
        }
        let mut items = Vec::new();
        loop {
            let type_byte = r.u8()?;
            if type_byte == TYPE_END {
                return Ok(Self { items });
            }
            let key_len = len_to_usize(r.u32_le()?)?;
            let key = std::str::from_utf8(r.bytes(key_len)?)
                .map_err(|_| corrupt("key is not valid UTF-8"))?
                .to_owned();
            let value_len = len_to_usize(r.u32_le()?)?;
            let value = decode_value(type_byte, r.bytes(value_len)?)?;
            items.push((key, value));
        }
    }

    /// Serialises the dictionary in insertion order.
    ///
    /// Lengths are `u32` on the wire; a single key or value above 4 GiB cannot be represented and
    /// is outside anything this engine produces (its inputs are KDF parameters of a few hundred
    /// bytes), so such a length saturates instead of panicking.
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&VERSION.to_le_bytes());
        for (key, value) in &self.items {
            let (type_byte, encoded): (u8, Vec<u8>) = match value {
                VariantValue::U32(v) => (TYPE_U32, v.to_le_bytes().to_vec()),
                VariantValue::U64(v) => (TYPE_U64, v.to_le_bytes().to_vec()),
                VariantValue::I32(v) => (TYPE_I32, v.to_le_bytes().to_vec()),
                VariantValue::I64(v) => (TYPE_I64, v.to_le_bytes().to_vec()),
                VariantValue::Bool(v) => (TYPE_BOOL, vec![u8::from(*v)]),
                VariantValue::String(v) => (TYPE_STRING, v.as_bytes().to_vec()),
                VariantValue::Bytes(v) => (TYPE_BYTES, v.clone()),
            };
            out.push(type_byte);
            push_len(&mut out, key.len());
            out.extend_from_slice(key.as_bytes());
            push_len(&mut out, encoded.len());
            out.extend_from_slice(&encoded);
        }
        out.push(TYPE_END);
        out
    }

    /// First value stored under `key`.
    pub fn get(&self, key: &str) -> Option<&VariantValue> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Replaces the value under `key` in place, or appends a new entry.
    pub fn set(&mut self, key: &str, v: VariantValue) {
        match self.items.iter_mut().find(|(k, _)| k == key) {
            Some((_, slot)) => *slot = v,
            None => self.items.push((key.to_owned(), v)),
        }
    }
}

fn len_to_usize(len: u32) -> Result<usize> {
    usize::try_from(len).map_err(|_| corrupt("length does not fit in memory"))
}

fn push_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::KdbxError;

    #[test]
    fn permissive_parse_policy_is_stable() {
        let dict = VariantDictionary {
            items: vec![
                ("x".into(), VariantValue::U32(1)),
                ("x".into(), VariantValue::U32(2)),
            ],
        };
        let mut bytes = dict.serialize();
        bytes[0] = 0xff;
        bytes.extend_from_slice(b"trailing");
        let parsed = VariantDictionary::parse(&bytes).unwrap();
        assert_eq!(parsed, dict);
        assert_eq!(parsed.get("x"), Some(&VariantValue::U32(1)));
    }
    fn sample() -> VariantDictionary {
        let mut d = VariantDictionary::default();
        d.set("u32", VariantValue::U32(7));
        d.set("u64", VariantValue::U64(1 << 40));
        d.set("i32", VariantValue::I32(-5));
        d.set("i64", VariantValue::I64(-(1 << 40)));
        d.set("bool", VariantValue::Bool(true));
        d.set(
            "str",
            VariantValue::String("\u{43f}\u{440}\u{438}\u{432}\u{456}\u{442}".into()),
        );
        d.set("bytes", VariantValue::Bytes(vec![0, 1, 255]));
        d
    }

    fn entry(type_byte: u8, name: &str, value: &[u8]) -> Vec<u8> {
        let mut v = vec![type_byte];
        v.extend_from_slice(&(name.len() as u32).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.extend_from_slice(&(value.len() as u32).to_le_bytes());
        v.extend_from_slice(value);
        v
    }

    #[test]
    fn all_value_types_round_trip() {
        let d = sample();
        let parsed = VariantDictionary::parse(&d.serialize()).unwrap();
        assert_eq!(parsed, d);
        assert_eq!(parsed.get("u64"), Some(&VariantValue::U64(1 << 40)));
        assert_eq!(parsed.get("i64"), Some(&VariantValue::I64(-(1 << 40))));
        assert_eq!(parsed.get("bool"), Some(&VariantValue::Bool(true)));
        assert_eq!(parsed.get("missing"), None);
    }

    #[test]
    fn encoding_matches_the_wire_format() {
        let mut d = VariantDictionary::default();
        d.set("S", VariantValue::U32(1));
        let mut expected = vec![0x00, 0x01];
        expected.extend(entry(0x04, "S", &[1, 0, 0, 0]));
        expected.push(0x00);
        assert_eq!(d.serialize(), expected);
    }

    #[test]
    fn encode_decode_empty_dictionary() {
        let d = VariantDictionary::default();
        let bytes = d.serialize();
        assert_eq!(bytes, vec![0x00, 0x01, 0x00]);
        assert_eq!(VariantDictionary::parse(&bytes).unwrap(), d);
    }

    #[test]
    fn insertion_order_is_preserved_and_set_replaces_in_place() {
        let mut d = VariantDictionary::default();
        d.set("b", VariantValue::U32(1));
        d.set("a", VariantValue::U32(2));
        d.set("b", VariantValue::U32(3));
        let keys: Vec<&str> = d.items.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["b", "a"]);
        assert_eq!(d.get("b"), Some(&VariantValue::U32(3)));
        let parsed = VariantDictionary::parse(&d.serialize()).unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn truncated_data_is_corrupt() {
        let bytes = sample().serialize();
        for len in 0..bytes.len() {
            let result = VariantDictionary::parse(&bytes[..len]);
            assert!(
                matches!(result, Err(KdbxError::Corrupt(_))),
                "prefix of {len} bytes gave {result:?}"
            );
        }
    }

    #[test]
    fn unknown_type_byte_is_corrupt() {
        let mut bytes = vec![0x00, 0x01];
        bytes.extend(entry(0x99, "x", &[1]));
        bytes.push(0x00);
        assert!(matches!(
            VariantDictionary::parse(&bytes),
            Err(KdbxError::Corrupt(_))
        ));
    }

    #[test]
    fn bad_version_is_corrupt() {
        // 0x0200 little-endian: high byte 2.
        assert!(matches!(
            VariantDictionary::parse(&[0x00, 0x02, 0x00]),
            Err(KdbxError::Corrupt(_))
        ));
    }

    #[test]
    fn declared_length_past_end_is_corrupt() {
        // Key length 0xFFFF_FFFF.
        let mut bytes = vec![0x00, 0x01, 0x04];
        bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert!(matches!(
            VariantDictionary::parse(&bytes),
            Err(KdbxError::Corrupt(_))
        ));
        // Value length 0xFFFF_FFFF after a valid key.
        let mut bytes = vec![0x00, 0x01, 0x04, 1, 0, 0, 0, b'k'];
        bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert!(matches!(
            VariantDictionary::parse(&bytes),
            Err(KdbxError::Corrupt(_))
        ));
    }

    #[test]
    fn wrong_fixed_width_value_length_is_corrupt() {
        for (type_byte, len) in [(0x04u8, 3usize), (0x05, 4), (0x08, 2), (0x0C, 5), (0x0D, 4)] {
            let mut bytes = vec![0x00, 0x01];
            bytes.extend(entry(type_byte, "k", &vec![0u8; len]));
            bytes.push(0x00);
            assert!(
                matches!(VariantDictionary::parse(&bytes), Err(KdbxError::Corrupt(_))),
                "type {type_byte:#x} length {len}"
            );
        }
    }

    #[test]
    fn invalid_utf8_in_key_or_string_is_corrupt() {
        let mut bytes = vec![0x00, 0x01];
        bytes.extend(entry(0x18, "k", &[0xFF, 0xFE]));
        bytes.push(0x00);
        assert!(matches!(
            VariantDictionary::parse(&bytes),
            Err(KdbxError::Corrupt(_))
        ));
        let mut bytes = vec![
            0x00, 0x01, 0x04, 2, 0, 0, 0, 0xFF, 0xFE, 4, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        bytes.push(0x00);
        assert!(matches!(
            VariantDictionary::parse(&bytes),
            Err(KdbxError::Corrupt(_))
        ));
    }

    #[test]
    fn bool_is_one_byte_and_zero_is_false() {
        let mut bytes = vec![0x00, 0x01];
        bytes.extend(entry(0x08, "f", &[0]));
        bytes.extend(entry(0x08, "t", &[1]));
        bytes.push(0x00);
        let d = VariantDictionary::parse(&bytes).unwrap();
        assert_eq!(d.get("f"), Some(&VariantValue::Bool(false)));
        assert_eq!(d.get("t"), Some(&VariantValue::Bool(true)));
    }
}
