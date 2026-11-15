//! [`ICloudFileIo`]: the `FileIo` of a vault kept in iCloud Drive (stage 5, T7; rulings R17, R20,
//! R37, R38, R39).
//!
//! - **Coordinated access.** Every read and write of the vault goes through the platform's file
//!   coordinator ([`CoordinatedFiles`]), bounded by [`COORDINATION_TIMEOUT`]; the bytes inside the
//!   accessor are read bounded and regular-file only (`MAX_VAULT_FILE_BYTES`, R20) and written
//!   through `PlainFileIo`. Primary saves recheck the expected digest inside the write
//!   accessor and preserve racing copies with guarded replacement.
//! - **Downloads.** A vault that is still a placeholder (also the legacy `.name.icloud` one) is
//!   downloaded first and waited for at most [`DOWNLOAD_TIMEOUT`]; then the read fails with
//!   `ErrorKind::TimedOut` (R17), never `NotFound` and never the cached copy.
//! - **Offline cache and fallback.** A copy of the last bytes read or written is kept at
//!   `<cache dir>/<sha256 of the vault path>.kdbx`. When the vault's folder is gone (iCloud Drive
//!   turned off, signed out, the folder removed) reads and writes use that copy. A write to the
//!   cache first records the cache as *ahead* (`<cache>.state`, written atomically and never
//!   removed), and while it is ahead every read keeps coming from the cache, also after the
//!   folder came back: a clean session would otherwise adopt the iCloud file wholesale and drop
//!   the offline edits. [`ICloudFileIo::resolve_conflicts`] merges the ahead cache (and every
//!   `NSFileVersion` conflict version) into the real file with the merge engine; only once that
//!   merge is on disk is the cache marked not ahead. Nothing here ever deletes a file (R38): the
//!   cache is overwritten, never removed, and the vault is only ever replaced atomically.

#[cfg(test)]
#[path = "file_io_tests.rs"]
mod tests;

use super::{
    ensure_downloaded, kdbx_to_io, placeholder_path, resolve_conflicts as merge_versions, resolved,
    Clock, ConflictOutcome, ConflictVersions, CoordinatedProbe, ICloudError, MergeFn,
    PlatformVersions, SystemClock, DOWNLOAD_TIMEOUT,
};
use mypass_kdbx::{
    FileIo, FileMeta, KdbxError, PlainFileIo, ReplaceOutcome, Result, SaveOutcome,
    MAX_VAULT_FILE_BYTES,
};
use mypass_platform::{ChangeCallback, ConflictVersion, CoordinatedFiles, Subscription};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, PoisonError, Weak,
    },
    time::Duration,
};

/// How long a coordinated read or write waits for another process's access (then `TimedOut`).
pub const COORDINATION_TIMEOUT: Duration = Duration::from_secs(10);
/// The suffix of the file that records whether the offline cache is ahead of iCloud.
pub const CACHE_STATE_SUFFIX: &str = ".state";
/// The folder of the offline caches inside the app's data folder.
pub const ICLOUD_CACHE_DIR: &str = "icloud-cache";

/// Maximum size of the two fixed cache-state records.
pub const MAX_CACHE_STATE_BYTES: u64 = 32;

const STATE_AHEAD: &[u8] = b"{\"ahead\":true}\n";
const STATE_SYNCED: &[u8] = b"{\"ahead\":false}\n";

/// What the UI and the session need to know about an iCloud vault's storage. Codes only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoHealth {
    /// The vault is served from the offline cache: the folder is unavailable, or the cache holds
    /// edits not merged into iCloud yet.
    pub fallback_active: bool,
    /// The cache holds edits that are not in iCloud yet.
    pub cache_ahead: bool,
    /// The last read timed out waiting for the download.
    pub downloading: bool,
}

/// The `FileIo` of one vault in iCloud Drive. See the module docs.
pub struct ICloudFileIo {
    coordinated: Arc<dyn CoordinatedFiles>,
    /// The vault path with symlinks resolved where it exists (the key of the platform calls).
    vault: PathBuf,
    cache: PathBuf,
    cache_gate: Arc<Mutex<()>>,
    clock: Arc<dyn Clock>,
    download_timeout: Duration,
    coordination_timeout: Duration,
    served_from_cache: AtomicBool,
    downloading: AtomicBool,
    presenter: Mutex<Option<Subscription>>,
}

