//! The icon of the running app, so the app shell can show the light or the dark variant of its key
//! for the effective theme (macOS: the Dock tile). Only the running app's icon changes: Finder and
//! the bundle keep the bundle icon.

use crate::PlatformResult;

/// The OS appearance and the running app's icon.
pub trait AppIcon: Send + Sync {
    /// Whether the OS appearance is dark, whatever appearance the app forces on its own windows.
    fn system_dark(&self) -> PlatformResult<bool>;
    /// Shows `png` (an encoded PNG) as the running app's icon until the next call or the app
    /// quits. Main thread only on macOS.
    fn set_png(&self, png: &[u8]) -> PlatformResult<()>;
}
