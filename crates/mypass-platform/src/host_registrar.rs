//! Registration of the native messaging host with the Chromium-family browsers.
//!
//! Stage 3 (macOS) writes the native messaging manifests; stage 6 (Windows) writes the manifest and
//! the registry keys. The registrar owns its [`crate::AppPaths`] (given to [`crate::current`] or its
//! constructor); the extension id is the pinned constant of stage 3's manifest builder, not a
//! parameter.

use crate::PlatformResult;
use std::path::{Path, PathBuf};

/// Registration state of one browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationState {
    /// Our manifest is present and points at our host.
    Ok,
    /// No manifest of ours.
    Missing,
    /// A manifest of ours points somewhere else.
    Stale,
    /// The state could not be determined or written.
    Error,
}

/// One browser's row in a registration report and in the Browser pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRegistration {
    /// [`crate::Browser::id`].
    pub id: String,
    /// [`crate::Browser::display_name`].
    pub display: String,
    /// The state.
    pub state: RegistrationState,
    /// The manifest path, when one applies.
    pub manifest_path: Option<PathBuf>,
    /// Error text for `Error`, the stale target for `Stale`.
    pub detail: Option<String>,
}

/// What a `register` or `unregister` call did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostRegistrationReport {
    /// The host binary was copied (or removed, for `unregister`).
    pub host_copied: bool,
    /// One row per browser that was written or checked.
    pub registrations: Vec<BrowserRegistration>,
    /// Browser ids whose browser is not installed, so nothing was written.
    pub skipped: Vec<String>,
}

/// Installs, inspects and removes the native messaging host registration.
pub trait NativeHostRegistrar: Send + Sync {
    /// Copies the host (hash-checked) and writes the manifests (and, on Windows, registry keys).
    /// Per-browser failures are reported in `registrations`, never fatal.
    fn register(&self, bundled_host: &Path) -> PlatformResult<HostRegistrationReport>;
    /// Read-only: `Ok` / `Missing` / `Stale` / `Error` per browser; never writes.
    fn status(&self) -> Vec<BrowserRegistration>;
    /// Removes only our own manifests, keys and host copy.
    fn unregister(&self) -> PlatformResult<HostRegistrationReport>;
}
