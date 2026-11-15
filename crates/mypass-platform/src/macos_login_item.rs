//! macOS launch at login through ServiceManagement's `SMAppService.mainAppService` (macOS 13+).
//!
//! Why `SMAppService` and not a LaunchAgent plist: the old app used it; the login item shows up in
//! System Settings > General > Login Items under the app's name and signature; it survives the app
//! being moved inside `/Applications`; and it reports `requiresApproval`, which the General pane
//! turns into the "Approval required" message. No LaunchAgent plist is ever written, so there is
//! nothing legacy to clean up.
//!
//! On macOS 12 and earlier the class does not exist: the class is looked up by name first and
//! [`MacLaunchAtLogin`] then reports [`LoginItemStatus::Unsupported`] instead of sending a message
//! to a missing class (which would abort). Real registration needs the signed app bundle, so it is
//! covered by the owner's manual check, never by an automated test.

// `unsafe` is denied crate-wide and allowed in this module only: every ServiceManagement binding
// is an `unsafe` method. Each block is minimal and carries a `// SAFETY:` note; the guard
// `unsafe_only_in_allow_listed_modules` lists this file.
#![allow(unsafe_code)]

use crate::{map_sm_status, LaunchAtLogin, LoginItemStatus, PlatformError, PlatformResult};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyClass;
use objc2_service_management::SMAppService;

/// [`LaunchAtLogin`] over `SMAppService.mainAppService`.
#[derive(Debug, Default)]
pub(crate) struct MacLaunchAtLogin;

/// The main-app service object, or `None` before macOS 13 (no `SMAppService` class).
fn main_app_service() -> Option<Retained<SMAppService>> {
    AnyClass::get(c"SMAppService")?;
    // SAFETY: the class exists (checked above, so the message is never sent to a missing class);
    // `mainAppService` takes no arguments and returns the shared, retained service object.
    Some(unsafe { SMAppService::mainAppService() })
}

/// The status of `service`, mapped.
fn status_of(service: &SMAppService) -> LoginItemStatus {
    // SAFETY: `service` is a live `SMAppService`; `status` takes no arguments and only reads.
    let raw = unsafe { service.status() };
    map_sm_status(i64::try_from(raw.0).unwrap_or(-1))
}

/// A failed register / unregister call: the system's own description, shown verbatim by the
/// General pane ("Could not update login item: <system text>").
fn system_error(error: &objc2_foundation::NSError) -> PlatformError {
    PlatformError::Failed(error.localizedDescription().to_string())
}

impl LaunchAtLogin for MacLaunchAtLogin {
    fn status(&self) -> PlatformResult<LoginItemStatus> {
        autoreleasepool(|_| {
            Ok(main_app_service().map_or(LoginItemStatus::Unsupported, |s| status_of(&s)))
        })
    }

    /// Registers or unregisters the main app. Already in the asked state is a no-op; the caller
    /// re-reads [`LaunchAtLogin::status`] afterwards (registration may end in `RequiresApproval`).
    fn set_enabled(&self, enabled: bool) -> PlatformResult<()> {
        autoreleasepool(|_| {
            let service =
                main_app_service().ok_or(PlatformError::Unsupported("launch at login"))?;
            match (enabled, status_of(&service)) {
                (true, LoginItemStatus::Enabled) => Ok(()),
                (false, LoginItemStatus::Disabled | LoginItemStatus::NotFound) => Ok(()),
                (true, _) => {
                    // SAFETY: `service` is a live `SMAppService`; the call takes no arguments and
                    // reports failure through the returned `NSError`.
                    unsafe { service.registerAndReturnError() }.map_err(|e| system_error(&e))
                }
                (false, _) => {
                    // SAFETY: as above, for unregistration.
                    unsafe { service.unregisterAndReturnError() }.map_err(|e| system_error(&e))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_service_management::SMAppServiceStatus;

    #[test]
    fn sdk_status_values_match_the_mapping_table() {
        let raw = |s: SMAppServiceStatus| i64::try_from(s.0).unwrap();
        assert_eq!(
            map_sm_status(raw(SMAppServiceStatus::NotRegistered)),
            LoginItemStatus::Disabled
        );
        assert_eq!(
            map_sm_status(raw(SMAppServiceStatus::Enabled)),
            LoginItemStatus::Enabled
        );
        assert_eq!(
            map_sm_status(raw(SMAppServiceStatus::RequiresApproval)),
            LoginItemStatus::RequiresApproval
        );
        assert_eq!(
            map_sm_status(raw(SMAppServiceStatus::NotFound)),
            LoginItemStatus::NotFound
        );
    }

    #[test]
    fn mac_launch_at_login_debug_is_plain() {
        assert_eq!(format!("{MacLaunchAtLogin:?}"), "MacLaunchAtLogin");
    }

    /// Reads the real status without registering anything. Ignored: no automated test touches
    /// `SMAppService` (an unsigned test binary is not an app bundle, so it reports `NotFound` or
    /// `NotRegistered`). Run with `cargo test -p mypass-platform -- --ignored login_item_`.
    #[test]
    #[ignore = "reads the real SMAppService status; manual check M10 covers registration"]
    fn login_item_status_reads_without_registering() {
        let status = MacLaunchAtLogin.status().unwrap();
        println!("SMAppService.mainAppService status of the test binary: {status:?}");
        assert_ne!(status, LoginItemStatus::Enabled);
    }
}
