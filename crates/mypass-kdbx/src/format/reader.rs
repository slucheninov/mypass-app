//! Bounds-checked little-endian byte reader.

use crate::error::{KdbxError, Result};

/// A cursor over a byte slice. Every read is bounds-checked and returns
/// `Corrupt("truncated ...")` instead of indexing past the end; a failed read does not advance.
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Offset of the next unread byte.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Number of unread bytes.
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn u8(&mut self) -> Result<u8> {
        let [b] = self.array::<1>()?;
        Ok(b)
    }

    pub fn u16_le(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64_le(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// Borrows the next `n` bytes. `n` comes from untrusted length fields, so it is compared with
    /// `remaining()` before anything is sliced or allocated.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(KdbxError::Corrupt(format!(
                "truncated data: wanted {n} bytes, {} left",
                self.remaining()
            )));
        }
        let end = self.pos.saturating_add(n);
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| KdbxError::Corrupt("truncated data".into()))?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let slice = self.bytes(N)?;
        <[u8; N]>::try_from(slice).map_err(|_| KdbxError::Corrupt("truncated data".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::KdbxError;

    #[test]
    fn reads_little_endian_integers_and_tracks_position() {
        let data = [
            0x01, 0x02, 0x01, 0x04, 0x03, 0x02, 0x01, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02,
            0x01,
        ];
        let mut r = Reader::new(&data);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.u16_le().unwrap(), 0x0102);
        assert_eq!(r.u32_le().unwrap(), 0x0102_0304);
        assert_eq!(r.pos(), 7);
        assert_eq!(r.remaining(), 8);
        assert_eq!(r.u64_le().unwrap(), 0x0102_0304_0506_0708);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn bytes_returns_a_borrowed_slice() {
        let data = [1u8, 2, 3, 4];
        let mut r = Reader::new(&data);
        assert_eq!(r.bytes(3).unwrap(), &[1, 2, 3]);
        assert_eq!(r.bytes(0).unwrap(), &[] as &[u8]);
        assert_eq!(r.remaining(), 1);
    }

    #[test]
    fn every_read_past_the_end_is_truncated_and_does_not_advance() {
        let mut r = Reader::new(&[1, 2, 3]);
        for err in [
            r.u32_le().map(|_| ()),
            r.u64_le().map(|_| ()),
            r.bytes(4).map(|_| ()),
            r.bytes(usize::MAX).map(|_| ()),
        ] {
            assert!(matches!(err, Err(KdbxError::Corrupt(m)) if m.starts_with("truncated")));
        }
        assert_eq!(r.pos(), 0);
        let mut empty = Reader::new(&[]);
        assert!(matches!(empty.u8(), Err(KdbxError::Corrupt(_))));
        assert!(matches!(empty.u16_le(), Err(KdbxError::Corrupt(_))));
    }
}
