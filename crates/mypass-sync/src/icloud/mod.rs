//! iCloud Drive as vault storage (stage 5, T6 and the sync half of T7; rulings R2, R17, R19, R20,
//! R38).
//!
//! This module holds the portable, pure parts: where iCloud Drive is ([`ICloudDriveLocation`],
//! [`vault_location_of`]), the legacy placeholder name, waiting for a download
//! ([`ensure_downloaded`] over a [`DownloadProbe`] and a [`Clock`]) and merging the conflict
//! versions of a file ([`resolve_conflicts`] over [`ConflictVersions`]). [`ICloudFileIo`]
//! (`file_io.rs`) puts them together as the `FileIo` of a vault in iCloud Drive, over the
//! `mypass-platform` [`CoordinatedFiles`] seam, so every rule here runs (and is tested) on any OS
//! against the platform's fake.
//!
//! One home per name (R19): `vault_location_of`, `ICloudDriveLocation`, `DownloadProbe` and
//! `ConflictVersions` live here; the session's `storage.rs` holds the status and the resolver.
//! [`DownloadState`] and [`ConflictVersion`] are the platform's types.

#[cfg(test)]
mod tests;

use mypass_kdbx::{Database, FileIo, KdbxError, PlainFileIo, MAX_VAULT_FILE_BYTES};
pub use mypass_platform::{ConflictVersion, CoordinatedFiles, DownloadState};
use secrecy::SecretString;
use serde::Serialize;
use std::{
    fmt, io,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

/// How long a read waits for a vault that is not downloaded yet (R17: then `TimedOut`).
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// How often [`ensure_downloaded`] asks for the download state while it waits.
pub const DOWNLOAD_POLL: Duration = Duration::from_millis(200);
/// The folder name of iCloud Drive itself inside `~/Library/Mobile Documents`.
pub const ICLOUD_DRIVE_CONTAINER: &str = "com~apple~CloudDocs";

/// Where iCloud Drive lives in a home folder.
#[derive(Debug, Clone, Copy, Default)]
pub struct ICloudDriveLocation;

impl ICloudDriveLocation {
    /// `<home>/Library/Mobile Documents/com~apple~CloudDocs`: the folder Finder shows as
    /// "iCloud Drive".
    pub fn root(home: &Path) -> PathBuf {
        Self::mobile_documents(home).join(ICLOUD_DRIVE_CONTAINER)
    }

    /// `<home>/Library/Mobile Documents`: every ubiquity container of the account (iCloud Drive
    /// itself and the per-app folders that Finder also shows inside iCloud Drive).
    pub fn mobile_documents(home: &Path) -> PathBuf {
        home.join("Library").join("Mobile Documents")
    }

    /// Whether `path` is [`ICloudDriveLocation::root`] or inside it. The comparison is by path
    /// component (so `com~apple~CloudDocs2` is not inside), made both as written (with `.` and
    /// `..` folded) and with symlinks resolved where the path exists, so a link into iCloud
    /// Drive is caught.
    pub fn is_inside(path: &Path, home: &Path) -> bool {
        starts_within(path, &Self::root(home))
    }

    /// Whether `path` is inside any ubiquity container (`<home>/Library/Mobile Documents`), with
    /// the same comparison as [`ICloudDriveLocation::is_inside`].
    pub fn is_ubiquitous(path: &Path, home: &Path) -> bool {
        starts_within(path, &Self::mobile_documents(home))
    }
}

fn starts_within(path: &Path, base: &Path) -> bool {
    lexical(path).starts_with(lexical(base)) || resolved(path).starts_with(resolved(base))
}

/// `path` with `.` dropped and `..` folded, without touching the file system.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `path` with symlinks resolved: the whole path when it exists, else its parent plus the name
/// (a file about to be created), else the lexical form.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(real) = std::fs::canonicalize(parent) {
            return real.join(name);
        }
    }
    lexical(path)
}

/// Where a vault file is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum VaultLocation {
    /// A plain folder on this computer (or a drive the OS mounts as one).
    Local,
    /// iCloud Drive (any ubiquity container under `~/Library/Mobile Documents`).
    ICloudDrive,
}

/// Where the vault at `path` is kept. Every ubiquity container counts as iCloud Drive, not only
/// `com~apple~CloudDocs`: the per-app folders sync the same way and need the same coordinated
/// access.
pub fn vault_location_of(path: &Path, home: &Path) -> VaultLocation {
    if ICloudDriveLocation::is_ubiquitous(path, home) {
        VaultLocation::ICloudDrive
    } else {
        VaultLocation::Local
    }
}

/// The name of the legacy placeholder iCloud Drive leaves for an evicted file:
/// `vault.kdbx` -> `.vault.kdbx.icloud`.
pub fn placeholder_name(file_name: &str) -> String {
    format!(".{file_name}.icloud")
}

/// The legacy placeholder path of `path` (same folder), if `path` has a file name.
pub fn placeholder_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy();
    Some(path.with_file_name(placeholder_name(&name)))
}