impl fmt::Debug for ICloudFileIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ICloudFileIo")
            .field("vault", &self.vault)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl ICloudFileIo {
    /// The `FileIo` of the vault at `vault`, caching its bytes under `cache_dir` (normally
    /// `<data dir>/icloud-cache`), with the system clock and the default timeouts.
    pub fn new(coordinated: Arc<dyn CoordinatedFiles>, vault: &Path, cache_dir: &Path) -> Self {
        let vault = resolved(vault);
        let cache = Self::cache_path_for(cache_dir, &vault);
        Self {
            cache_gate: cache_gate(&cache),
            coordinated,
            vault,
            cache,
            clock: Arc::new(SystemClock),
            download_timeout: DOWNLOAD_TIMEOUT,
            coordination_timeout: COORDINATION_TIMEOUT,
            served_from_cache: AtomicBool::new(false),
            downloading: AtomicBool::new(false),
            presenter: Mutex::new(None),
        }
    }

    /// Creates a primary vault only when no primary, placeholder or offline copy exists.
    /// The no-replace install runs inside coordination and shares the cache publication gate.
    pub fn create_new(&self, bytes: &[u8]) -> Result<()> {
        let mut out = Err(io_kdbx(io::Error::other(
            "the coordinator skipped creation",
        )));
        // Refuse cached vault state even when the remote folder is unavailable.
        let exists = || {
            fs::symlink_metadata(&self.cache).is_ok()
                || fs::symlink_metadata(self.state_path()).is_ok()
                || fs::symlink_metadata(&self.vault).is_ok()
                || placeholder_path(&self.vault).is_some_and(|p| fs::symlink_metadata(p).is_ok())
        };
        {
            let _gate = self
                .cache_gate
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if exists() {
                return Err(io_kdbx(io::ErrorKind::AlreadyExists.into()));
            }
        }
        self.coordinated
            .coordinated_write(&self.vault, self.coordination_timeout, &mut |p| {
                let _gate = self
                    .cache_gate
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                out = if exists() {
                    Err(io_kdbx(io::ErrorKind::AlreadyExists.into()))
                } else {
                    match PlainFileIo.replace_if(p, None, bytes, MAX_VAULT_FILE_BYTES) {
                        Ok(ReplaceOutcome::Replaced) => Ok(()),
                        Ok(_) => Err(io_kdbx(io::ErrorKind::AlreadyExists.into())),
                        Err(e) => Err(e),
                    }
                };
                Ok(())
            })
            .map_err(io_kdbx)?;
        out?;
        self.refresh_cache(bytes);
        Ok(())
    }

    /// Replaces the clock (tests advance a fake one).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replaces the download and coordination timeouts.
    pub fn with_timeouts(mut self, download: Duration, coordination: Duration) -> Self {
        self.download_timeout = download;
        self.coordination_timeout = coordination;
        self
    }

    /// The cache file of the vault at `vault`: `<cache_dir>/<hex sha256 of the resolved path>.kdbx`.
    /// Keyed by path, not by vault id, so it is known before the first open and two copies of
    /// one vault never share a cache.
    pub fn cache_path_for(cache_dir: &Path, vault: &Path) -> PathBuf {
        let key = Sha256::digest(resolved(vault).to_string_lossy().as_bytes());
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        cache_dir.join(format!("{hex}.kdbx"))
    }

    /// The vault path this `FileIo` serves (symlinks resolved where it exists).
    pub fn vault_path(&self) -> &Path {
        &self.vault
    }

    /// The offline cache file.
    pub fn cache_path(&self) -> &Path {
        &self.cache
    }

    /// The current storage health.
    pub fn health(&self) -> IoHealth {
        let cache_ahead = self.cache_ahead();
        IoHealth {
            fallback_active: cache_ahead || self.served_from_cache.load(Ordering::SeqCst),
            cache_ahead,
            downloading: self.downloading.load(Ordering::SeqCst),
        }
    }

    /// Whether an iCloud account is signed in (status wording only; the fallback decision is
    /// made by the folder, see [`ICloudFileIo::folder_available`]).
    pub fn icloud_signed_in(&self) -> bool {
        self.coordinated.icloud_available()
    }

    /// Registers `on_change` as the file presenter of the vault: it runs (on a platform thread)
    /// whenever another process changes the file. Replaces (and so unregisters) an earlier one.
    /// The callback must not block; the session debounces it onto its scheduler.
    pub fn set_on_remote_change(&self, on_change: ChangeCallback) -> io::Result<()> {
        let subscription = self.coordinated.present(&self.vault, on_change)?;
        *self
            .presenter
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(subscription);
        Ok(())
    }

    /// Unregisters the presenter, if any.
    pub fn clear_on_remote_change(&self) {
        let old = self
            .presenter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(old);
    }

    /// Whether the vault's folder is there. `NotFound`, `PermissionDenied` (iCloud Drive turned
    /// off can leave the container unreadable) and a non-folder are "unavailable"; any other
    /// error propagates.
    pub fn folder_available(&self) -> io::Result<bool> {
        let Some(parent) = self.vault.parent() else {
            return Ok(false);
        };
        match fs::metadata(parent) {
            Ok(meta) => Ok(meta.is_dir()),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::PermissionDenied
                        | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Merges the unresolved conflict versions of the vault, and the offline cache when it is
    /// ahead, into the real file, inside one coordinated write: the current bytes are read, every
    /// version is merged with `merge(current, version)` (a refused one stays unresolved), the
    /// result is written atomically once, and only then are the merged versions marked resolved
    /// (the cache: rewritten with the merged bytes, then marked not ahead). A no-op returning the
    /// empty outcome while the folder is unavailable or when there is nothing to merge. Never
    /// removes a file (R38).
    pub fn resolve_conflicts(&self, merge: MergeFn<'_>) -> io::Result<ConflictOutcome> {
        if !self.folder_available()? {
            return Ok(ConflictOutcome::default());
        }
        let ahead = self.cache_ahead();
        let platform = self.coordinated.unresolved_conflicts(&self.vault)?;
        if !ahead && platform.is_empty() {
            return Ok(ConflictOutcome::default());
        }
        let mut outcome = Ok(ConflictOutcome::default());
        self.coordinated.coordinated_write(
            &self.vault,
            self.coordination_timeout,
            &mut |path: &Path| {
                outcome = self.resolve_inside(path, ahead, &mut *merge);
                Ok(())
            },
        )?;
        let outcome = outcome?;
        if outcome.changed() {
            self.served_from_cache.store(false, Ordering::SeqCst);
        }
        Ok(outcome)
    }

    fn resolve_inside(
        &self,
        path: &Path,
        ahead: bool,
        merge: MergeFn<'_>,
    ) -> io::Result<ConflictOutcome> {
        self.wait_for_download(path).map_err(io::Error::from)?;
        let current = PlainFileIo
            .read_limited(path, MAX_VAULT_FILE_BYTES)
            .map_err(kdbx_to_io)?;
        let expected: [u8; 32] = Sha256::digest(&current).into();
        let source = Versions {
            io: self,
            ahead,
            written: Mutex::new(None),
            cached: Mutex::new(None),
        };
        let mut write = |bytes: &[u8]| {
            match PlainFileIo
                .replace_if(path, Some(&expected), bytes, MAX_VAULT_FILE_BYTES)
                .map_err(kdbx_to_io)?
            {
                ReplaceOutcome::Replaced => {}
                _ => return Err(io::Error::from(io::ErrorKind::WouldBlock)),
            }
            *source
                .written
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(bytes.to_vec());
            Ok(())
        };
        merge_versions(&source, path, current, merge, &mut write)
    }

    /// Whether the cache holds edits not in iCloud yet. A state file that cannot be read or
    /// parsed counts as ahead (merging an up-to-date cache again is harmless).
    fn cache_ahead(&self) -> bool {
        match PlainFileIo.read_limited(&self.state_path(), MAX_CACHE_STATE_BYTES) {
            Ok(bytes) => bytes.as_slice() != STATE_SYNCED,
            Err(KdbxError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            }) => fs::symlink_metadata(self.state_path()).is_ok(),
            Err(_) => true,
        }
    }

    fn state_path(&self) -> PathBuf {
        let mut name = self.cache.as_os_str().to_owned();
        name.push(CACHE_STATE_SUFFIX);
        PathBuf::from(name)
    }

    fn is_vault(&self, path: &Path) -> bool {
        path == self.vault || resolved(path) == self.vault
    }

    /// Whether a vault access goes to the cache: the cache is ahead, or the folder is gone.
    fn use_cache(&self) -> Result<bool> {
        if self.cache_ahead() {
            return Ok(true);
        }
        Ok(!self.folder_available().map_err(io_kdbx)?)
    }

    fn unavailable(&self) -> KdbxError {
        if self.coordinated.icloud_available() {
            ICloudError::FolderUnavailable.into()
        } else {
            ICloudError::NotSignedIn.into()
        }
    }

    fn read_cache(&self, max_len: u64) -> Result<Vec<u8>> {
        let _gate = self
            .cache_gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.served_from_cache.store(true, Ordering::SeqCst);
        match PlainFileIo.read_limited(&self.cache, max_len) {
            Err(KdbxError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            }) => Err(self.unavailable()),
            other => other,
        }
    }

    fn write_cache_ahead(&self, bytes: &[u8]) -> Result<()> {
        self.served_from_cache.store(true, Ordering::SeqCst);
        self.ensure_cache_dir()?;
        // The marker first: a crash between the two leaves an "ahead" cache holding the older
        // bytes, which merges again harmlessly; the reverse order could hide new edits.
        PlainFileIo.write_atomic(&self.state_path(), STATE_AHEAD)?;
        PlainFileIo.write_atomic(&self.cache, bytes)
    }

    /// Best effort: keeps the cache equal to what iCloud holds, while it is not ahead.
    fn refresh_cache(&self, bytes: &[u8]) {
        let _gate = self
            .cache_gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.cache_ahead() {
            return;
        }
        let same = PlainFileIo
            .read_limited(&self.cache, MAX_VAULT_FILE_BYTES)
            .is_ok_and(|cached| cached == bytes);
        if !same && self.ensure_cache_dir().is_ok() {
            let _ = PlainFileIo.write_atomic(&self.cache, bytes);
        }
    }

    fn ensure_cache_dir(&self) -> Result<()> {
        match self.cache.parent() {
            Some(dir) => fs::create_dir_all(dir).map_err(io_kdbx),
            None => Ok(()),
        }
    }

    fn wait_for_download(&self, path: &Path) -> std::result::Result<(), ICloudError> {
        let probe = CoordinatedProbe(self.coordinated.as_ref());
        let result = ensure_downloaded(&probe, self.clock.as_ref(), path, self.download_timeout);
        self.downloading.store(
            matches!(result, Err(ICloudError::DownloadTimedOut)),
            Ordering::SeqCst,
        );
        result
    }

    fn read_coordinated(&self, path: &Path, max_len: u64) -> Result<Vec<u8>> {
        self.wait_for_download(path)?;
        let mut out = None;
        self.coordinated
            .coordinated_read(path, self.coordination_timeout, &mut |p: &Path| {
                out = Some(PlainFileIo.read_limited(p, max_len));
                Ok(())
            })
            .map_err(io_kdbx)?;
        out.unwrap_or_else(|| {
            Err(io_kdbx(io::Error::other(
                "the coordinator skipped the read",
            )))
        })
    }

    fn write_coordinated(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let mut out = None;
        self.coordinated
            .coordinated_write(path, self.coordination_timeout, &mut |p: &Path| {
                out = Some(PlainFileIo.write_atomic(p, bytes));
                Ok(())
            })
            .map_err(io_kdbx)?;
        out.unwrap_or_else(|| {
            Err(io_kdbx(io::Error::other(
                "the coordinator skipped the write",
            )))
        })
    }
}

