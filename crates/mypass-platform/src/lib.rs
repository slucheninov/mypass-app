//! OS glue behind traits: system events, the secure clipboard, secret storage, device unlock, the
//! native host registrar, launch at login, the keyboard layout and every on-disk path.
//!
//! [`current`] wires the real implementations of the running OS; [`testing`] (behind the
//! `test-util` feature) holds the fakes every other crate's tests build on.

pub mod dev_overrides;
#[cfg(windows)]
pub mod windows;
pub mod windows_registry;

#[cfg(any(windows, test))]
mod dpapi;
#[cfg(windows)]
pub use dpapi::{DpapiSecretStore, SecretClass};
mod clipboard;
mod coordinated;
mod device_unlock;
pub mod endpoint_store;
mod error;
mod events;
mod fallback;
mod file_store;
mod host_registrar;
mod keyboard_layout;
// The portable core of the macOS device unlock (tested on every platform).
#[cfg(any(target_os = "macos", test))]
mod keychain_device_unlock;
mod launch_at_login;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_device_unlock;
#[cfg(unix)]
mod macos_host_install;
#[cfg(target_os = "macos")]
mod macos_icloud;
#[cfg(target_os = "macos")]
mod macos_keychain;
#[cfg(target_os = "macos")]
mod macos_keychain_acl;
#[cfg(target_os = "macos")]
mod macos_login_item;
mod paths;
mod secrets;

#[cfg(any(test, feature = "test-util"))]
#[doc(hidden)]
pub mod testing;

pub use clipboard::{ClipboardStamp, SecureClipboard};
pub use coordinated::{
    coordinated_files, map_download_status, Accessor, ChangeCallback, ConflictVersion,
    CoordinatedFiles, DownloadState, UnsupportedCoordinatedFiles,
};
pub use device_unlock::{DeviceUnlock, DeviceUnlockAvailability, DeviceUnlockMode};
pub use endpoint_store::{
    app_shared_store, host_shared_store, select_shared_store, SharedStoreKind, SignatureProbe,
    StoreSide, APP_REQUIREMENT, HOST_IDENTIFIER, HOST_REQUIREMENT, TEAM_ID,
};
pub use error::{PlatformError, PlatformResult};
pub use events::{Subscription, SystemEvent, SystemEvents};
pub use file_store::{shared_store, FileSecretStore};
pub use host_registrar::{
    locate_bundled_host, manifest_json, BrowserRegistration, HostRegistrationReport, ManifestError,
    NativeHostRegistrar, RegistrationState, BUNDLED_HOST_FILE, EXTENSION_ID, HOST_NAME,
};
pub use keyboard_layout::KeyboardLayout;
pub use launch_at_login::{map_sm_status, LaunchAtLogin, LoginItemStatus};
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
    #[cfg(windows)]
    return windows::platform(paths);
    #[cfg(not(any(target_os = "macos", windows)))]
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

#[cfg(all(test, unix))]
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

#[cfg(target_os = "macos")]
mod macos_peer_process;
pub mod peer_process;

/// Last-resort native error dialog, before a runtime exists. Call only on the main thread.
pub fn show_startup_error(title: &str, message: &str) {
    #[cfg(target_os = "macos")]
    macos::show_startup_error(title, message);
    #[cfg(not(target_os = "macos"))]
    let _ = (title, message);
}
