//! Safe Windows desktop policy; native ownership and locking live in windows.rs.
use crate::{ClipboardStamp, PlatformError, PlatformResult, SystemEvent};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug)]
pub enum Message {
    Session(u32),
    Power(u32),
    Display(u32),
}

pub fn map_message(message: Message) -> Option<SystemEvent> {
    use SystemEvent::*;
    match message {
        // Lock, console/remote disconnect and logoff all close access. Connect/logon never
        // imply an unlock. WTS notifications are scoped to this session by the native adapter.
        Message::Session(7 | 2 | 4 | 6) => Some(ScreenLocked),
        Message::Session(8) => Some(ScreenUnlocked),
        Message::Power(4) | Message::Display(0) => Some(Sleep),
        Message::Power(18) | Message::Display(1 | 2) => Some(Wake),
        // PBT_APMRESUMESUSPEND (7) follows RESUMEAUTOMATIC: do not double-fire.
        _ => None,
    }
}

/// One already-open OS clipboard transaction. Drop of the native guard closes it. Neither
/// reads nor empty may release/reacquire the OS lock (including in clear_clipboard).
pub trait ClipboardTransaction {
    fn empty(&mut self) -> PlatformResult<()>;
    fn set(&mut self, format: &str, bytes: &[u8]) -> PlatformResult<()>;
    fn sequence(&self) -> PlatformResult<u64>;
    fn has_format(&self, format: &str) -> PlatformResult<bool>;
    fn read_unicode_text(&self) -> PlatformResult<Option<Zeroizing<Vec<u8>>>>;
}

pub const MAX_CLIPBOARD_BYTES: usize = 2 * 1024 * 1024;
pub const OUR_MARKER: &str = "ExcludeClipboardContentFromMonitorProcessing";

/// Private per-write authentication state, never exposed through the public stamp or Debug.
pub struct ClipboardWrite {
    pub stamp: ClipboardStamp,
    key: Zeroizing<[u8; 32]>,
    digest: Zeroizing<[u8; 32]>,
}

fn text_mac(key: &[u8], bytes: &[u8]) -> PlatformResult<Hmac<Sha256>> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .map_err(|_| PlatformError::Failed("clipboard authentication key".into()))?;
    mac.update(bytes);
    Ok(mac)
}

/// An HGLOBAL may have allocation padding. Authenticate only through the first UTF-16 NUL,
/// including that terminator, exactly as staged by write_clipboard. Reject malformed/big input.
pub fn bounded_unicode_text(bytes: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if bytes.len() > MAX_CLIPBOARD_BYTES || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let end = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .position(|unit| *unit == [0, 0])?;
    bytes
        .get(..(end + 1) * 2)
        .map(|text| Zeroizing::new(text.to_vec()))
}

pub const EXCLUSIONS: [(&str, &[u8]); 4] = [
    ("ExcludeClipboardContentFromMonitorProcessing", &[0]),
    ("CanIncludeInClipboardHistory", &[0, 0, 0, 0]),
    ("CanUploadToCloudClipboard", &[0, 0, 0, 0]),
    // Legacy viewers and some remote-desktop clipboard synchronization honor this marker.
    ("Clipboard Viewer Ignore", &[0]),
];

pub fn write_clipboard(
    tx: &mut dyn ClipboardTransaction,
    text: &str,
) -> PlatformResult<ClipboardWrite> {
    // Validate and stage before destroying the previous clipboard. Only our staging is wiped;
    // a successfully transferred HGLOBAL belongs to Windows and is never modified or freed.
    if text.contains('\0') {
        return Err(PlatformError::Failed(
            "interior NUL in clipboard text".into(),
        ));
    }
    let length = text
        .encode_utf16()
        .count()
        .saturating_add(1)
        .saturating_mul(2);
    if length > MAX_CLIPBOARD_BYTES {
        return Err(PlatformError::Failed("clipboard text exceeds limit".into()));
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(length));
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let mut key = Zeroizing::new([0u8; 32]);
    getrandom::fill(key.as_mut())
        .map_err(|_| PlatformError::Failed("clipboard randomness unavailable".into()))?;
    let digest = Zeroizing::new(
        text_mac(key.as_ref(), &bytes)?
            .finalize()
            .into_bytes()
            .into(),
    );
    tx.empty()?;
    let result = (|| {
        for (name, payload) in EXCLUSIONS {
            tx.set(name, payload)?;
        }
        tx.set("CF_UNICODETEXT", &bytes)?;
        Ok(ClipboardWrite {
            stamp: ClipboardStamp(tx.sequence()?),
            key,
            digest,
        })
    })();
    if result.is_err() {
        // Still under the same OS lock: no partially published secret survives a failed write.
        tx.empty()?;
    }
    result
}

pub fn clear_clipboard(
    tx: &mut dyn ClipboardTransaction,
    written: &ClipboardWrite,
) -> PlatformResult<bool> {
    // Close-time synthesis may advance the sequence without any foreign copy. The sequence
    // in the public stamp only selects our retained state; it never authorizes deletion.
    if !tx.has_format(OUR_MARKER)? {
        return Ok(false);
    }
    let Some(bytes) = tx.read_unicode_text()? else {
        return Ok(false);
    };
    // RustCrypto verifies the complete HMAC tag in constant time.
    if text_mac(written.key.as_ref(), &bytes)?
        .verify_slice(written.digest.as_ref())
        .is_err()
    {
        return Ok(false);
    }
    tx.empty()?;
    Ok(true)
}

/// Unknown native locale names must never turn into arbitrary UI text (including paths).
pub fn layout_label(lang: u16, iso: Option<&str>) -> Option<String> {
    match lang {
        0x0409 => Some("EN".into()),
        0x0422 => Some("UK".into()),
        0x0419 => Some("RU".into()),
        _ => iso
            .filter(|s| (2..=3).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic()))
            .map(str::to_ascii_uppercase),
    }
}

/// Timer-independent change detector. A fake trigger exercises the same state as WM_TIMER.
pub struct LayoutPoll {
    last: Option<String>,
    active: bool,
}
impl LayoutPoll {
    pub fn new(last: Option<String>) -> Self {
        Self { last, active: true }
    }
    pub fn tick(&mut self, next: Option<String>) -> Option<Option<String>> {
        if !self.active || self.last == next {
            return None;
        }
        self.last.clone_from(&next);
        Some(next)
    }
    pub fn cancel(&mut self) {
        self.active = false;
    }
}

/// A command worker's calling thread is intentionally not an input to this selection.
pub fn layout_thread(
    own_pid: u32,
    foreground_pid: u32,
    foreground_thread: u32,
    captured_ui: u32,
) -> u32 {
    if own_pid == foreground_pid && foreground_thread != 0 {
        foreground_thread
    } else {
        captured_ui
    }
}
