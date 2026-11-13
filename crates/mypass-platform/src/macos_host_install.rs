//! The macOS body of [`NativeHostRegistrar`]: installs the host binary under the app's data folder
//! and writes one native messaging manifest per Chromium-family browser.
//!
//! Rules (ported from the old `scripts/dev.sh`):
//!
//! - Nothing is ever written outside `host_dir` and the per-user `NativeMessagingHosts` folders of
//!   the known browsers, and every existing path component below the home folder must be a real
//!   directory (no symlink is followed).
//! - A manifest is written through a temp file in its own folder (mode 0600 at open) and renamed;
//!   the destination must be absent or a plain regular file.
//! - The host copy is hash-checked, written through a temp file (mode 0755 at open) and renamed.
//!   An empty or non-executable bundled host (the build placeholder) is refused before anything is
//!   copied or written.
//! - `unregister` removes only the files this registrar created.
//!
//! Residual risk, accepted with the 0600 endpoint file: the installed host lives in the user's own
//! Application Support folder, so a same-user process can replace it. The Stage 5 roadmap moves
//! trust to the signed bundle.

use crate::host_registrar::manifest_json;
use crate::{
    AppPaths, Browser, BrowserRegistration, HostRegistrationReport, NativeHostRegistrar,
    PlatformError, PlatformResult, RegistrationState,
};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const MAX_TEMP_ATTEMPTS: usize = 8;
const NOT_BUILT: &str = "host not built";

/// Registers the host for the Chromium-family browsers of one user.
#[derive(Debug, Clone)]
pub struct MacHostRegistrar {
    paths: AppPaths,
}

impl MacHostRegistrar {
    /// A registrar over `paths` (tests build `AppPaths::macos(tempdir)`).
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    /// Whether `browser` is installed enough to register: its vendor folder exists (a real
    /// directory), or it is one of the always-registered browsers.
    fn is_target(&self, browser: Browser) -> bool {
        if browser.always_register() {
            return true;
        }
        let base = self
            .paths
            .home()
            .join("Library")
            .join("Application Support")
            .join(browser.vendor_dir());
        fs::symlink_metadata(base).is_ok_and(|m| m.is_dir())
    }

    fn row(
        browser: Browser,
        state: RegistrationState,
        manifest: Option<PathBuf>,
        detail: Option<String>,
    ) -> BrowserRegistration {
        BrowserRegistration {
            id: browser.id().to_owned(),
            display: browser.display_name().to_owned(),
            state,
            manifest_path: manifest,
            detail,
        }
    }

    fn error_rows(&self, detail: &str) -> Vec<BrowserRegistration> {
        Browser::ALL
            .into_iter()
            .filter(|b| self.is_target(*b))
            .map(|b| {
                Self::row(
                    b,
                    RegistrationState::Error,
                    Some(self.paths.manifest_path(b)),
                    Some(detail.to_owned()),
                )
            })
            .collect()
    }

    /// Copies `bundled` to `host_exe` unless the hash is equal. `Ok(true)` when it copied.
    fn install_host(&self, bundled: &Path) -> PlatformResult<bool> {
        let bytes = fs::read(bundled).map_err(|e| io_err("read the bundled host", bundled, &e))?;
        let dest = &self.paths.host_exe;
        check_dir_chain(self.paths.home(), &self.paths.host_dir)?;
        check_file_dest(dest)?;
        if let Ok(existing) = fs::read(dest) {
            if Sha256::digest(&existing) == Sha256::digest(&bytes) && is_executable(dest) {
                return Ok(false);
            }
        }
        create_dir_private(&self.paths.host_dir)?;
        write_atomic(dest, &bytes, 0o755)?;
        Ok(true)
    }

