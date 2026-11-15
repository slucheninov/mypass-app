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
    /// Repair an owned enabled startup entry under the saved setting, preserving OS approval.
    fn prepare_startup(&self, _desired: bool) -> PlatformResult<()> {
        Ok(())
    }
    /// Current status; unsupported platforms return `Ok(LoginItemStatus::Unsupported)`.
    fn status(&self) -> PlatformResult<LoginItemStatus>;
    /// `true` only for [`LoginItemStatus::Enabled`].
    fn is_enabled(&self) -> PlatformResult<bool> {
        Ok(self.status()? == LoginItemStatus::Enabled)
    }
    /// Turns the login item on or off.
    fn set_enabled(&self, enabled: bool) -> PlatformResult<()>;
}

/// Maps a raw `SMAppServiceStatus` value (ServiceManagement, macOS 13+) onto [`LoginItemStatus`]:
/// `0` NotRegistered, `1` Enabled, `2` RequiresApproval, `3` NotFound. A value a later macOS adds
/// counts as [`LoginItemStatus::Disabled`], so the setting never claims a login item it cannot
/// confirm. Pure, so the table is tested on every platform.
pub fn map_sm_status(raw: i64) -> LoginItemStatus {
    match raw {
        1 => LoginItemStatus::Enabled,
        2 => LoginItemStatus::RequiresApproval,
        3 => LoginItemStatus::NotFound,
        _ => LoginItemStatus::Disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sm_status_mapping_table() {
        assert_eq!(map_sm_status(0), LoginItemStatus::Disabled);
        assert_eq!(map_sm_status(1), LoginItemStatus::Enabled);
        assert_eq!(map_sm_status(2), LoginItemStatus::RequiresApproval);
        assert_eq!(map_sm_status(3), LoginItemStatus::NotFound);
        assert_eq!(map_sm_status(4), LoginItemStatus::Disabled);
        assert_eq!(map_sm_status(-1), LoginItemStatus::Disabled);
    }
}
