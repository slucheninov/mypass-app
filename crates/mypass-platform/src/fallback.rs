//! Fallbacks: `Unsupported` stubs for every trait and, off macOS, the `arboard` clipboard.

//!
//! Every stub is a one-line `Unsupported` body, so a later stage replaces a body, never a
//! signature. macOS uses the secret store, device unlock, host registrar and launch-at-login stubs
//! until stages 3 and 5; Windows uses all of them until stage 6.

use crate::{
    BrowserRegistration, DeviceUnlock, DeviceUnlockAvailability, DeviceUnlockMode,
    HostRegistrationReport, LaunchAtLogin, LoginItemStatus, NativeHostRegistrar, PlatformError,
    PlatformResult, SecretStore,
};
use secrecy::SecretString;
use std::path::Path;
// Items used only off macOS (and by this module's tests on macOS).
#[cfg(not(target_os = "macos"))]
use crate::{AppPaths, Platform};
#[cfg(any(not(target_os = "macos"), test))]
use crate::{KeyboardLayout, Subscription, SystemEvent, SystemEvents};
#[cfg(any(not(target_os = "macos"), test))]
use std::{path::PathBuf, sync::Arc};

/// No system events on this platform.
#[derive(Debug, Default)]
#[cfg(any(not(target_os = "macos"), test))]
pub(crate) struct UnsupportedSystemEvents;

#[cfg(any(not(target_os = "macos"), test))]
impl SystemEvents for UnsupportedSystemEvents {
    fn subscribe(&self, _: Arc<dyn Fn(SystemEvent) + Send + Sync>) -> PlatformResult<Subscription> {
        Err(PlatformError::Unsupported("system events"))
    }
}

/// No secret store on this platform yet (macOS stage 3, Windows stage 6).
#[derive(Debug, Default)]
pub(crate) struct UnsupportedSecretStore;

impl SecretStore for UnsupportedSecretStore {
    fn set(&self, _: &str, _: &str, _: &[u8]) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("secret store"))
    }
    fn get(&self, _: &str, _: &str) -> PlatformResult<Option<Vec<u8>>> {
        Err(PlatformError::Unsupported("secret store"))
    }
    fn delete(&self, _: &str, _: &str) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("secret store"))
    }
}

/// No device unlock on this platform yet (macOS stage 5).
#[derive(Debug, Default)]
pub(crate) struct UnsupportedDeviceUnlock;

impl DeviceUnlock for UnsupportedDeviceUnlock {
    fn availability(&self) -> DeviceUnlockAvailability {
        DeviceUnlockAvailability::Unavailable
    }
    fn is_enrolled(&self, _: DeviceUnlockMode, _: &str) -> PlatformResult<bool> {
        Err(PlatformError::Unsupported("device unlock"))
    }
    fn enroll(&self, _: DeviceUnlockMode, _: &str, _: &SecretString) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("device unlock"))
    }
    fn unlock(&self, _: DeviceUnlockMode, _: &str, _: &str) -> PlatformResult<SecretString> {
        Err(PlatformError::Unsupported("device unlock"))
    }
    fn forget(&self, _: &str) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("device unlock"))
    }
}

/// No native host registration on this platform yet (macOS stage 3, Windows stage 6).
#[derive(Debug, Default)]
pub(crate) struct UnsupportedHostRegistrar;

impl NativeHostRegistrar for UnsupportedHostRegistrar {
    fn register(&self, _: &Path) -> PlatformResult<HostRegistrationReport> {
        Err(PlatformError::Unsupported("native host registration"))
    }
    fn status(&self) -> Vec<BrowserRegistration> {
        Vec::new()
    }
    fn unregister(&self) -> PlatformResult<HostRegistrationReport> {
        Err(PlatformError::Unsupported("native host registration"))
    }
}

/// No launch at login on this platform yet (macOS stage 5, Windows stage 6).
#[derive(Debug, Default)]
pub(crate) struct UnsupportedLaunchAtLogin;

impl LaunchAtLogin for UnsupportedLaunchAtLogin {
    fn status(&self) -> PlatformResult<LoginItemStatus> {
        Ok(LoginItemStatus::Unsupported)
    }
    fn set_enabled(&self, _: bool) -> PlatformResult<()> {
        Err(PlatformError::Unsupported("launch at login"))
    }
}

/// No keyboard layout reading on this platform.
#[derive(Debug, Default)]
#[cfg(any(not(target_os = "macos"), test))]
pub(crate) struct UnsupportedKeyboardLayout;