/// Whether the vault at `path` exists in iCloud's eyes: the file itself or its legacy
/// placeholder. A placeholder is never "missing": reading it waits for the download.
pub fn exists_or_placeholder(path: &Path) -> bool {
    path.exists() || placeholder_path(path).is_some_and(|p| p.exists())
}

/// Why an iCloud Drive access failed. Codes only, never content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ICloudError {
    /// The file is still a placeholder after the download timeout (R17: surfaces as
    /// `ErrorKind::TimedOut`, never as a missing file and never as the cached copy).
    #[error("the vault did not download from iCloud in time")]
    DownloadTimedOut,
    /// The folder of the vault is gone (iCloud Drive turned off, or the folder removed).
    #[error("the iCloud Drive folder is unavailable")]
    FolderUnavailable,
    /// No iCloud account is signed in.
    #[error("no iCloud account is signed in")]
    NotSignedIn,
    /// Any other file-system failure.
    #[error("i/o error: {message}")]
    Io {
        kind: io::ErrorKind,
        message: String,
    },
}

impl ICloudError {
    fn io(e: &io::Error) -> Self {
        Self::Io {
            kind: e.kind(),
            message: e.to_string(),
        }
    }

    /// The `io::ErrorKind` the `FileIo` layer reports for this error.
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            Self::DownloadTimedOut => io::ErrorKind::TimedOut,
            Self::FolderUnavailable | Self::NotSignedIn => io::ErrorKind::NotFound,
            Self::Io { kind, .. } => *kind,
        }
    }
}

impl From<ICloudError> for io::Error {
    fn from(e: ICloudError) -> Self {
        io::Error::new(e.kind(), e.to_string())
    }
}

impl From<ICloudError> for KdbxError {
    fn from(e: ICloudError) -> Self {
        KdbxError::Io {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

/// The download state of a file and a way to ask for its download (R19). Production uses
/// [`CoordinatedProbe`] over the platform's [`CoordinatedFiles`].
pub trait DownloadProbe {
    /// The current download state; a missing file is `ErrorKind::NotFound`.
    fn state(&self, path: &Path) -> io::Result<DownloadState>;
    /// Asks for the download; returns at once.
    fn request_download(&self, path: &Path) -> io::Result<()>;
}

/// [`DownloadProbe`] over the platform's [`CoordinatedFiles`].
#[derive(Clone, Copy)]
pub struct CoordinatedProbe<'a>(pub &'a dyn CoordinatedFiles);

impl DownloadProbe for CoordinatedProbe<'_> {
    fn state(&self, path: &Path) -> io::Result<DownloadState> {
        self.0.download_state(path)
    }
    fn request_download(&self, path: &Path) -> io::Result<()> {
        self.0.request_download(path)
    }
}

impl fmt::Debug for CoordinatedProbe<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CoordinatedProbe")
    }
}

/// The time source of [`ensure_downloaded`]; tests advance a fake one, so no test waits.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// The download state of `path`, where a missing file with a legacy placeholder next to it is
/// [`DownloadState::NotDownloaded`] (never missing).
fn state_of(probe: &dyn DownloadProbe, path: &Path) -> Result<DownloadState, ICloudError> {
    match probe.state(path) {
        Ok(state) => Ok(state),
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                && placeholder_path(path).is_some_and(|p| p.exists()) =>
        {
            Ok(DownloadState::NotDownloaded)
        }
        Err(e) => Err(ICloudError::io(&e)),
    }
}

/// Makes sure the bytes of `path` are on this Mac before it is read.
///
/// - `NotUbiquitous` and `Current` return at once (no request).
/// - `Downloaded` (the bytes on disk are an older version) asks for the newer one and returns at
///   once: the local bytes are a valid vault, and when the newer version lands the presenter and
///   the stamp check merge it (waiting here would stall every save while offline).
/// - `NotDownloaded` (a placeholder, also a legacy `.name.icloud` one) asks for the download once
///   and polls every [`DOWNLOAD_POLL`] until the bytes are local; after `timeout`
///   [`ICloudError::DownloadTimedOut`].
pub fn ensure_downloaded(
    probe: &dyn DownloadProbe,
    clock: &dyn Clock,
    path: &Path,
    timeout: Duration,
) -> Result<(), ICloudError> {
    match state_of(probe, path)? {
        DownloadState::NotUbiquitous | DownloadState::Current => return Ok(()),
        DownloadState::Downloaded => {
            // Best effort: the local version is readable either way.
            let _ = probe.request_download(path);
            return Ok(());
        }
        DownloadState::NotDownloaded => {}
    }
    probe
        .request_download(path)
        .map_err(|e| ICloudError::io(&e))?;
    let start = clock.now();
    loop {
        let elapsed = clock.now().saturating_duration_since(start);
        let Some(remaining) = timeout.checked_sub(elapsed).filter(|d| !d.is_zero()) else {
            return Err(ICloudError::DownloadTimedOut);
        };
        clock.sleep(DOWNLOAD_POLL.min(remaining));
        if state_of(probe, path)?.is_local() && path.exists() {
            return Ok(());
        }
    }
}

