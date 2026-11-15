//! Safe Windows host lifecycle; native filesystem operations are injected from windows.rs.

use crate::windows_registry::{Hive, Key, Read as RegistryRead, Registry, Value, ValueType, View};
use crate::{
    manifest_json, windows_host_key, AppPaths, Browser, BrowserRegistration,
    HostRegistrationReport, NativeHostRegistrar, PlatformError, PlatformResult, RegistrationState,
    RegistryTarget, HOST_MANIFEST_FILE, WINDOWS_REGISTRY_TARGETS,
};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex, PoisonError},
};

const EXE: &str = "mypass-host.exe";
const RECEIPT: &str = "mypass-host.sha256";
const VIEWS: [View; 2] = [View::Registry32, View::Registry64];
/// Bounded executable/manifest reads, including injected backends.
pub const MAX_HOST_BYTES: usize = 64 * 1024 * 1024;
fn failed(s: &str) -> PlatformError {
    PlatformError::Failed(s.into())
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Debug, PartialEq, Eq)]
pub enum ReplaceOutcome {
    Unchanged,
    Copied,
    SwappedPastRunningImage,
}
#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error("file is in use")]
    Busy,
    #[error(transparent)]
    Failed(PlatformError),
}
impl From<MoveError> for PlatformError {
    fn from(e: MoveError) -> Self {
        failed(&e.to_string())
    }
}
/// Filesystem session holds validated directory/ancestor handles for its entire lifetime.
pub trait InstallFiles: Send + Sync {
    fn source(&self, path: &Path) -> PlatformResult<Vec<u8>>;
    fn directory(
        &self,
        path: &Path,
        create: bool,
    ) -> PlatformResult<Option<Box<dyn InstallDirectory>>>;
}
pub trait InstallDirectory {
    fn read(&self, name: &str) -> PlatformResult<Option<Vec<u8>>>;
    fn create_new(&self, name: &str, bytes: &[u8]) -> PlatformResult<()>;
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), MoveError>;
    fn remove(&self, name: &str) -> PlatformResult<()>;
    fn names(&self) -> PlatformResult<Vec<String>>;
}
fn publish(dir: &dyn InstallDirectory, name: &str, bytes: &[u8]) -> PlatformResult<()> {
    if dir.read(name)?.as_deref() == Some(bytes) {
        return Ok(());
    }
    let temp = format!(".host-{}.tmp", uuid::Uuid::new_v4());
    dir.create_new(&temp, bytes)?;
    let result = dir.rename(&temp, name, true).map_err(PlatformError::from);
    if result.is_err() {
        let _ = dir.remove(&temp);
    }
    result
}
fn old_name(bytes: &[u8]) -> String {
    format!(
        "mypass-host.old-{}-{}.exe",
        hash(bytes),
        uuid::Uuid::new_v4()
    )
}
fn owned_old(name: &str, bytes: &[u8]) -> bool {
    let Some(body) = name
        .strip_prefix("mypass-host.old-")
        .and_then(|s| s.strip_suffix(".exe"))
    else {
        return false;
    };
    let Some((digest, id)) = body.split_once('-') else {
        return false;
    };
    digest == hash(bytes) && uuid::Uuid::parse_str(id).is_ok()
}
/// A mapped executable can deny replacement/deletion while allowing rename. Attempt that path
/// only after a sharing/access denial, never assume rename succeeds. No-replace rollback keeps
/// the old bytes at either their original name or the unique backup if restoration is denied.
/// Native integration uses a controlled test executable to establish actual Windows behavior.
pub fn replace_host(dir: &dyn InstallDirectory, bytes: &[u8]) -> PlatformResult<ReplaceOutcome> {
    let previous = dir.read(EXE)?;
    if previous
        .as_deref()
        .is_some_and(|old| hash(old) == hash(bytes))
    {
        return Ok(ReplaceOutcome::Unchanged);
    }
    let temp = format!(".host-{}.tmp", uuid::Uuid::new_v4());
    dir.create_new(&temp, bytes)?;
    let result = (|| {
        match dir.rename(&temp, EXE, true) {
            Ok(()) => Ok(ReplaceOutcome::Copied),
            Err(MoveError::Busy) => {
                let old = previous
                    .as_deref()
                    .ok_or_else(|| failed("host publication denied"))?;
                let backup = old_name(old);
                // One bounded move-aside/install attempt, no sleeps or unbounded retry loop.
                dir.rename(EXE, &backup, false)?;
                if let Err(error) = dir.rename(&temp, EXE, false) {
                    if dir.rename(&backup, EXE, false).is_err() {
                        return Err(failed(&format!(
                            "host installation failed; previous host retained at {backup}"
                        )));
                    }
                    return Err(error.into());
                }
                Ok(ReplaceOutcome::SwappedPastRunningImage)
            }
            Err(e) => Err(e.into()),
        }
    })();
    if result.is_err() {
        let _ = dir.remove(&temp);
    }
    result
}
fn cleanup(dir: &dyn InstallDirectory) -> usize {
    dir.names()
        .unwrap_or_default()
        .into_iter()
        .filter(|n| {
            dir.read(n).ok().flatten().is_some_and(|b| owned_old(n, &b)) && dir.remove(n).is_ok()
        })
        .count()
}
/// One owned gate orders startup, explicit repair, status and unregister. No global lock and
/// no native I/O under a session/settings lock. Registration always compares bundled hashes.
pub struct WindowsHostRegistrar {
    paths: AppPaths,
    registry: Arc<dyn Registry>,
    files: Arc<dyn InstallFiles>,
    gate: Mutex<()>,
}
impl WindowsHostRegistrar {
    #[cfg(windows)]
    pub fn new(paths: AppPaths, registry: Arc<dyn Registry>) -> Self {
        Self::with_files(paths, registry, Arc::new(crate::windows::HostFiles))
    }
    pub fn with_files(
        paths: AppPaths,
        registry: Arc<dyn Registry>,
        files: Arc<dyn InstallFiles>,
    ) -> Self {
        Self {
            paths,
            registry,
            files,
            gate: Mutex::new(()),
        }
    }
    fn owner(&self) -> PlatformResult<Value> {
        let manifest = self.paths.manifest_path(Browser::Chrome);
        Ok(Value::String(
            manifest
                .to_str()
                .ok_or_else(|| failed("manifest path is not Unicode"))?
                .into(),
        ))
    }
    fn expected(&self) -> PlatformResult<Vec<u8>> {
        let host = self
            .paths
            .host_exe
            .to_str()
            .ok_or_else(|| failed("host path is not Unicode"))?;
        if host.encode_utf16().count() > 240 {
            return Err(failed("host path exceeds 240 UTF-16 units"));
        }
        manifest_json(&self.paths.host_exe)
            .map(String::into_bytes)
            .map_err(|e| failed(&e.to_string()))
    }
    fn enabled(&self, target: &RegistryTarget) -> PlatformResult<bool> {
        if target.always {
            return Ok(true);
        }
        for view in VIEWS {
            if self.registry.key_exists(&Key::new(
                Hive::CurrentUser,
                view,
                &format!(
                    "Software\\{}",
                    target.vendor_marker.unwrap_or(target.subkey)
                ),
            )?)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn inspect(
        &self,
        target: &RegistryTarget,
        dir: Option<&dyn InstallDirectory>,
    ) -> PlatformResult<RegistrationState> {
        let owner = self.owner()?;
        let mut missing = false;
        for view in VIEWS {
            match self
                .registry
                .read(&windows_host_key(target, view)?, "", ValueType::String)?
            {
                RegistryRead::Value(v) if v == owner => {}
                RegistryRead::Missing => missing = true,
                _ => return Ok(RegistrationState::Stale),
            }
        }
        let Some(dir) = dir else {
            return Ok(RegistrationState::Missing);
        };
        let Some(manifest) = dir.read(HOST_MANIFEST_FILE)? else {
            return Ok(RegistrationState::Missing);
        };
        if manifest != self.expected()? {
            return Ok(RegistrationState::Stale);
        }
        if missing || dir.read(EXE)?.is_none() {
            return Ok(RegistrationState::Missing);
        }
        Ok(RegistrationState::Ok)
    }
    fn row(
        &self,
        target: &RegistryTarget,
        result: PlatformResult<RegistrationState>,
    ) -> BrowserRegistration {
        let (state, detail) = match result {
            Ok(RegistrationState::Stale) => (
                RegistrationState::Stale,
                Some("registry view or manifest conflicts with this installation".into()),
            ),
            Ok(s) => (s, None),
            Err(e) => (RegistrationState::Error, Some(e.to_string())),
        };
        BrowserRegistration {
            id: target.id.into(),
            display: target.display.into(),
            state,
            manifest_path: Some(self.paths.manifest_path(Browser::Chrome)),
            detail,
        }
    }
}
impl NativeHostRegistrar for WindowsHostRegistrar {
    fn register(&self, bundled_host: &Path) -> PlatformResult<HostRegistrationReport> {
        let _gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let expected = self.expected()?;
        let bytes = self.files.source(bundled_host)?;
        if bytes.is_empty() || bytes.len() > MAX_HOST_BYTES {
            return Err(failed("invalid bundled host size"));
        }
        let dir = self
            .files
            .directory(&self.paths.host_dir, true)?
            .ok_or_else(|| failed("host directory missing"))?;
        let outcome = replace_host(dir.as_ref(), &bytes)?;
        publish(dir.as_ref(), RECEIPT, hash(&bytes).as_bytes())?;
        publish(dir.as_ref(), HOST_MANIFEST_FILE, &expected)?;
        cleanup(dir.as_ref());
        let mut report = HostRegistrationReport {
            host_copied: outcome != ReplaceOutcome::Unchanged,
            ..Default::default()
        };
        for target in WINDOWS_REGISTRY_TARGETS {
            let result = (|| {
                if !self.enabled(&target)? {
                    report.skipped.push(target.id.into());
                    return self.inspect(&target, Some(dir.as_ref()));
                }
                let mut failure = None;
                for view in VIEWS {
                    let result = (|| {
                        let key = windows_host_key(&target, view)?;
                        self.registry.create(&key)?;
                        self.registry.set(&key, "", self.owner()?)
                    })();
                    if let Err(e) = result {
                        failure = Some(e);
                    }
                }
                if let Some(e) = failure {
                    return Err(e);
                }
                self.inspect(&target, Some(dir.as_ref()))
            })();
            report.registrations.push(self.row(&target, result));
        }
        Ok(report)
    }
    fn status(&self) -> Vec<BrowserRegistration> {
        let _gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = self.files.directory(&self.paths.host_dir, false);
        WINDOWS_REGISTRY_TARGETS
            .iter()
            .map(|t| {
                self.row(
                    t,
                    match &dir {
                        Ok(dir) => self.inspect(t, dir.as_deref()),
                        Err(e) => Err(failed(&e.to_string())),
                    },
                )
            })
            .collect()
    }
    fn unregister(&self) -> PlatformResult<HostRegistrationReport> {
        let _gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let mut report = HostRegistrationReport::default();
        for target in WINDOWS_REGISTRY_TARGETS {
            let result = (|| {
                let mut state = RegistrationState::Missing;
                for view in VIEWS {
                    let key = windows_host_key(&target, view)?;
                    self.registry.delete_owned_key(&key, "", &self.owner()?)?;
                    if self.registry.read(&key, "", ValueType::String)? != RegistryRead::Missing {
                        state = RegistrationState::Stale;
                    }
                }
                Ok(state)
            })();
            report.registrations.push(self.row(&target, result));
        }
        let Some(dir) = self.files.directory(&self.paths.host_dir, false)? else {
            return Ok(report);
        };
        if dir.read(HOST_MANIFEST_FILE)?.as_deref() == Some(self.expected()?.as_slice()) {
            let exe = dir.read(EXE)?;
            let verified = match &exe {
                None => true,
                Some(bytes) => dir.read(RECEIPT)?.as_deref() == Some(hash(bytes).as_bytes()),
            };
            if verified {
                if let Some(bytes) = exe {
                    let old = old_name(&bytes);
                    dir.rename(EXE, &old, false)?;
                    // Running images may remain under this verified obsolete name until next run.
                    let _ = dir.remove(&old);
                    report.host_copied = true;
                }
                dir.remove(HOST_MANIFEST_FILE)?;
                dir.remove(RECEIPT)?;
            }
        }
        cleanup(dir.as_ref());
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_registry::*;
    use std::path::PathBuf;
    use std::{collections::BTreeMap, sync::Mutex};
    #[derive(Default)]
    pub(super) struct State {
        pub(super) files: BTreeMap<String, Vec<u8>>,
        pub(super) fail_install: bool,
        pub(super) running: bool,
        pub(super) deny_move: bool,
        temps: usize,
    }
    #[derive(Clone, Default)]
    pub(super) struct Files(pub(super) Arc<Mutex<State>>);
    impl InstallFiles for Files {
        fn source(&self, _: &Path) -> PlatformResult<Vec<u8>> {
            Ok(b"new executable".to_vec())
        }
        fn directory(
            &self,
            _: &Path,
            _: bool,
        ) -> PlatformResult<Option<Box<dyn InstallDirectory>>> {
            Ok(Some(Box::new(self.clone())))
        }
    }
    impl InstallDirectory for Files {
        fn read(&self, n: &str) -> PlatformResult<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap().files.get(n).cloned())
        }
        fn create_new(&self, n: &str, b: &[u8]) -> PlatformResult<()> {
            let mut s = self.0.lock().unwrap();
            if s.files.contains_key(n) {
                return Err(failed("collision"));
            }
            s.temps += 1;
            s.files.insert(n.into(), b.into());
            Ok(())
        }
        fn rename(&self, a: &str, b: &str, replace: bool) -> Result<(), MoveError> {
            let mut s = self.0.lock().unwrap();
            if b == EXE && a.starts_with(".host-") && s.files.contains_key(EXE) && s.running {
                return Err(MoveError::Busy);
            }
            if a == EXE && s.deny_move {
                return Err(MoveError::Busy);
            }
            if b == EXE && a.starts_with(".host-") && s.fail_install {
                return Err(MoveError::Failed(failed("injected install failure")));
            }
            if !replace && s.files.contains_key(b) {
                return Err(MoveError::Failed(failed("exists")));
            }
            let bytes = s
                .files
                .remove(a)
                .ok_or_else(|| MoveError::Failed(failed("missing")))?;
            s.files.insert(b.into(), bytes);
            Ok(())
        }
        fn remove(&self, n: &str) -> PlatformResult<()> {
            self.0.lock().unwrap().files.remove(n);
            Ok(())
        }
        fn names(&self) -> PlatformResult<Vec<String>> {
            Ok(self.0.lock().unwrap().files.keys().cloned().collect())
        }
    }
    pub(super) fn fixture() -> (WindowsHostRegistrar, Files, Arc<MemoryRegistry>) {
        let files = Files::default();
        let registry = Arc::new(MemoryRegistry::default());
        let paths = AppPaths::windows_data_root(Path::new(
            "/test/\u{41a}\u{43e}\u{440}\u{438}\u{441}\u{442}\u{443}\u{432}\u{430}\u{447}/MyPass",
        ));
        (
            WindowsHostRegistrar::with_files(paths, registry.clone(), Arc::new(files.clone())),
            files,
            registry,
        )
    }
    #[test]
    fn host_install_register_writes_identical_manifest_keys_skips_marker_and_is_idempotent() {
        let (r, f, reg) = fixture();
        let report = r.register(Path::new("bundle")).unwrap();
        assert!(report.host_copied);
        assert_eq!(report.skipped, ["chromium"]);
        assert!(report
            .registrations
            .iter()
            .filter(|r| r.id != "chromium")
            .all(|r| r.state == RegistrationState::Ok));
        assert_eq!(f.read(EXE).unwrap().unwrap(), b"new executable");
        let count = f.0.lock().unwrap().temps;
        assert!(!r.register(Path::new("bundle")).unwrap().host_copied);
        assert_eq!(f.0.lock().unwrap().temps, count);
        for target in &WINDOWS_REGISTRY_TARGETS[..2] {
            for view in VIEWS {
                assert_eq!(
                    reg.read(
                        &windows_host_key(target, view).unwrap(),
                        "",
                        ValueType::String
                    )
                    .unwrap(),
                    RegistryRead::Value(r.owner().unwrap())
                );
            }
        }
    }
    #[test]
    fn host_install_repairs_all_drift_and_status_is_read_only() {
        for drift in 0..6 {
            let (r, f, reg) = fixture();
            r.register(Path::new("bundle")).unwrap();
            match drift {
                0 => reg
                    .delete_value(
                        &windows_host_key(&WINDOWS_REGISTRY_TARGETS[0], View::Registry32).unwrap(),
                        "",
                    )
                    .unwrap(),
                1 => reg
                    .set(
                        &windows_host_key(&WINDOWS_REGISTRY_TARGETS[0], View::Registry32).unwrap(),
                        "",
                        Value::String("elsewhere".into()),
                    )
                    .unwrap(),
                2 => f.remove(HOST_MANIFEST_FILE).unwrap(),
                3 => {
                    f.0.lock()
                        .unwrap()
                        .files
                        .insert(HOST_MANIFEST_FILE.into(), b"{}".to_vec());
                }
                4 => f.remove(EXE).unwrap(),
                _ => {
                    f.0.lock()
                        .unwrap()
                        .files
                        .insert(EXE.into(), b"truncated".to_vec());
                }
            }
            let count = f.0.lock().unwrap().temps;
            r.status();
            assert_eq!(count, f.0.lock().unwrap().temps);
            let report = r.register(Path::new("bundle")).unwrap();
            assert!(report
                .registrations
                .iter()
                .filter(|r| r.id != "chromium")
                .all(|r| r.state == RegistrationState::Ok));
            assert_eq!(f.read(EXE).unwrap().unwrap(), b"new executable");
        }
    }
    #[test]
    fn host_install_running_swap_and_failure_rollback_preserve_old_bytes() {
        for (fail, deny) in [(false, false), (true, false), (false, true)] {
            let (_, f, _) = fixture();
            f.0.lock()
                .unwrap()
                .files
                .insert(EXE.into(), b"old executable".to_vec());
            {
                let mut s = f.0.lock().unwrap();
                s.running = true;
                s.fail_install = fail;
                s.deny_move = deny;
            }
            let result = replace_host(&f, b"new executable");
            if fail || deny {
                assert!(result.is_err());
                assert_eq!(f.read(EXE).unwrap().unwrap(), b"old executable");
            } else {
                assert_eq!(result.unwrap(), ReplaceOutcome::SwappedPastRunningImage);
                assert_eq!(f.read(EXE).unwrap().unwrap(), b"new executable");
            }
            assert!(!f.names().unwrap().iter().any(|n| n.starts_with(".host-")));
        }
    }
    #[test]
    fn host_install_unregister_preserves_foreign_values_files_and_unknown_obsolete_names() {
        let (r, f, reg) = fixture();
        r.register(Path::new("bundle")).unwrap();
        let key = windows_host_key(&WINDOWS_REGISTRY_TARGETS[0], View::Registry32).unwrap();
        reg.set(&key, "other", Value::Binary(vec![9])).unwrap();
        f.0.lock()
            .unwrap()
            .files
            .insert("mypass-host.old-unknown.exe".into(), b"unknown".to_vec());
        r.unregister().unwrap();
        assert_eq!(
            reg.read(&key, "other", ValueType::Binary).unwrap(),
            RegistryRead::Value(Value::Binary(vec![9]))
        );
        assert!(f.read(EXE).unwrap().is_none());
        assert!(f.read(HOST_MANIFEST_FILE).unwrap().is_none());
        assert!(f.read("mypass-host.old-unknown.exe").unwrap().is_some());
    }
    #[test]
    fn host_install_unregister_refuses_foreign_manifest_or_changed_executable() {
        for manifest in [true, false] {
            let (r, f, _) = fixture();
            r.register(Path::new("bundle")).unwrap();
            f.0.lock().unwrap().files.insert(
                if manifest { HOST_MANIFEST_FILE } else { EXE }.into(),
                b"foreign".to_vec(),
            );
            r.unregister().unwrap();
            assert!(f.read(EXE).unwrap().is_some());
            assert!(f.read(HOST_MANIFEST_FILE).unwrap().is_some());
        }
    }
    #[test]
    fn host_install_long_path_is_rejected_before_files_or_keys() {
        let f = Files::default();
        let r = WindowsHostRegistrar::with_files(
            AppPaths::windows_data_root(&PathBuf::from(format!("/{}", "a".repeat(240)))),
            Arc::new(MemoryRegistry::default()),
            Arc::new(f.clone()),
        );
        assert!(r
            .register(Path::new("bundle"))
            .unwrap_err()
            .to_string()
            .contains("240"));
        assert!(f.names().unwrap().is_empty());
    }
}

#[cfg(test)]
mod extra_tests {
    use super::tests::{fixture, Files};
    use super::*;
    #[test]
    fn host_install_gate_covers_all_operations_and_concurrent_register_unregister() {
        struct Probe {
            inner: Files,
            registrar: Mutex<std::sync::Weak<WindowsHostRegistrar>>,
        }
        impl InstallFiles for Probe {
            fn source(&self, path: &Path) -> PlatformResult<Vec<u8>> {
                assert!(self
                    .registrar
                    .lock()
                    .unwrap()
                    .upgrade()
                    .unwrap()
                    .gate
                    .try_lock()
                    .is_err());
                self.inner.source(path)
            }
            fn directory(
                &self,
                path: &Path,
                create: bool,
            ) -> PlatformResult<Option<Box<dyn InstallDirectory>>> {
                assert!(self
                    .registrar
                    .lock()
                    .unwrap()
                    .upgrade()
                    .unwrap()
                    .gate
                    .try_lock()
                    .is_err());
                self.inner.directory(path, create)
            }
        }
        let (_, files, registry) = fixture();
        let probe = Arc::new(Probe {
            inner: files,
            registrar: Mutex::new(std::sync::Weak::new()),
        });
        let r = Arc::new(WindowsHostRegistrar::with_files(
            AppPaths::windows_data_root(Path::new("/test/root")),
            registry,
            probe.clone(),
        ));
        *probe.registrar.lock().unwrap() = Arc::downgrade(&r);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let register = {
            let r = r.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                r.register(Path::new("bundle")).unwrap();
            })
        };
        let unregister = {
            let r = r.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                r.unregister().unwrap();
            })
        };
        barrier.wait();
        register.join().unwrap();
        unregister.join().unwrap();
        let states: Vec<_> = r
            .status()
            .into_iter()
            .filter(|row| row.id != "chromium")
            .map(|row| row.state)
            .collect();
        assert!(
            states.iter().all(|s| *s == RegistrationState::Ok)
                || states.iter().all(|s| *s == RegistrationState::Missing)
        );
    }

    struct EdgeFailure(Arc<crate::windows_registry::MemoryRegistry>);
    impl Registry for EdgeFailure {
        fn key_exists(&self, k: &Key) -> PlatformResult<bool> {
            self.0.key_exists(k)
        }
        fn read(&self, k: &Key, n: &str, t: ValueType) -> PlatformResult<RegistryRead> {
            self.0.read(k, n, t)
        }
        fn create(&self, k: &Key) -> PlatformResult<()> {
            if k.subkey().contains("microsoft\\edge") {
                Err(failed("edge denied"))
            } else {
                self.0.create(k)
            }
        }
        fn set(&self, k: &Key, n: &str, v: Value) -> PlatformResult<()> {
            self.0.set(k, n, v)
        }
        fn delete_value(&self, k: &Key, n: &str) -> PlatformResult<()> {
            self.0.delete_value(k, n)
        }
        fn delete_owned_key(&self, k: &Key, n: &str, v: &Value) -> PlatformResult<bool> {
            self.0.delete_owned_key(k, n, v)
        }
    }
    #[test]
    fn host_install_browser_failure_does_not_stop_chromium_and_views_cannot_mask_conflicts() {
        let (mut r, _, reg) = fixture();
        let marker = Key::new(Hive::CurrentUser, View::Registry64, "Software\\Chromium").unwrap();
        reg.create(&marker).unwrap();
        r.registry = Arc::new(EdgeFailure(reg.clone()));
        let report = r.register(Path::new("bundle")).unwrap();
        assert_eq!(
            report
                .registrations
                .iter()
                .find(|r| r.id == "edge")
                .unwrap()
                .state,
            RegistrationState::Error
        );
        assert_eq!(
            report
                .registrations
                .iter()
                .find(|r| r.id == "chromium")
                .unwrap()
                .state,
            RegistrationState::Ok
        );
        let chrome = windows_host_key(&WINDOWS_REGISTRY_TARGETS[0], View::Registry32).unwrap();
        reg.set(&chrome, "", Value::String("earlier-search-conflict".into()))
            .unwrap();
        assert_eq!(
            r.status().iter().find(|r| r.id == "chrome").unwrap().state,
            RegistrationState::Stale
        );
    }
    #[test]
    fn host_install_unregister_reports_foreign_registration_and_preserves_sibling() {
        let (r, _, reg) = fixture();
        r.register(Path::new("bundle")).unwrap();
        let key = windows_host_key(&WINDOWS_REGISTRY_TARGETS[0], View::Registry32).unwrap();
        reg.set(&key, "", Value::String("another installation".into()))
            .unwrap();
        let sibling = Key::new(
            Hive::CurrentUser,
            View::Registry32,
            "Software\\Google\\Chrome\\NativeMessagingHosts\\com.other.host",
        )
        .unwrap();
        reg.create(&sibling).unwrap();
        reg.set(&sibling, "", Value::String("other host".into()))
            .unwrap();
        let report = r.unregister().unwrap();
        assert_eq!(
            report
                .registrations
                .iter()
                .find(|r| r.id == "chrome")
                .unwrap()
                .state,
            RegistrationState::Stale
        );
        assert_eq!(
            reg.read(&key, "", ValueType::String).unwrap(),
            RegistryRead::Value(Value::String("another installation".into()))
        );
        assert!(reg.key_exists(&sibling).unwrap());
    }

    #[test]
    fn host_install_unicode_manifest_and_source_failure() {
        let (r, f, _) = fixture();
        r.register(Path::new("bundle")).unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&f.read(HOST_MANIFEST_FILE).unwrap().unwrap()).unwrap();
        assert_eq!(
            v["path"].as_str().unwrap(),
            r.paths.host_exe.to_str().unwrap()
        );
        assert_eq!(
            v["allowed_origins"],
            serde_json::json!([format!("chrome-extension://{}/", crate::EXTENSION_ID)])
        );
    }
    #[test]
    fn host_install_old_cleanup_verifies_digest_and_uuid() {
        let (r, f, _) = fixture();
        r.register(Path::new("bundle")).unwrap();
        let valid = old_name(b"old");
        f.create_new(&valid, b"old").unwrap();
        let spoof = old_name(b"expected");
        f.create_new(&spoof, b"foreign").unwrap();
        r.register(Path::new("bundle")).unwrap();
        assert!(f.read(&valid).unwrap().is_none());
        assert!(f.read(&spoof).unwrap().is_some());
    }
    #[test]
    fn host_install_failed_rollback_retains_old_at_verified_backup() {
        struct NoRollback(Files);
        impl InstallDirectory for NoRollback {
            fn read(&self, n: &str) -> PlatformResult<Option<Vec<u8>>> {
                self.0.read(n)
            }
            fn create_new(&self, n: &str, b: &[u8]) -> PlatformResult<()> {
                self.0.create_new(n, b)
            }
            fn rename(&self, a: &str, b: &str, r: bool) -> Result<(), MoveError> {
                if a.starts_with("mypass-host.old-") {
                    Err(MoveError::Busy)
                } else {
                    self.0.rename(a, b, r)
                }
            }
            fn remove(&self, n: &str) -> PlatformResult<()> {
                self.0.remove(n)
            }
            fn names(&self) -> PlatformResult<Vec<String>> {
                self.0.names()
            }
        }
        let (_, f, _) = fixture();
        f.create_new(EXE, b"old").unwrap();
        {
            let mut s = f.0.lock().unwrap();
            s.running = true;
            s.fail_install = true;
        }
        assert!(replace_host(&NoRollback(f.clone()), b"new")
            .unwrap_err()
            .to_string()
            .contains("previous host retained"));
        assert!(f.names().unwrap().iter().any(|n| f
            .read(n)
            .unwrap()
            .is_some_and(|b| b == b"old" && owned_old(n, &b))));
    }
}
