//! Safe Windows desktop policy; native ownership and locking live in windows.rs.
use crate::{ClipboardStamp, PlatformError, PlatformResult, SystemEvent};
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
/// sequence nor empty may release/reacquire the OS lock (including in clear_clipboard).
pub trait ClipboardTransaction {
    fn empty(&mut self) -> PlatformResult<()>;
    fn set(&mut self, format: &str, bytes: &[u8]) -> PlatformResult<()>;
    fn sequence(&self) -> PlatformResult<u64>;
}

pub const EXCLUSIONS: [(&str, &[u8]); 4] = [
    ("ExcludeClipboardContentFromMonitorProcessing", &[0]),
    ("CanIncludeInClipboardHistory", &[0, 0, 0, 0]),
    ("CanUploadToCloudClipboard", &[0, 0, 0, 0]),
    // Legacy viewers and some remote-desktop clipboard synchronization honor this marker.
    ("Clipboard Viewer Ignore", &[]),
];

pub fn write_clipboard(
    tx: &mut dyn ClipboardTransaction,
    text: &str,
) -> PlatformResult<ClipboardStamp> {
    // Validate and stage before destroying the previous clipboard. Only our staging is wiped;
    // a successfully transferred HGLOBAL belongs to Windows and is never modified or freed.
    if text.contains('\0') {
        return Err(PlatformError::Failed(
            "interior NUL in clipboard text".into(),
        ));
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(
        text.len().saturating_mul(2).saturating_add(2),
    ));
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    tx.empty()?;
    let result = (|| {
        for (name, payload) in EXCLUSIONS {
            tx.set(name, payload)?;
        }
        tx.set("CF_UNICODETEXT", &bytes)?;
        tx.sequence().map(ClipboardStamp)
    })();
    if result.is_err() {
        // Still under the same OS lock: no partially published secret survives a failed write.
        tx.empty()?;
    }
    result
}

pub fn clear_clipboard(
    tx: &mut dyn ClipboardTransaction,
    stamp: &ClipboardStamp,
) -> PlatformResult<bool> {
    if tx.sequence()? != stamp.0 {
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