#[cfg(any(not(target_os = "macos"), test))]
impl KeyboardLayout for UnsupportedKeyboardLayout {
    fn current(&self) -> PlatformResult<Option<String>> {
        Err(PlatformError::Unsupported("keyboard layout"))
    }
    fn subscribe(
        &self,
        _: Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> PlatformResult<Subscription> {
        Err(PlatformError::Unsupported("keyboard layout"))
    }
}

/// The pure clipboard core: compiled off macOS (where `arboard` drives it) and for tests.
#[cfg(any(not(target_os = "macos"), test))]
mod hashing {
    use crate::{ClipboardStamp, PlatformError, PlatformResult, SecureClipboard};
    use sha2::{Digest, Sha256};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// The plain text operations of a system clipboard: the seam that keeps [`HashingClipboard`]'s
    /// logic testable without touching the developer's real clipboard.
    pub(crate) trait ClipboardBackend: Send {
        fn set_text(&mut self, text: &str) -> Result<(), String>;
        /// `Ok(None)` when the clipboard holds no text.
        fn get_text(&mut self) -> Result<Option<String>, String>;
        fn clear(&mut self) -> Result<(), String>;
    }

    struct HashingState<B> {
        backend: B,
        /// Monotonic count of our writes: the stamp.
        writes: u64,
        /// The stamp and SHA-256 of our last write while it may still be on the clipboard. Only the
        /// hash is kept, never the secret.
        last: Option<(u64, [u8; 32])>,
    }

    /// [`SecureClipboard`] for clipboards without a change counter: a monotonically increasing write
    /// counter plus a SHA-256 of the last written text. `clear_if_unchanged` reads the clipboard back
    /// and clears it only when the stamp is our latest write and the text still hashes the same.
    ///
    /// The Windows clipboard-history and cloud-clipboard exclusion formats are set by the `arboard`
    /// backend (`ArboardBackend::set_text` under `cfg(windows)`); other targets use the plain setter.
    pub(crate) struct HashingClipboard<B> {
        state: Mutex<HashingState<B>>,
    }

    fn digest(text: &str) -> [u8; 32] {
        Sha256::digest(text.as_bytes()).into()
    }

    impl<B: ClipboardBackend> HashingClipboard<B> {
        pub(crate) fn new(backend: B) -> Self {
            Self {
                state: Mutex::new(HashingState {
                    backend,
                    writes: 0,
                    last: None,
                }),
            }
        }

        fn lock(&self) -> MutexGuard<'_, HashingState<B>> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        #[cfg(test)]
        pub(crate) fn with_backend<R>(&self, f: impl FnOnce(&mut B) -> R) -> R {
            f(&mut self.lock().backend)
        }
    }

    impl<B: ClipboardBackend> SecureClipboard for HashingClipboard<B> {
        fn write_secret(&self, text: &str) -> PlatformResult<crate::ClipboardStamp> {
            let mut s = self.lock();
            s.writes += 1;
            // A failed write leaves no claim on the clipboard.
            s.last = None;
            s.backend.set_text(text).map_err(PlatformError::Failed)?;
            let stamp = s.writes;
            s.last = Some((stamp, digest(text)));
            Ok(ClipboardStamp(stamp))
        }

        // Read-back then clear is not atomic: a foreign copy in between can be wiped (accepted;
        // see `SecureClipboard::clear_if_unchanged`).
        fn clear_if_unchanged(&self, stamp: &ClipboardStamp) -> PlatformResult<bool> {
            let mut s = self.lock();
            let Some((last_stamp, last_hash)) = s.last else {
                return Ok(false);
            };
            if last_stamp != stamp.0 {
                return Ok(false);
            }
            let current = s.backend.get_text().map_err(PlatformError::Failed)?;
            if current.as_deref().map(digest) != Some(last_hash) {
                s.last = None;
                return Ok(false);
            }
            s.backend.clear().map_err(PlatformError::Failed)?;
            s.last = None;
            Ok(true)
        }
    }

    impl<B> std::fmt::Debug for HashingClipboard<B> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("HashingClipboard").finish_non_exhaustive()
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) use hashing::{ClipboardBackend, HashingClipboard};

/// The system clipboard through `arboard`. A fresh `arboard::Clipboard` per call: the handle is
/// cheap and this keeps the backend free of any thread-affinity question.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub(crate) struct ArboardBackend;

#[cfg(not(target_os = "macos"))]
impl ClipboardBackend for ArboardBackend {
    // Only ever called for secrets (`HashingClipboard::write_secret`), so on Windows the write
    // opts out of clipboard history, the cloud clipboard and monitoring.
    #[cfg(windows)]
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        use arboard::SetExtWindows;
        arboard::Clipboard::new()
            .and_then(|mut c| {
                c.set()
                    .exclude_from_monitoring()
                    .exclude_from_cloud()
                    .exclude_from_history()
                    .text(text)
            })
            .map_err(|e| e.to_string())
    }

    #[cfg(not(windows))]
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.set_text(text))
            .map_err(|e| e.to_string())
    }

    fn get_text(&mut self) -> Result<Option<String>, String> {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn clear(&mut self) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.clear())
            .map_err(|e| e.to_string())
    }
}