    /// What the manifest of one browser says now: the state and, when stale, what it points at.
    fn read_state(&self, manifest: &Path) -> (RegistrationState, Option<String>) {
        let meta = match fs::symlink_metadata(manifest) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => return (RegistrationState::Missing, None),
            Err(e) => return (RegistrationState::Error, Some(e.to_string())),
        };
        if !meta.is_file() {
            return (
                RegistrationState::Error,
                Some("not a regular file".to_owned()),
            );
        }
        let Ok(text) = fs::read_to_string(manifest) else {
            return (
                RegistrationState::Stale,
                Some("unreadable manifest".to_owned()),
            );
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return (
                RegistrationState::Stale,
                Some("unparsable manifest".to_owned()),
            );
        };
        // Ok only when both the target and the allowed origins are exactly ours.
        let expected = manifest_json(&self.paths.host_exe)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok());
        if expected.as_ref() == Some(&value) {
            (RegistrationState::Ok, None)
        } else {
            let target = value
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("an unknown target");
            (RegistrationState::Stale, Some(target.to_owned()))
        }
    }

    fn register_one(&self, browser: Browser, json: &str) -> BrowserRegistration {
        let manifest = self.paths.manifest_path(browser);
        let outcome = (|| -> PlatformResult<(RegistrationState, Option<String>)> {
            let dir = manifest
                .parent()
                .ok_or_else(|| PlatformError::Failed("manifest path has no parent".to_owned()))?;
            check_dir_chain(self.paths.home(), dir)?;
            check_file_dest(&manifest)?;
            let previous = self.read_state(&manifest);
            create_dir_private(dir)?;
            write_atomic(&manifest, format!("{json}\n").as_bytes(), 0o600)?;
            Ok(previous)
        })();
        match outcome {
            Ok((RegistrationState::Stale, target)) => Self::row(
                browser,
                RegistrationState::Stale,
                Some(manifest),
                target.map(|t| format!("rewrote a manifest that pointed at {t}")),
            ),
            Ok(_) => Self::row(browser, RegistrationState::Ok, Some(manifest), None),
            Err(e) => Self::row(
                browser,
                RegistrationState::Error,
                Some(manifest),
                Some(e.to_string()),
            ),
        }
    }

    fn unregister_one(&self, browser: Browser) -> Option<BrowserRegistration> {
        let manifest = self.paths.manifest_path(browser);
        let result = (|| -> PlatformResult<bool> {
            let dir = manifest
                .parent()
                .ok_or_else(|| PlatformError::Failed("manifest path has no parent".to_owned()))?;
            check_dir_chain(self.paths.home(), dir)?;
            match fs::symlink_metadata(&manifest) {
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(io_err("inspect", &manifest, &e)),
                Ok(_) => {}
            }
            check_file_dest(&manifest)?;
            fs::remove_file(&manifest).map_err(|e| io_err("remove", &manifest, &e))?;
            Ok(true)
        })();
        match result {
            Ok(true) => Some(Self::row(
                browser,
                RegistrationState::Missing,
                Some(manifest),
                None,
            )),
            Ok(false) => None,
            Err(e) => Some(Self::row(
                browser,
                RegistrationState::Error,
                Some(manifest),
                Some(e.to_string()),
            )),
        }
    }
}

impl NativeHostRegistrar for MacHostRegistrar {
    fn register(&self, bundled_host: &Path) -> PlatformResult<HostRegistrationReport> {
        let mut report = HostRegistrationReport::default();
        // The build placeholder (empty or not executable) must never be installed or advertised.
        let usable = fs::metadata(bundled_host)
            .is_ok_and(|m| m.is_file() && m.len() > 0 && m.mode() & 0o111 != 0);
        if !usable {
            report.registrations = self.error_rows(NOT_BUILT);
            return Ok(report);
        }
        let json = manifest_json(&self.paths.host_exe)
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        match self.install_host(bundled_host) {
            Ok(copied) => report.host_copied = copied,
            Err(e) => {
                report.registrations = self.error_rows(&e.to_string());
                return Ok(report);
            }
        }
        for browser in Browser::ALL {
            if self.is_target(browser) {
                report.registrations.push(self.register_one(browser, &json));
            } else {
                report.skipped.push(browser.id().to_owned());
            }
        }
        Ok(report)
    }

    fn status(&self) -> Vec<BrowserRegistration> {
        Browser::ALL
            .into_iter()
            .filter(|b| self.is_target(*b))
            .map(|b| {
                let manifest = self.paths.manifest_path(b);
                let chain_ok = manifest
                    .parent()
                    .is_some_and(|d| check_dir_chain(self.paths.home(), d).is_ok());
                if !chain_ok {
                    return Self::row(
                        b,
                        RegistrationState::Error,
                        Some(manifest),
                        Some("unsafe manifest folder".to_owned()),
                    );
                }
                let (state, detail) = self.read_state(&manifest);
                Self::row(b, state, Some(manifest), detail)
            })
            .collect()
    }

