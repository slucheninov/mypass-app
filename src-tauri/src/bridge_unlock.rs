//! Browser unlock uses the same vault operations and device-unlock policy as the app window.

use std::sync::Arc;

use mypass_bridge::unlock::{DeviceOffer, UnlockFailure, VaultUnlocker};
use mypass_session::app_core::ErrorCode;
use mypass_session::session::SessionState;
use mypass_session::AppCore;
use secrecy::SecretString;

pub(crate) struct AppUnlocker {
    core: Arc<AppCore>,
}

impl AppUnlocker {
    pub(crate) fn new(core: Arc<AppCore>) -> Self {
        Self { core }
    }
}

impl VaultUnlocker for AppUnlocker {
    fn unlock_with_password(&self, password: SecretString) -> Result<(), UnlockFailure> {
        let result = match self.core.session.status().state {
            SessionState::Unlocked => return Ok(()),
            SessionState::Locked => self.core.vault_unlock(password),
            SessionState::NoVault => {
                let path = featured_vault(&self.core).ok_or(UnlockFailure::InApp)?;
                let warnings = self
                    .core
                    .vault_open_warnings(&path)
                    .map_err(|error| map_password_error(error.code))?;
                if !warnings.is_empty() {
                    return Err(UnlockFailure::InApp);
                }
                self.core.vault_open(&path, password)
            }
        };
        result
            .map(|_| ())
            .map_err(|error| map_password_error(error.code))
    }

    fn unlock_with_device(&self, automatic: bool) -> Result<(), UnlockFailure> {
        let reason = crate::i18n::t(self.core.settings.get().language, "Unlock MyPass", &[]);
        let device = &self.core.device_unlock;
        if automatic {
            device.unlock_with_device_automatically(&reason)
        } else {
            device.unlock_with_device(&reason)
        }
        .map_err(|error| map_device_error(error.code))
    }

    fn device_offer(&self) -> DeviceOffer {
        let status = self.core.device_unlock.status();
        DeviceOffer {
            offered: status.can_offer_now,
            touch_id: status.biometrics && status.touch_id_enabled,
        }
    }
}

fn featured_vault(core: &AppCore) -> Option<String> {
    core.vault_recent()
        .ok()?
        .into_iter()
        .find(|vault| vault.exists)
        .map(|vault| vault.path)
}

fn map_password_error(code: ErrorCode) -> UnlockFailure {
    match code {
        ErrorCode::WrongPassword => UnlockFailure::WrongPassword,
        ErrorCode::NoVault => UnlockFailure::InApp,
        _ => UnlockFailure::Failed,
    }
}