impl Drop for ICloudFileIo {
    fn drop(&mut self) {
        self.clear_on_remote_change();
    }
}

impl FileIo for ICloudFileIo {
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        self.read_limited(path, MAX_VAULT_FILE_BYTES)
    }

    fn read_limited(&self, path: &Path, max_len: u64) -> Result<Vec<u8>> {
        if !self.is_vault(path) {
            return self.read_coordinated(path, max_len);
        }
        if self.use_cache()? {
            return self.read_cache(max_len);
        }
        let bytes = self.read_coordinated(path, max_len)?;
        self.served_from_cache.store(false, Ordering::SeqCst);
        self.refresh_cache(&bytes);
        Ok(bytes)
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        if !self.is_vault(path) {
            return self.write_coordinated(path, bytes);
        }
        {
            let _gate = self
                .cache_gate
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if self.use_cache()? {
                return self.write_cache_ahead(bytes);
            }
        }
        match self.write_coordinated(path, bytes) {
            Ok(()) => {
                self.served_from_cache.store(false, Ordering::SeqCst);
                self.refresh_cache(bytes);
                Ok(())
            }
            // The folder vanished between the check and the write: keep the bytes offline.
            Err(_) if !self.folder_available().unwrap_or(true) => {
                let _gate = self
                    .cache_gate
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                self.write_cache_ahead(bytes)
            }
            Err(e) => Err(e),
        }
    }

    fn write_checked(&self, path: &Path, expected: &[u8; 32], bytes: &[u8]) -> Result<SaveOutcome> {
        {
            let _gate = self
                .cache_gate
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if self.is_vault(path) && self.use_cache()? {
                let current = PlainFileIo.read_limited(&self.cache, MAX_VAULT_FILE_BYTES)?;
                if <[u8; 32]>::from(Sha256::digest(&current)) != *expected {
                    return Ok(SaveOutcome::ChangedOnDisk);
                }
                self.write_cache_ahead(bytes)?;
                return Ok(SaveOutcome::Saved);
            }
        }
        let mut out = None;
        self.coordinated
            .coordinated_write(path, self.coordination_timeout, &mut |p| {
                let _gate = self
                    .cache_gate
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                out = Some(
                    if self.is_vault(path) && self.use_cache().map_err(kdbx_to_io)? {
                        // Destination changed after the caller's read: retry from the selected copy.
                        Ok(ReplaceOutcome::Conflict)
                    } else {
                        PlainFileIo.replace_if(p, Some(expected), bytes, MAX_VAULT_FILE_BYTES)
                    },
                );
                Ok(())
            })
            .map_err(io_kdbx)?;
        match out.unwrap_or_else(|| {
            Err(io_kdbx(io::Error::other(
                "the coordinator skipped the write",
            )))
        })? {
            ReplaceOutcome::Replaced => {
                self.served_from_cache.store(false, Ordering::SeqCst);
                self.refresh_cache(bytes);
                Ok(SaveOutcome::Saved)
            }
            ReplaceOutcome::Conflict => Ok(SaveOutcome::ChangedOnDisk),
            // The foreign copy remains at `kept`; never acknowledge it as merged.
            ReplaceOutcome::Displaced { kept, .. } => Err(io_kdbx(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "a concurrent vault copy was preserved at {}",
                    kept.display()
                ),
            ))),
        }
    }

    fn metadata(&self, path: &Path) -> Result<FileMeta> {
        if self.is_vault(path) && self.use_cache()? {
            self.served_from_cache.store(true, Ordering::SeqCst);
            return match PlainFileIo.metadata(&self.cache) {
                Err(KdbxError::Io {
                    kind: io::ErrorKind::NotFound,
                    ..
                }) => Err(self.unavailable()),
                other => other,
            };
        }
        match PlainFileIo.metadata(path) {
            // A legacy placeholder is never "missing": wait for the download, then look again.
            Err(KdbxError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            }) if placeholder_path(path).is_some_and(|p| p.exists()) => {
                self.wait_for_download(path)?;
                PlainFileIo.metadata(path)
            }
            other => other,
        }
    }

    fn replace_if(
        &self,
        path: &Path,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
        max_len: u64,
    ) -> Result<ReplaceOutcome> {
        if self.is_vault(path) && self.use_cache()? {
            return Err(self.unavailable());
        }
        let mut out = None;
        self.coordinated
            .coordinated_write(path, self.coordination_timeout, &mut |p: &Path| {
                out = Some(PlainFileIo.replace_if(p, expected, bytes, max_len));
                Ok(())
            })
            .map_err(io_kdbx)?;
        let outcome = out.unwrap_or_else(|| {
            Err(io_kdbx(io::Error::other(
                "the coordinator skipped the write",
            )))
        })?;
        if matches!(outcome, ReplaceOutcome::Replaced) && self.is_vault(path) {
            self.refresh_cache(bytes);
        }
        Ok(outcome)
    }
}