    fn unregister(&self) -> PlatformResult<HostRegistrationReport> {
        let mut report = HostRegistrationReport::default();
        for browser in Browser::ALL {
            if self.is_target(browser) {
                report.registrations.extend(self.unregister_one(browser));
            } else {
                report.skipped.push(browser.id().to_owned());
            }
        }
        let exe = &self.paths.host_exe;
        if check_dir_chain(self.paths.home(), &self.paths.host_dir).is_ok()
            && fs::symlink_metadata(exe).is_ok_and(|m| m.is_file())
            && fs::remove_file(exe).is_ok()
        {
            report.host_copied = true;
            // Only an empty folder goes; anything else in it is not ours.
            let _ = fs::remove_dir(&self.paths.host_dir);
        }
        Ok(report)
    }
}

fn io_err(action: &str, path: &Path, e: &std::io::Error) -> PlatformError {
    PlatformError::Failed(format!("{action} {}: {:?}", path.display(), e.kind()))
}

fn refuse(path: &Path, why: &str) -> PlatformError {
    PlatformError::Failed(format!("refusing to use {}: {why}", path.display()))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

/// Walks `dir` below `home` and refuses any existing component that is a symlink or not a
/// directory. Components that do not exist yet are fine: they are created under verified parents.
fn check_dir_chain(home: &Path, dir: &Path) -> PlatformResult<()> {
    let rel = dir
        .strip_prefix(home)
        .map_err(|_| refuse(dir, "outside the home folder"))?;
    let mut cur = home.to_path_buf();
    for part in rel.components() {
        cur.push(part);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Err(refuse(&cur, "it is a symlink")),
            Ok(m) if !m.is_dir() => return Err(refuse(&cur, "it is not a directory")),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => break,
            Err(e) => return Err(io_err("inspect", &cur, &e)),
        }
    }
    Ok(())
}

/// An existing destination must be a plain regular file.
fn check_file_dest(path: &Path) -> PlatformResult<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Ok(()),
        Ok(_) => Err(refuse(path, "it is not a regular file")),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("inspect", path, &e)),
    }
}

/// Creates `dir` (and missing parents) with mode 0700 for the folders this call makes.
fn create_dir_private(dir: &Path) -> PlatformResult<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder
        .create(dir)
        .map_err(|e| io_err("create directory", dir, &e))
}

