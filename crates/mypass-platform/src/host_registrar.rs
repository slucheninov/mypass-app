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

/// The one extension id allowed to talk to the host (the Chrome Web Store id pinned by the old app's
/// `scripts/dev.sh`). Never a wildcard: Chrome refuses the connection from any other origin.
pub const EXTENSION_ID: &str = "hjijbabodfldconmhdcdphlbgaajkejn";
/// The native messaging host name the extension passes to `connectNative`.
pub const HOST_NAME: &str = "com.mypass.chromehost";
/// File name of the host binary cargo builds (`mypass-host` package) on the running OS.
#[cfg(windows)]
const HOST_FILE: &str = "mypass-host.exe";
#[cfg(not(windows))]
const HOST_FILE: &str = "mypass-host";
/// File name of the host as the app bundles it (the Tauri `externalBin` sidecar, staged by
/// `scripts/build-host.sh`). It differs from [`HOST_FILE`] on purpose: tauri-build copies the
/// sidecar to `<profile dir>/<this name>`, and a sidecar called `mypass-host` would replace cargo's
/// own `mypass-host` output there. The installed copy and the manifest `path` keep `mypass-host`.
#[cfg(windows)]
pub const BUNDLED_HOST_FILE: &str = "mypass-native-host.exe";
#[cfg(not(windows))]
pub const BUNDLED_HOST_FILE: &str = "mypass-native-host";

/// Why a manifest cannot be built.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    /// The host path is not valid UTF-8, so JSON cannot carry it faithfully.
    #[error("host path is not valid UTF-8")]
    NotUtf8,
    /// A control character (newline, tab, NUL, ...) cannot sit safely in a manifest path.
    #[error("host path contains a control character")]
    ControlCharacter,
}

/// Field order is the serialisation order (`allowed_origins`, `description`, `name`, `path`, `type`),
/// fixed by the struct rather than by a map, so a feature-unified `preserve_order` cannot reorder it.
#[derive(serde::Serialize)]
struct Manifest<'a> {
    allowed_origins: [String; 1],
    description: &'static str,
    name: &'static str,
    path: &'a str,
    r#type: &'static str,
}

/// The native messaging manifest for the host at `host_path`: one allowed origin, ours.
pub fn manifest_json(host_path: &Path) -> Result<String, ManifestError> {
    let path = host_path.to_str().ok_or(ManifestError::NotUtf8)?;
    if path.chars().any(char::is_control) {
        return Err(ManifestError::ControlCharacter);
    }
    let manifest = Manifest {
        allowed_origins: [format!("chrome-extension://{EXTENSION_ID}/")],
        description: "MyPass native messaging host",
        name: HOST_NAME,
        path,
        r#type: "stdio",
    };
    // Unreachable: serialising a struct of plain strings cannot fail. The variant only keeps the
    // function total without a panic.
    serde_json::to_string_pretty(&manifest).map_err(|_| ManifestError::NotUtf8)
}

/// The host binary shipped next to the running executable: `<exe dir>/mypass-native-host` (the
/// macOS bundle's `Contents/MacOS`, or the Windows zip), else the cargo-built `mypass-host` in the
/// same directory, else in the cargo profile directory (`target/debug`), which is the parent when
/// the executable is a test binary in `deps/`. An EMPTY file (the build placeholder) is skipped, so
/// a dev run finds the real cargo-built host behind it.
pub fn locate_bundled_host(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let mut dirs = vec![dir];
    if dir.file_name().is_some_and(|n| n == "deps") {
        dirs.extend(dir.parent());
    }
    let mut candidates = Vec::new();
    for d in dirs {
        candidates.push(d.join(BUNDLED_HOST_FILE));
        candidates.push(d.join(HOST_FILE));
    }
    candidates
        .into_iter()
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_json_matches_dev_sh_format() {
        let json = manifest_json(Path::new(
            "/h/Library/Application Support/com.mypass.app/host/mypass-host",
        ))
        .unwrap();
        let expected = "{\n  \"allowed_origins\": [\n    \"chrome-extension://hjijbabodfldconmhdcdphlbgaajkejn/\"\n  ],\n  \"description\": \"MyPass native messaging host\",\n  \"name\": \"com.mypass.chromehost\",\n  \"path\": \"/h/Library/Application Support/com.mypass.app/host/mypass-host\",\n  \"type\": \"stdio\"\n}";
        assert_eq!(json, expected);
    }

    #[test]
    fn manifest_json_escapes_backslashes_and_non_ascii() {
        let input = "C:\\Users\\Zo\u{eb} \\\"q\\\"\\MyPass\\mypass-host.exe";
        let json = manifest_json(Path::new(input)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["path"].as_str().unwrap(), input);
        assert!(json.contains("\\\\Users\\\\"));
    }

    #[test]
    fn manifest_json_rejects_control_characters() {
        for c in ['\n', '\t', '\u{7}', '\u{0}'] {
            let p = format!("/h/mypass{c}host");
            assert_eq!(
                manifest_json(Path::new(&p)),
                Err(ManifestError::ControlCharacter)
            );
        }
    }

    #[test]
    fn manifest_lists_only_our_extension_origin() {
        let json = manifest_json(Path::new("/h/mypass-host")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let origins = v["allowed_origins"].as_array().unwrap();
        assert_eq!(origins.len(), 1);
        assert!(!json.contains('*'));
    }

    #[test]
    fn locate_bundled_host_prefers_the_bundled_sidecar_then_the_cargo_host() {
        let dir = tempfile::tempdir().unwrap();
        let macos = dir.path().join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(macos.join(HOST_FILE), b"x").unwrap();
        std::fs::write(macos.join(BUNDLED_HOST_FILE), b"y").unwrap();
        assert_eq!(
            locate_bundled_host(&macos.join("mypass-app")),
            Some(macos.join(BUNDLED_HOST_FILE))
        );
        // The empty build placeholder is skipped: the cargo-built host behind it is found.
        std::fs::write(macos.join(BUNDLED_HOST_FILE), b"").unwrap();
        assert_eq!(
            locate_bundled_host(&macos.join("mypass-app")),
            Some(macos.join(HOST_FILE))
        );
        // cargo profile dir sibling of a test binary in deps/
        let deps = dir.path().join("target").join("debug").join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        std::fs::write(deps.parent().unwrap().join(HOST_FILE), b"x").unwrap();
        assert_eq!(
            locate_bundled_host(&deps.join("t-123")),
            Some(deps.parent().unwrap().join(HOST_FILE))
        );
        assert_eq!(
            locate_bundled_host(&dir.path().join("none").join("x")),
            None
        );
    }
}
