//! The native messaging frame codec: a `u32` little-endian length followed by that many bytes of
//! JSON, the framing Chromium uses on the host's stdin and stdout.
//!
//! The stream is hostile input. A claimed length is checked against a cap before anything is
//! buffered or allocated, so a header claiming 4 GiB costs nothing, and malformed or truncated
//! input returns an error and never panics.

use std::io::{self, Read};

/// The largest frame accepted by default. Chromium itself caps a message from the extension at
/// 4 GiB and a message to it at 1 MiB; no legitimate bridge message comes near 10 MiB.
pub const MAX_FRAME_BYTES: usize = 10 * 1024 * 1024;

const HEADER_LEN: usize = 4;

/// Frame `payload` with its four-byte little-endian length. A payload above `u32::MAX` bytes cannot
/// occur by construction (the callers cap frames at [`MAX_FRAME_BYTES`]).
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    debug_assert!(u32::try_from(payload.len()).is_ok());
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(HEADER_LEN.saturating_add(payload.len()));
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("frame too large: claimed {claimed}, max {max}")]
    TooLarge { claimed: usize, max: usize },
}

/// Incremental frame decoder for a byte stream that arrives in arbitrary pieces.
#[derive(Debug)]
pub struct FrameReader {
    buf: Vec<u8>,
    max: usize,
}

impl FrameReader {
    pub fn new(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            max,
        }
    }

    pub fn default_max() -> Self {
        Self::new(MAX_FRAME_BYTES)
    }

    /// Add `data` and return every frame that is now complete. A claimed length above the cap
    /// clears the buffer and returns an error; the stream cannot be resynchronised after that, so
    /// the caller drops the connection.
    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();
        let mut pos = 0usize;
        loop {
            let rest = self.buf.get(pos..).unwrap_or_default();
            let Some(claimed) = peek_len(rest) else { break };
            if claimed > self.max {
                self.buf.clear();
                return Err(FrameError::TooLarge {
                    claimed,
                    max: self.max,
                });
            }
            let Some(end) = HEADER_LEN.checked_add(claimed) else {
                break;
            };
            let Some(payload) = rest.get(HEADER_LEN..end) else {
                break;
            };
            frames.push(payload.to_vec());
            pos = pos.saturating_add(end);
        }
        self.buf.drain(..pos);
        Ok(frames)
    }
}

/// The claimed length of the frame at the start of `bytes`, once its header is complete.
fn peek_len(bytes: &[u8]) -> Option<usize> {
    let header: [u8; HEADER_LEN] = bytes.get(..HEADER_LEN)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(header)).ok()
}

/// Read one frame from a blocking stream. `Ok(None)` is a clean end of stream at a frame boundary;
/// an oversize claim is `InvalidData` (checked before any payload is read or allocated) and an
/// end of stream inside a frame is `UnexpectedEof`.
pub fn read_frame(r: &mut impl Read, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut header = [0u8; HEADER_LEN];
    let mut got = 0usize;
    while got < HEADER_LEN {
        let n = match header.get_mut(got..) {
            Some(slot) => match r.read(slot) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            },
            None => break,
        };
        if n == 0 {
            return if got == 0 {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream ended inside a frame header",
                ))
            };
        }
        got = got.saturating_add(n);
    }
    let claimed = usize::try_from(u32::from_le_bytes(header))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length overflow"))?;
    if claimed > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            FrameError::TooLarge { claimed, max },
        ));
    }
    // `take` + `read_to_end` grows the buffer with the bytes that actually arrive, so a lying
    // header on a short stream never allocates the claimed length.
    let mut payload = Vec::new();
    let want = u64::try_from(claimed).unwrap_or(u64::MAX);
    r.take(want).read_to_end(&mut payload)?;
    if payload.len() != claimed {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "stream ended inside a frame payload",
        ));
    }
    Ok(Some(payload))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test-only
mod tests {
    use super::*;

    #[test]
    fn encode_prepends_four_byte_little_endian_length() {
        assert_eq!(encode_frame(b"hi"), vec![2, 0, 0, 0, b'h', b'i']);
        // 0x01020304 is just above 16 MiB, so the cap is raised; read as big endian the same
        // bytes would claim 0x04030201 (over 67 MB) and be rejected.
        let mut r = FrameReader::new(32 * 1024 * 1024);
        assert_eq!(r.feed(&[0x04, 0x03, 0x02, 0x01]), Ok(vec![]));
        assert_eq!(r.buf.len(), 4);
        let mut capped = FrameReader::new(16 * 1024 * 1024);
        assert!(capped.feed(&[0x04, 0x03, 0x02, 0x01]).is_err());
    }