/// Temp file in the destination folder (mode set at open), sync, rename onto `dest`.
fn write_atomic(dest: &Path, bytes: &[u8], mode: u32) -> PlatformResult<()> {
    let dir = dest
        .parent()
        .ok_or_else(|| refuse(dest, "no parent folder"))?;
    let (mut file, temp) = create_temp(dir, mode)?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_err("write", &temp, &e))
        .and_then(|()| fs::rename(&temp, dest).map_err(|e| io_err("rename onto", dest, &e)));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn create_temp(dir: &Path, mode: u32) -> PlatformResult<(File, PathBuf)> {
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".tmp-{}-{n}.partial", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(mode);
        match options.open(&path) {
            Ok(file) => {
                // The umask can only narrow the mode at open; make an executable explicit.
                if mode == 0o755 {
                    let _ = file.set_permissions(fs::Permissions::from_mode(mode));
                }
                return Ok((file, path));
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err("create a temp file in", dir, &e)),
        }
    }
    Err(refuse(dir, "could not pick a free temp name"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture {
        _dir: tempfile::TempDir,
        home: PathBuf,
        paths: AppPaths,
        host: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("h");
        fs::create_dir_all(&home).unwrap();
        let host = dir.path().join("bundled-host");
        fs::write(&host, b"host-v1").unwrap();
        fs::set_permissions(&host, fs::Permissions::from_mode(0o755)).unwrap();
        Fixture {
            paths: AppPaths::macos(&home),
            home,
            host,
            _dir: dir,
        }
    }

    fn support(f: &Fixture) -> PathBuf {
        f.home.join("Library").join("Application Support")
    }

    fn state_of(rows: &[BrowserRegistration], b: Browser) -> RegistrationState {
        rows.iter().find(|r| r.id == b.id()).unwrap().state
    }

    #[test]
    fn register_copies_host_and_marks_executable() {
        let f = fixture();
        let r = MacHostRegistrar::new(f.paths.clone());
        let report = r.register(&f.host).unwrap();
        assert!(report.host_copied);
        assert_eq!(fs::read(&f.paths.host_exe).unwrap(), b"host-v1");
        let mode = fs::metadata(&f.paths.host_exe).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn register_skips_unchanged_host_by_hash() {
        let f = fixture();
        let r = MacHostRegistrar::new(f.paths.clone());
        r.register(&f.host).unwrap();
        let before = fs::metadata(&f.paths.host_exe).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let report = r.register(&f.host).unwrap();
        assert!(!report.host_copied);
        let after = fs::metadata(&f.paths.host_exe).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn register_updates_changed_host_atomically() {
        let f = fixture();
        let r = MacHostRegistrar::new(f.paths.clone());
        r.register(&f.host).unwrap();
        fs::write(&f.host, b"host-v2").unwrap();
        let report = r.register(&f.host).unwrap();
        assert!(report.host_copied);
        assert_eq!(fs::read(&f.paths.host_exe).unwrap(), b"host-v2");
        let leftovers: Vec<_> = fs::read_dir(&f.paths.host_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no temp file left: {leftovers:?}");
    }

    #[test]
    fn register_writes_manifest_only_for_existing_browser_dirs_plus_always() {
        let f = fixture();
        fs::create_dir_all(support(&f).join("Microsoft Edge")).unwrap();
        fs::create_dir_all(support(&f).join("Arc").join("User Data")).unwrap();
        let r = MacHostRegistrar::new(f.paths.clone());
        let report = r.register(&f.host).unwrap();
        assert_eq!(report.registrations.len(), 4);
        for b in [
            Browser::Chrome,
            Browser::ChromeForTesting,
            Browser::Edge,
            Browser::Arc,
        ] {
            let p = f.paths.manifest_path(b);
            assert!(p.is_file(), "{p:?}");
            assert_eq!(state_of(&report.registrations, b), RegistrationState::Ok);
            assert_eq!(fs::metadata(&p).unwrap().mode() & 0o777, 0o600);
        }
        assert_eq!(report.skipped.len(), 14);
        assert!(!f.paths.manifest_path(Browser::Brave).exists());
    }

    #[test]
    fn opera_manifest_path_is_application_support_com_operasoftware_opera() {
        let f = fixture();
        fs::create_dir_all(support(&f).join("com.operasoftware.Opera")).unwrap();
        MacHostRegistrar::new(f.paths.clone())
            .register(&f.host)
            .unwrap();
        assert!(support(&f)
            .join("com.operasoftware.Opera/NativeMessagingHosts/com.mypass.chromehost.json")
            .is_file());
    }

    #[test]
    fn manifest_content_points_at_installed_host_with_one_origin() {
        let f = fixture();
        MacHostRegistrar::new(f.paths.clone())
            .register(&f.host)
            .unwrap();
        let text = fs::read_to_string(f.paths.manifest_path(Browser::Chrome)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v["path"].as_str().unwrap(),
            f.paths.host_exe.to_str().unwrap()
        );
        assert_eq!(v["allowed_origins"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn register_refuses_symlinked_native_messaging_hosts_dir() {
        let f = fixture();
        let chrome = support(&f).join("Google").join("Chrome");
        fs::create_dir_all(&chrome).unwrap();
        let elsewhere = f.home.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        symlink(&elsewhere, chrome.join("NativeMessagingHosts")).unwrap();
        let report = MacHostRegistrar::new(f.paths.clone())
            .register(&f.host)
            .unwrap();
        assert_eq!(
            state_of(&report.registrations, Browser::Chrome),
            RegistrationState::Error
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[test]
    fn register_refuses_directory_at_manifest_path() {
        let f = fixture();
        fs::create_dir_all(f.paths.manifest_path(Browser::Chrome)).unwrap();
        let report = MacHostRegistrar::new(f.paths.clone())
            .register(&f.host)
            .unwrap();
        assert_eq!(
            state_of(&report.registrations, Browser::Chrome),
            RegistrationState::Error
        );
    }

    #[test]
    fn one_browser_failure_does_not_stop_the_rest() {
        let f = fixture();
        fs::create_dir_all(f.paths.manifest_path(Browser::Chrome)).unwrap();
        let report = MacHostRegistrar::new(f.paths.clone())
            .register(&f.host)
            .unwrap();
        assert_eq!(
            state_of(&report.registrations, Browser::ChromeForTesting),
            RegistrationState::Ok
        );
        assert!(f.paths.manifest_path(Browser::ChromeForTesting).is_file());
    }

    #[test]
    fn register_rewrites_stale_manifest_and_reports_stale() {
        let f = fixture();
        let path = f.paths.manifest_path(Browser::Chrome);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, manifest_json(Path::new("/old/mypass-host")).unwrap()).unwrap();
        let r = MacHostRegistrar::new(f.paths.clone());
        let report = r.register(&f.host).unwrap();
        assert_eq!(
            state_of(&report.registrations, Browser::Chrome),
            RegistrationState::Stale
        );
        assert_eq!(
            state_of(&r.status(), Browser::Chrome),
            RegistrationState::Ok
        );
    }

    #[test]
    fn register_is_idempotent() {
        let f = fixture();
        let r = MacHostRegistrar::new(f.paths.clone());
        r.register(&f.host).unwrap();
        let again = r.register(&f.host).unwrap();
        assert!(!again.host_copied);
        assert!(again
            .registrations
            .iter()
            .all(|x| x.state == RegistrationState::Ok));
    }

    #[test]
    fn register_rejects_control_char_in_host_path() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(
            "h
x",
        );
        fs::create_dir_all(&home).unwrap();
        let host = dir.path().join("host");
        fs::write(&host, b"x").unwrap();
        fs::set_permissions(&host, fs::Permissions::from_mode(0o755)).unwrap();
        let r = MacHostRegistrar::new(AppPaths::macos(&home));
        assert!(r.register(&host).is_err());
        assert!(!home.join("Library").exists());
    }

    #[test]
    fn register_refuses_empty_placeholder_host() {
        let f = fixture();
        fs::write(&f.host, b"").unwrap();
        let r = MacHostRegistrar::new(f.paths.clone());
        let report = r.register(&f.host).unwrap();
        assert!(!report.host_copied);
        assert!(!report.registrations.is_empty());
        assert!(report
            .registrations
            .iter()
            .all(|x| x.state == RegistrationState::Error
                && x.detail.as_deref() == Some("host not built")));
        assert!(!f.paths.host_exe.exists());
        assert!(!f.paths.manifest_path(Browser::Chrome).exists());
        // not executable is refused too
        fs::write(&f.host, b"x").unwrap();
        fs::set_permissions(&f.host, fs::Permissions::from_mode(0o644)).unwrap();
        let report = r.register(&f.host).unwrap();
        assert!(!report.host_copied);
        assert!(!f.paths.host_exe.exists());
    }

    #[test]
    fn status_reports_ok_missing_and_stale() {
        let f = fixture();
        let r = MacHostRegistrar::new(f.paths.clone());
        assert_eq!(
            state_of(&r.status(), Browser::Chrome),
            RegistrationState::Missing
        );
        r.register(&f.host).unwrap();
        assert_eq!(
            state_of(&r.status(), Browser::Chrome),
            RegistrationState::Ok
        );
        let path = f.paths.manifest_path(Browser::ChromeForTesting);
        fs::write(&path, manifest_json(Path::new("/old/mypass-host")).unwrap()).unwrap();
        let rows = r.status();
        assert_eq!(
            state_of(&rows, Browser::ChromeForTesting),
            RegistrationState::Stale
        );
        // status never writes
        assert!(fs::read_to_string(&path).unwrap().contains("/old/"));
    }

    #[test]
    fn unregister_removes_only_our_files() {
        let f = fixture();
        let r = MacHostRegistrar::new(f.paths.clone());
        r.register(&f.host).unwrap();
        let dir = f.paths.manifest_path(Browser::Chrome);
        let other = dir.parent().unwrap().join("com.other.host.json");
        fs::write(&other, b"{}").unwrap();
        let report = r.unregister().unwrap();
        assert!(report.host_copied);
        assert!(!f.paths.manifest_path(Browser::Chrome).exists());
        assert!(!f.paths.host_exe.exists());
        assert!(other.is_file());
    }

    #[test]
    fn unregister_does_not_follow_a_symlinked_manifest() {
        let f = fixture();
        let path = f.paths.manifest_path(Browser::Chrome);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let victim = f.home.join("victim");
        fs::write(&victim, b"keep").unwrap();
        symlink(&victim, &path).unwrap();
        let report = MacHostRegistrar::new(f.paths.clone()).unregister().unwrap();
        assert_eq!(
            state_of(&report.registrations, Browser::Chrome),
            RegistrationState::Error
        );
        assert_eq!(fs::read(&victim).unwrap(), b"keep");
    }
}
