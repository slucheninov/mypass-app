//! Launch at login (stage 5: macOS ServiceManagement login item; stage 6: Windows Run key). There
//! is no LaunchAgent plugin on macOS.

use crate::PlatformResult;

/// Status of the login item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItemStatus {
    /// Registered and enabled.
    Enabled,
    /// Not registered.
    Disabled,
    /// Registered, waiting for the user's approval in System Settings.
    RequiresApproval,
    /// The OS cannot find the app to register.
    NotFound,
    /// Not supported on this platform.
    Unsupported,
}

/// Starts the app at login.
pub trait LaunchAtLogin: Send + Sync {
    /// Current status; unsupported platforms return `Ok(LoginItemStatus::Unsupported)`.
    fn status(&self) -> PlatformResult<LoginItemStatus>;
    /// `true` only for [`LoginItemStatus::Enabled`].
    fn is_enabled(&self) -> PlatformResult<bool> {
        Ok(self.status()? == LoginItemStatus::Enabled)
    }
    /// Turns the login item on or off.
    fn set_enabled(&self, enabled: bool) -> PlatformResult<()>;
}
