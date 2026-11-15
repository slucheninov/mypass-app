//! The clipboard for secrets: concealed writes and a clear that never wipes a foreign copy.

use crate::PlatformResult;

/// Identifies one write: the macOS changeCount, Windows clipboard sequence, or a fallback
/// write counter checked together with a content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardStamp(pub u64);

/// A clipboard that writes secrets and clears them later only if nobody copied anything since.
pub trait SecureClipboard: Send + Sync {
    /// Writes `text` (a secret: never logged), concealed / excluded from clipboard history where
    /// the OS supports it.
    fn write_secret(&self, text: &str) -> PlatformResult<ClipboardStamp>;
    /// Clears the clipboard if it still holds the write identified by `stamp`. `Ok(true)` means
    /// cleared, `Ok(false)` means the user copied something else since, which is left alone.
    ///
    /// Windows compares and clears under one open OS clipboard lock. macOS and fallback
    /// APIs retain a small accepted check/clear race; their implementations document it.
    fn clear_if_unchanged(&self, stamp: &ClipboardStamp) -> PlatformResult<bool>;
}