/// The conflict versions [`ICloudFileIo::resolve_conflicts`] merges: the ahead cache (a pseudo
/// version whose path is the cache file) and the platform's `NSFileVersion`s.
struct Versions<'a> {
    io: &'a ICloudFileIo,
    ahead: bool,
    /// The merged bytes once written (the cache is rewritten with them before it is marked not
    /// ahead).
    written: Mutex<Option<Vec<u8>>>,
    cached: Mutex<Option<Vec<u8>>>,
}

impl Versions<'_> {
    fn is_cache(&self, version: &ConflictVersion) -> bool {
        version.path == self.io.cache
    }
}

impl ConflictVersions for Versions<'_> {
    fn unresolved(&self, _path: &Path) -> io::Result<Vec<ConflictVersion>> {
        let mut versions = Vec::new();
        if self.ahead {
            versions.push(ConflictVersion {
                path: self.io.cache.clone(),
                modified: None,
                saving_computer: None,
            });
        }
        versions.extend(self.io.coordinated.unresolved_conflicts(&self.io.vault)?);
        Ok(versions)
    }

    fn read_version(&self, version: &ConflictVersion) -> io::Result<Vec<u8>> {
        if self.is_cache(version) {
            let _gate = self
                .io
                .cache_gate
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let bytes = PlainFileIo
                .read_limited(&version.path, MAX_VAULT_FILE_BYTES)
                .map_err(kdbx_to_io)?;
            *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(bytes.clone());
            return Ok(bytes);
        }
        PlatformVersions(self.io.coordinated.as_ref()).read_version(version)
    }

    fn mark_resolved(&self, _path: &Path, versions: &[ConflictVersion]) -> io::Result<()> {
        let (cache, platform): (Vec<_>, Vec<_>) =
            versions.iter().cloned().partition(|v| self.is_cache(v));
        if !cache.is_empty() {
            let _gate = self
                .io
                .cache_gate
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
            let current = PlainFileIo
                .read_limited(&self.io.cache, MAX_VAULT_FILE_BYTES)
                .map_err(kdbx_to_io)?;
            if cached.as_ref() != Some(&current) {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            let written = self
                .written
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(bytes) = written {
                PlainFileIo
                    .write_atomic(&self.io.cache, &bytes)
                    .map_err(kdbx_to_io)?;
            }
            PlainFileIo
                .write_atomic(&self.io.state_path(), STATE_SYNCED)
                .map_err(kdbx_to_io)?;
        }
        if platform.is_empty() {
            return Ok(());
        }
        self.io
            .coordinated
            .mark_conflicts_resolved(&self.io.vault, &platform)
    }
}

fn io_kdbx(e: io::Error) -> KdbxError {
    KdbxError::Io {
        kind: e.kind(),
        message: e.to_string(),
    }
}

// Independently resolved handles of the same cache share a publication lock. Weak entries do
// not retain a vault handle or any secret-bearing content.
fn cache_gate(path: &Path) -> Arc<Mutex<()>> {
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    gates.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = gates.get(path).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(Mutex::new(()));
    gates.insert(path.to_path_buf(), Arc::downgrade(&gate));
    gate
}