/// The platform of a target without a native implementation: `arboard` for the clipboard off
/// macOS, `Unsupported` for everything else.
#[cfg(not(target_os = "macos"))]
pub(crate) fn platform(_paths: &AppPaths) -> Platform {
    Platform {
        events: Arc::new(UnsupportedSystemEvents),
        clipboard: Arc::new(HashingClipboard::new(ArboardBackend)),
        secrets: Arc::new(UnsupportedSecretStore),
        shared_secrets: Arc::new(UnsupportedSecretStore),
        device_unlock: Arc::new(UnsupportedDeviceUnlock),
        host_registrar: Arc::new(UnsupportedHostRegistrar),
        launch_at_login: Arc::new(UnsupportedLaunchAtLogin),
        keyboard_layout: Arc::new(UnsupportedKeyboardLayout),
    }
}

/// No running-application lookup off macOS.
#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn running_app_executables(_bundle_id: &str) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use std::path::Path;
    use std::sync::Arc;

    #[test]
    fn fallback_system_events_are_unsupported_not_panicking() {
        let sink: Arc<dyn Fn(SystemEvent) + Send + Sync> = Arc::new(|_| {});
        assert!(matches!(
            UnsupportedSystemEvents.subscribe(sink),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn fallback_device_unlock_reports_unavailable() {
        let du = UnsupportedDeviceUnlock;
        assert_eq!(du.availability(), DeviceUnlockAvailability::Unavailable);
        assert!(matches!(
            du.unlock(DeviceUnlockMode::Biometrics, "v", "r"),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(
            du.is_enrolled(DeviceUnlockMode::AccountPassword, "v"),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(du.forget("v"), Err(PlatformError::Unsupported(_))));
    }

    #[test]
    fn fallback_launch_at_login_registrar_keyboard_layout_are_unsupported() {
        assert_eq!(
            UnsupportedLaunchAtLogin.status(),
            Ok(LoginItemStatus::Unsupported)
        );
        assert_eq!(UnsupportedLaunchAtLogin.is_enabled(), Ok(false));
        assert!(matches!(
            UnsupportedLaunchAtLogin.set_enabled(true),
            Err(PlatformError::Unsupported(_))
        ));
        let r = UnsupportedHostRegistrar;
        assert!(matches!(
            r.register(Path::new("/x")),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(r.unregister(), Err(PlatformError::Unsupported(_))));
        assert!(r.status().is_empty());
        assert!(matches!(
            UnsupportedKeyboardLayout.current(),
            Err(PlatformError::Unsupported(_))
        ));
        let sink: Arc<dyn Fn(Option<String>) + Send + Sync> = Arc::new(|_| {});
        assert!(matches!(
            UnsupportedKeyboardLayout.subscribe(sink),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(
            UnsupportedSecretStore.get("s", "a"),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn fallback_running_app_executables_is_empty() {
        assert!(super::running_app_executables("com.mypass.app").is_empty());
    }

    /// An in-memory clipboard backend standing in for the system clipboard.
    #[derive(Default)]
    struct MemoryBackend {
        text: Option<String>,
        fail_reads: bool,
    }

    impl ClipboardBackend for MemoryBackend {
        fn set_text(&mut self, text: &str) -> Result<(), String> {
            self.text = Some(text.to_owned());
            Ok(())
        }
        fn get_text(&mut self) -> Result<Option<String>, String> {
            if self.fail_reads {
                return Err("busy".to_owned());
            }
            Ok(self.text.clone())
        }
        fn clear(&mut self) -> Result<(), String> {
            self.text = None;
            Ok(())
        }
    }

    #[test]
    fn hashing_clipboard_stamps_increase_and_clear_when_unchanged() {
        let c = HashingClipboard::new(MemoryBackend::default());
        let a = c.write_secret("one").unwrap();
        let b = c.write_secret("two").unwrap();
        assert!(b.0 > a.0);
        // An older stamp never clears a newer write of ours.
        assert_eq!(c.clear_if_unchanged(&a), Ok(false));
        assert_eq!(c.with_backend(|be| be.text.clone()), Some("two".to_owned()));
        assert_eq!(c.clear_if_unchanged(&b), Ok(true));
        assert_eq!(c.with_backend(|be| be.text.clone()), None);
        // Clearing twice is a no-op.
        assert_eq!(c.clear_if_unchanged(&b), Ok(false));
    }

    #[test]
    fn hashing_clipboard_skips_after_foreign_copy() {
        let c = HashingClipboard::new(MemoryBackend::default());
        let stamp = c.write_secret("pw").unwrap();
        c.with_backend(|be| be.text = Some("someone else".to_owned()));
        assert_eq!(c.clear_if_unchanged(&stamp), Ok(false));
        assert_eq!(
            c.with_backend(|be| be.text.clone()),
            Some("someone else".to_owned())
        );
        let stamp = c.write_secret("pw").unwrap();
        c.with_backend(|be| be.text = None);
        assert_eq!(c.clear_if_unchanged(&stamp), Ok(false));
    }

    #[test]
    fn hashing_clipboard_reports_backend_errors_and_never_leaks_the_secret() {
        let c = HashingClipboard::new(MemoryBackend::default());
        let stamp = c.write_secret("hunter2").unwrap();
        c.with_backend(|be| be.fail_reads = true);
        assert_eq!(
            c.clear_if_unchanged(&stamp),
            Err(PlatformError::Failed("busy".to_owned()))
        );
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