    #[test]
    fn frame_reader_returns_frame_when_all_bytes_arrive_at_once() {
        let mut r = FrameReader::default_max();
        assert_eq!(r.feed(&encode_frame(b"hello")), Ok(vec![b"hello".to_vec()]));
        assert!(r.buf.is_empty());
    }

    #[test]
    fn frame_reader_accumulates_across_partial_feeds() {
        let mut r = FrameReader::default_max();
        let bytes = encode_frame(b"partial");
        let mut got = Vec::new();
        for b in &bytes {
            got.extend(r.feed(std::slice::from_ref(b)).unwrap());
        }
        assert_eq!(got, vec![b"partial".to_vec()]);
    }

    #[test]
    fn frame_reader_returns_multiple_frames_from_one_feed() {
        let mut r = FrameReader::default_max();
        let mut bytes = encode_frame(b"one");
        bytes.extend(encode_frame(b"two"));
        bytes.extend_from_slice(&encode_frame(b"three")[..3]);
        assert_eq!(r.feed(&bytes), Ok(vec![b"one".to_vec(), b"two".to_vec()]));
        assert_eq!(r.buf.len(), 3);
        assert_eq!(
            r.feed(&encode_frame(b"three")[3..]),
            Ok(vec![b"three".to_vec()])
        );
    }

    #[test]
    fn empty_payload_round_trips() {
        let mut r = FrameReader::default_max();
        assert_eq!(r.feed(&encode_frame(b"")), Ok(vec![Vec::new()]));
        let mut cur = io::Cursor::new(encode_frame(b""));
        assert_eq!(read_frame(&mut cur, 16).unwrap(), Some(Vec::new()));
    }

    #[test]
    fn frame_reader_rejects_claimed_length_above_max_without_buffering() {
        let mut r = FrameReader::new(1024);
        let err = r.feed(&1025u32.to_le_bytes()).unwrap_err();
        assert_eq!(
            err,
            FrameError::TooLarge {
                claimed: 1025,
                max: 1024
            }
        );
        assert!(r.buf.is_empty());
        assert_eq!(err.to_string(), "frame too large: claimed 1025, max 1024");
    }

    #[test]
    fn frame_reader_accepts_frame_exactly_at_max() {
        let mut r = FrameReader::new(1024);
        let payload = vec![7u8; 1024];
        assert_eq!(r.feed(&encode_frame(&payload)), Ok(vec![payload]));
    }

    #[test]
    fn default_max_is_ten_mebibytes() {
        assert_eq!(MAX_FRAME_BYTES, 10 * 1024 * 1024);
        assert_eq!(FrameReader::default_max().max, 10 * 1024 * 1024);
    }

    #[test]
    fn claimed_4_gib_length_is_rejected_immediately() {
        let mut r = FrameReader::default_max();
        let err = r.feed(&[0xFF, 0xFF, 0xFF, 0xFF, b'x']).unwrap_err();
        assert!(
            matches!(err, FrameError::TooLarge { claimed, .. } if claimed == u32::MAX as usize)
        );
        assert!(r.buf.is_empty());
        assert!(r.buf.capacity() < 1024);
        let mut cur = io::Cursor::new(vec![0xFF, 0xFF, 0xFF, 0xFF]);
        let e = read_frame(&mut cur, MAX_FRAME_BYTES).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_frame_clean_eof_is_none_and_mid_frame_eof_is_error() {
        let mut empty = io::Cursor::new(Vec::new());
        assert_eq!(read_frame(&mut empty, 16).unwrap(), None);

        let mut two = io::Cursor::new([encode_frame(b"a"), encode_frame(b"bc")].concat());
        assert_eq!(read_frame(&mut two, 16).unwrap(), Some(b"a".to_vec()));
        assert_eq!(read_frame(&mut two, 16).unwrap(), Some(b"bc".to_vec()));
        assert_eq!(read_frame(&mut two, 16).unwrap(), None);

        for cut in [1usize, 3, 5, 6] {
            let bytes = encode_frame(b"abcdef");
            let mut cur = io::Cursor::new(bytes[..cut].to_vec());
            let e = read_frame(&mut cur, 16).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof, "cut {cut}");
        }
    }

    #[test]
    fn lying_header_on_a_short_stream_is_an_error_not_a_huge_allocation() {
        let mut bytes = (MAX_FRAME_BYTES as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(b"tiny");
        let mut cur = io::Cursor::new(bytes);
        let e = read_frame(&mut cur, MAX_FRAME_BYTES).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }
}