/// The unresolved conflict versions of a file (R19; production: `NSFileVersion` through the
/// platform seam, plus the offline cache of [`ICloudFileIo`]).
pub trait ConflictVersions {
    /// The versions not resolved yet; empty for a file without conflicts.
    fn unresolved(&self, path: &Path) -> io::Result<Vec<ConflictVersion>>;
    /// The bytes of one version, bounded (R20).
    fn read_version(&self, version: &ConflictVersion) -> io::Result<Vec<u8>>;
    /// Marks exactly `versions` resolved (called only after the merge of all of them is on
    /// disk).
    fn mark_resolved(&self, path: &Path, versions: &[ConflictVersion]) -> io::Result<()>;
}

/// What [`resolve_conflicts`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConflictOutcome {
    /// Versions merged into the file and marked resolved.
    pub merged: usize,
    /// Versions left unresolved: unreadable, or the merge refused them (another password, not a
    /// vault). They stay in the version store for a later attempt or for the user.
    pub unresolved: usize,
}

impl ConflictOutcome {
    /// Whether the file was rewritten.
    pub fn changed(&self) -> bool {
        self.merged > 0
    }
}

/// The merge step of [`resolve_conflicts`]: `(current, version) -> merged bytes`.
pub type MergeFn<'a> = &'a mut dyn FnMut(&[u8], &[u8]) -> io::Result<Vec<u8>>;

/// Merges every unresolved conflict version of `path` into `current` and writes the result once.
///
/// For each version, in the order listed: its bytes are read (an unreadable version is skipped
/// and stays unresolved) and `acc = merge(acc, version)` (a refused merge, for example a version
/// under another password, leaves that version unresolved). When at least one version merged,
/// `write_current(acc)` runs once, and only after it succeeded are exactly the merged versions
/// marked resolved: a crash in between loses nothing, the versions are still there and merge
/// again (the merge is idempotent). No conflicts is a no-op that writes nothing.
pub fn resolve_conflicts(
    src: &dyn ConflictVersions,
    path: &Path,
    current: Vec<u8>,
    merge: MergeFn<'_>,
    write_current: &mut dyn FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<ConflictOutcome> {
    let versions = src.unresolved(path)?;
    let mut outcome = ConflictOutcome::default();
    if versions.is_empty() {
        return Ok(outcome);
    }
    let mut acc = current;
    let mut merged = Vec::new();
    for version in versions {
        let Ok(bytes) = src.read_version(&version) else {
            outcome.unresolved += 1;
            continue;
        };
        match merge(&acc, &bytes) {
            Ok(next) => {
                acc = next;
                merged.push(version);
            }
            Err(_) => outcome.unresolved += 1,
        }
    }
    if merged.is_empty() {
        return Ok(outcome);
    }
    write_current(&acc)?;
    src.mark_resolved(path, &merged)?;
    outcome.merged = merged.len();
    Ok(outcome)
}

/// The production merge step: opens both blobs with `password`, merges the version into the
/// current file with the merge engine (`mypass_core::merge::merge`, which keeps the current
/// file's metadata and save settings: cipher, KDF and its parameters) and encodes the result
/// with the same password. A blob that does not open (another password, not a vault) is an
/// error, which leaves that version unresolved. Never logs or returns content in errors.
pub fn merge_vault_bytes(
    current: &[u8],
    version: &[u8],
    password: &SecretString,
) -> io::Result<Vec<u8>> {
    let current = Database::open(current, password).map_err(merge_error)?;
    let version = Database::open(version, password).map_err(merge_error)?;
    mypass_core::merge::merge(&current, &version)
        .save(password)
        .map_err(merge_error)
}

fn merge_error(e: KdbxError) -> io::Error {
    match e {
        KdbxError::Io { kind, message } => io::Error::new(kind, message),
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

/// [`ConflictVersions`] over the platform's `NSFileVersion` listing; version bytes are read
/// bounded and regular-file only (R20).
pub struct PlatformVersions<'a>(pub &'a dyn CoordinatedFiles);

impl ConflictVersions for PlatformVersions<'_> {
    fn unresolved(&self, path: &Path) -> io::Result<Vec<ConflictVersion>> {
        self.0.unresolved_conflicts(path)
    }
    fn read_version(&self, version: &ConflictVersion) -> io::Result<Vec<u8>> {
        PlainFileIo
            .read_limited(&version.path, MAX_VAULT_FILE_BYTES)
            .map_err(kdbx_to_io)
    }
    fn mark_resolved(&self, path: &Path, versions: &[ConflictVersion]) -> io::Result<()> {
        self.0.mark_conflicts_resolved(path, versions)
    }
}

pub(crate) fn kdbx_to_io(e: KdbxError) -> io::Error {
    match e {
        KdbxError::Io { kind, message } => io::Error::new(kind, message),
        other => io::Error::other(other.to_string()),
    }
}
