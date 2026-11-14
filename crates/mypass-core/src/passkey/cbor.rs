//! Emit-only CBOR (RFC 8949) for the fixed attestation shapes: definite lengths in the shortest
//! form, no floats, tags or indefinite items. Nothing here can panic.

fn head(major: u8, n: u64) -> Vec<u8> {
    let m = major << 5;
    let mut out = Vec::with_capacity(9);
    if n < 24 {
        out.push(m | (n as u8));
    } else if let Ok(v) = u8::try_from(n) {
        out.extend([m | 24, v]);
    } else if let Ok(v) = u16::try_from(n) {
        out.push(m | 25);
        out.extend(v.to_be_bytes());
    } else if let Ok(v) = u32::try_from(n) {
        out.push(m | 26);
        out.extend(v.to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend(n.to_be_bytes());
    }
    out
}

fn length(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Major type 0: an unsigned integer.
pub fn uint(v: u64) -> Vec<u8> {
    head(0, v)
}

/// Major type 1: the negative integer `-1 - magnitude` (so `-7` is `nint(6)`). Taking the
/// encoded magnitude means no subtraction can underflow.
pub fn nint(magnitude: u64) -> Vec<u8> {
    head(1, magnitude)
}

/// Major type 2: a byte string.
pub fn bytes(b: &[u8]) -> Vec<u8> {
    let mut out = head(2, length(b.len()));
    out.extend_from_slice(b);
    out
}

/// Major type 3: a text string.
pub fn text(s: &str) -> Vec<u8> {
    let mut out = head(3, length(s.len()));
    out.extend_from_slice(s.as_bytes());
    out
}

/// Major type 5: the header of a map with `pairs` key/value pairs (the caller appends them).
pub fn map_header(pairs: usize) -> Vec<u8> {
    head(5, length(pairs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciborium::value::Value;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn cbor_matches_rfc8949_appendix_a_vectors() {
        for (v, h) in [
            (0u64, "00"),
            (23, "17"),
            (24, "1818"),
            (100, "1864"),
            (1000, "1903e8"),
            (1_000_000, "1a000f4240"),
            (4_294_967_296, "1b0000000100000000"),
            (u64::MAX, "1bffffffffffffffff"),
        ] {
            assert_eq!(hex(&uint(v)), h, "uint {v}");
        }
        for (v, h) in [
            (-1i64, "20"),
            (-7, "26"),
            (-10, "29"),
            (-100, "3863"),
            (-1000, "3903e7"),
        ] {
            assert_eq!(hex(&nint(v.unsigned_abs() - 1)), h, "nint {v}");
        }
        assert_eq!(hex(&nint(u64::MAX)), "3bffffffffffffffff");
        assert_eq!(hex(&text("")), "60");
        assert_eq!(hex(&text("a")), "6161");
        assert_eq!(hex(&text("IETF")), "6449455446");
        assert_eq!(hex(&text("\u{fc}")), "62c3bc");
        assert_eq!(hex(&bytes(&[1, 2, 3, 4])), "4401020304");
        assert_eq!(hex(&map_header(0)), "a0");
        assert_eq!(hex(&map_header(3)), "a3");
    }

    #[test]
    fn cbor_headers_across_length_boundaries() {
        for n in [0usize, 23, 24, 255, 256, 65_535, 65_536] {
            let enc = bytes(&vec![7u8; n]);
            let v: Value = ciborium::de::from_reader(enc.as_slice()).unwrap();
            assert_eq!(v, Value::Bytes(vec![7u8; n]), "len {n}");
            let t = text(&"x".repeat(n));
            let v: Value = ciborium::de::from_reader(t.as_slice()).unwrap();
            assert_eq!(v, Value::Text("x".repeat(n)), "text len {n}");
        }
    }

    #[test]
    fn cbor_output_decodes_with_ciborium_as_expected_map() {
        let mut enc = map_header(3);
        enc.extend(text("fmt"));
        enc.extend(text("none"));
        enc.extend(text("authData"));
        enc.extend(bytes(&[1, 2, 3]));
        enc.extend(uint(1));
        enc.extend(nint(6));
        let v: Value = ciborium::de::from_reader(enc.as_slice()).unwrap();
        assert_eq!(
            v,
            Value::Map(vec![
                (Value::Text("fmt".into()), Value::Text("none".into())),
                (Value::Text("authData".into()), Value::Bytes(vec![1, 2, 3])),
                (Value::Integer(1.into()), Value::Integer((-7).into())),
            ])
        );
    }
}
