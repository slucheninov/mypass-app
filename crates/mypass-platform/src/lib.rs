//! OS glue behind traits: system events, the secure clipboard, secret storage, device unlock, the
//! native host registrar, launch at login, the keyboard layout and every on-disk path.
//!
//! [`current`] wires the real implementations of the running OS; [`testing`] (behind the
//! `test-util` feature) holds the fakes every other crate's tests build on.

mod clipboard;
mod device_unlock;
mod error;
mod events;
mod fallback;
mod file_store;
mod host_registrar;
mod keyboard_layout;
mod launch_at_login;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod macos_host_install;
#[cfg(target_os = "macos")]
mod macos_keychain;
mod paths;
mod secrets;

#[cfg(any(test, feature = "test-util"))]
#[doc(hidden)]
pub mod testing;

pub use clipboard::{ClipboardStamp, SecureClipboard};
pub use device_unlock::{DeviceUnlock, DeviceUnlockAvailability, DeviceUnlockMode};
pub use error::{PlatformError, PlatformResult};
pub use events::{Subscription, SystemEvent, SystemEvents};
pub use file_store::{shared_store, FileSecretStore};
pub use host_registrar::{
    locate_bundled_host, manifest_json, BrowserRegistration, HostRegistrationReport, ManifestError,
    NativeHostRegistrar, RegistrationState, BUNDLED_HOST_FILE, EXTENSION_ID, HOST_NAME,
};
pub use keyboard_layout::KeyboardLayout;
pub use launch_at_login::{LaunchAtLogin, LoginItemStatus};
#[cfg(unix)]
pub use macos_host_install::MacHostRegistrar;
pub use paths::{AppPaths, Browser, VaultId, BUNDLE_ID, HOST_MANIFEST_FILE};
pub use secrets::SecretStore;

use std::path::PathBuf;
use std::sync::Arc;

/// Every platform service, as trait objects. This is the final shape: later stages fill bodies and
/// never add or rename a field.
#[derive(Clone)]
pub struct Platform {
    pub events: Arc<dyn SystemEvents>,
    pub clipboard: Arc<dyn SecureClipboard>,
    pub secrets: Arc<dyn SecretStore>,
    pub shared_secrets: Arc<dyn SecretStore>,
    pub device_unlock: Arc<dyn DeviceUnlock>,
    pub host_registrar: Arc<dyn NativeHostRegistrar>,
    pub launch_at_login: Arc<dyn LaunchAtLogin>,
    pub keyboard_layout: Arc<dyn KeyboardLayout>,
}

impl std::fmt::Debug for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Platform").finish_non_exhaustive()
    }
}

/// The platform services of the running OS; the host registrar is built over `paths`.
pub fn current(paths: &AppPaths) -> Platform {
    #[cfg(target_os = "macos")]
    return macos::platform(paths);
    #[cfg(not(target_os = "macos"))]
    return fallback::platform(paths);
}

/// Executable paths of every running process with `bundle_id` (macOS); elsewhere empty. Not a
/// `Platform` field: only the old-app guard (D7) uses it.
pub fn running_app_executables(bundle_id: &str) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    return macos::running_app_executables(bundle_id);
    #[cfg(not(target_os = "macos"))]
    return fallback::running_app_executables(bundle_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests the selection logic on its own: `current` would touch the real pasteboard on macOS.
    #[cfg(unix)]
    #[test]
    fn shared_store_is_the_file_store_under_secrets_shared() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::macos(tmp.path());
        let store = file_store::shared_store(&paths);
        store.set("svc", "acct", b"endpoint").unwrap();
        let direct = FileSecretStore::new(paths.secrets_shared.clone(), tmp.path().to_path_buf());
        assert_eq!(
            direct.get("svc", "acct").unwrap(),
            Some(b"endpoint".to_vec())
        );
        assert!(paths.secrets_shared.is_dir());
    }
}