fn map_device_error(code: ErrorCode) -> UnlockFailure {
    match code {
        ErrorCode::Cancelled => UnlockFailure::Cancelled,
        ErrorCode::DeviceUnlockUnavailable
        | ErrorCode::InvalidInput
        | ErrorCode::Lockout
        | ErrorCode::Invalidated
        | ErrorCode::NotEnrolledOnDevice => UnlockFailure::DeviceUnavailable,
        ErrorCode::NoVault => UnlockFailure::InApp,
        ErrorCode::AuthenticationFailed => UnlockFailure::NotRecognized,
        _ => UnlockFailure::Failed,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test-only
mod tests {
    use super::*;
    use crate::testing::{open_world, pw, shared_world, world};
    use mypass_bridge::relay::RelayOptions;
    use mypass_session::vault_files::ProcessLookup;
    use std::path::PathBuf;

    #[test]
    fn locked_vault_unlocks_with_the_right_password() {
        let w = shared_world(true);
        w.core.vault_lock().unwrap();
        let unlocker = AppUnlocker::new(w.core.clone());
        assert_eq!(unlocker.unlock_with_password(pw("pw")), Ok(()));
        assert_eq!(w.core.session.status().state, SessionState::Unlocked);
    }

    #[test]
    fn locked_vault_wrong_password_is_wrong_password() {
        let w = shared_world(true);
        w.core.vault_lock().unwrap();
        let unlocker = AppUnlocker::new(w.core.clone());
        assert_eq!(
            unlocker.unlock_with_password(pw("bad")),
            Err(UnlockFailure::WrongPassword)
        );
        assert_eq!(w.core.session.status().state, SessionState::Locked);
    }

    #[test]
    fn no_vault_opens_the_featured_recent_vault() {
        let source = open_world();
        let w = shared_world(false);
        w.core
            .settings
            .remember_vault(source.path.to_str().unwrap());
        w.core
            .settings
            .remember_vault(w.dir.path().join("missing.kdbx").to_str().unwrap());
        let unlocker = AppUnlocker::new(w.core.clone());
        assert_eq!(unlocker.unlock_with_password(pw("pw")), Ok(()));
        assert_eq!(w.core.session.status().state, SessionState::Unlocked);
        assert_eq!(w.core.session.vault_path(), source.path.canonicalize().ok());
    }

    #[test]
    fn no_vault_without_recent_is_in_app() {
        let w = shared_world(false);
        let unlocker = AppUnlocker::new(w.core.clone());
        assert_eq!(
            unlocker.unlock_with_password(pw("pw")),
            Err(UnlockFailure::InApp)
        );
        assert_eq!(w.core.session.status().state, SessionState::NoVault);
    }

    #[test]
    fn no_vault_with_open_warnings_is_in_app_and_opens_nothing() {
        struct OldApp(PathBuf);

        impl ProcessLookup for OldApp {
            fn other_app_executables(&self, _bundle_id: &str) -> Vec<PathBuf> {
                vec![self.0.clone()]
            }
        }

        let source = open_world();
        let mut w = world();
        w.core
            .settings
            .remember_vault(source.path.to_str().unwrap());
        w.core
            .set_process_lookup(Arc::new(OldApp(w.dir.path().join("Old MyPass"))));
        let core = Arc::new(w.core);
        let unlocker = AppUnlocker::new(core.clone());
        assert_eq!(
            unlocker.unlock_with_password(pw("pw")),
            Err(UnlockFailure::InApp)
        );
        assert_eq!(core.session.status().state, SessionState::NoVault);
    }

    #[test]
    fn device_error_codes_map_to_spec_failures() {
        for (code, failure) in [
            (ErrorCode::Cancelled, UnlockFailure::Cancelled),
            (
                ErrorCode::DeviceUnlockUnavailable,
                UnlockFailure::DeviceUnavailable,
            ),
            (ErrorCode::InvalidInput, UnlockFailure::DeviceUnavailable),
            (ErrorCode::Lockout, UnlockFailure::DeviceUnavailable),
            (ErrorCode::Invalidated, UnlockFailure::DeviceUnavailable),
            (
                ErrorCode::NotEnrolledOnDevice,
                UnlockFailure::DeviceUnavailable,
            ),
            (ErrorCode::NoVault, UnlockFailure::InApp),
            // A rejected finger is counted by the popup (three switch it to the password).
            (
                ErrorCode::AuthenticationFailed,
                UnlockFailure::NotRecognized,
            ),
            (ErrorCode::Internal, UnlockFailure::Failed),
        ] {
            assert_eq!(map_device_error(code), failure);
        }
    }

    #[test]
    fn device_offer_comes_from_device_unlock_status() {
        let w = shared_world(true);
        w.core.vault_lock().unwrap();
        let unlocker = AppUnlocker::new(w.core.clone());
        assert_eq!(unlocker.device_offer(), DeviceOffer::default());
    }

    #[test]
    fn a_rejected_finger_is_not_recognized_and_an_automatic_prompt_right_after_is_cancelled() {
        let w = shared_world(true);
        w.core
            .device_unlock
            .configure(Some(pw("pw")), true, false)
            .unwrap();
        w.core.vault_lock().unwrap();
        let unlocker = AppUnlocker::new(w.core.clone());
        w.fake
            .device_unlock
            .fail_next_unlock(mypass_platform::PlatformError::AuthenticationFailed);
        assert_eq!(
            unlocker.unlock_with_device(false),
            Err(UnlockFailure::NotRecognized)
        );
        assert_eq!(w.fake.device_unlock.evaluation_count(), 1);
        // The popup's automatic prompt within the app's debounce is refused without a prompt.
        assert_eq!(
            unlocker.unlock_with_device(true),
            Err(UnlockFailure::Cancelled)
        );
        assert_eq!(w.fake.device_unlock.evaluation_count(), 1);
        // An explicit press still prompts.
        assert_eq!(unlocker.unlock_with_device(false), Ok(()));
        assert_eq!(w.fake.device_unlock.evaluation_count(), 2);
        assert_eq!(w.core.session.status().state, SessionState::Unlocked);
    }

    #[test]
    fn relay_drain_outlasts_the_device_prompt() {
        assert!(RelayOptions::default().drain_timeout > mypass_platform::DEVICE_UNLOCK_TIMEOUT);
    }
}
