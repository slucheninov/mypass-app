//! The coordinated-files seam (stage 5, ruling R2): the file access a vault in iCloud Drive needs.
//!
//! A vault in iCloud Drive (or any other file provider) may be a placeholder that is not on disk
//! yet, may be rewritten by the sync daemon while we read it, and may grow conflict versions when
//! two Macs saved it while offline. [`CoordinatedFiles`] is the one place that talks to the
//! system's file coordination for that: coordinated reads and writes, the download state of the
//! item, a download request, the unresolved conflict versions, and a presenter that reports
//! changes made by other processes.
//!
//! The trait speaks `std` types only (`Path`, `io::Result`, `SystemTime`), so `mypass-session`
//! can use it without this crate depending on `mypass-kdbx` (and the other way round).
//!
//! Bodies: macOS uses `NSFileCoordinator`, `NSFileVersion`, the ubiquity resource keys and an
//! `NSFilePresenter` (`macos_icloud.rs`); every other OS gets [`UnsupportedCoordinatedFiles`],
//! where plain access *is* the coordinated access (there is no coordinator to talk to) and nothing
//! is ever ubiquitous. Tests use `testing::FakeCoordinatedFiles`.
//!
//! Timing is the caller's: nothing here waits for a download or polls. The caller asks for the
//! download state, requests the download and re-checks the state on its own scheduler.

use crate::Subscription;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Where the bytes of an item are, as far as the file provider knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    /// A plain local file, not managed by a file provider: always readable.
    NotUbiquitous,
    /// A placeholder: the bytes are not on this Mac; request a download first.
    NotDownloaded,
    /// On disk, but a newer version exists in the cloud.
    Downloaded,
    /// On disk and the newest version.
    Current,
}

impl DownloadState {
    /// The bytes on disk may be read now (possibly an older version for `Downloaded`).
    pub fn is_local(self) -> bool {
        !matches!(self, Self::NotDownloaded)
    }
}

/// The raw value of `NSURLUbiquitousItemDownloadingStatusNotDownloaded` (any status other than
/// the two below maps the same way, so only the tests name it).
#[cfg(test)]
pub const STATUS_NOT_DOWNLOADED: &str = "NSURLUbiquitousItemDownloadingStatusNotDownloaded";
/// The raw value of `NSURLUbiquitousItemDownloadingStatusDownloaded`.
pub const STATUS_DOWNLOADED: &str = "NSURLUbiquitousItemDownloadingStatusDownloaded";
/// The raw value of `NSURLUbiquitousItemDownloadingStatusCurrent`.
pub const STATUS_CURRENT: &str = "NSURLUbiquitousItemDownloadingStatusCurrent";

/// Maps the two ubiquity resource values to a [`DownloadState`]. A ubiquitous item with a missing
/// or unknown status counts as not downloaded: asking for a download that is not needed is
/// harmless, reading a placeholder is not.
pub fn map_download_status(is_ubiquitous: bool, status: Option<&str>) -> DownloadState {
    if !is_ubiquitous {
        return DownloadState::NotUbiquitous;
    }
    match status {
        Some(STATUS_CURRENT) => DownloadState::Current,
        Some(STATUS_DOWNLOADED) => DownloadState::Downloaded,
        _ => DownloadState::NotDownloaded,
    }
}

/// One unresolved conflict version of an item (`NSFileVersion`): another device's save that lost
/// the race. The caller reads it at `path`, merges it, writes the result, and then passes it back
/// to [`CoordinatedFiles::mark_conflicts_resolved`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictVersion {
    /// Where the bytes of this version can be read (inside the system's version store).
    pub path: PathBuf,
    /// When the version was saved, if known.
    pub modified: Option<SystemTime>,
    /// The name of the computer that saved it, if known (for the UI only).
    pub saving_computer: Option<String>,
}

/// Called with the path to actually read or write; the path may differ from the one asked for
/// (the coordinator may hand out a different URL for the same item).
pub type Accessor<'a> = &'a mut dyn FnMut(&Path) -> io::Result<()>;

/// Called (on a background queue) when another process changed the presented item or a new
/// version of it appeared. It must not block; it only hands the event on.
pub type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

/// Coordinated file access to a vault that may live in iCloud Drive. See the module docs.
pub trait CoordinatedFiles: Send + Sync {
    /// Runs `accessor` under a coordinated read of `path` and returns its result. Waits for other
    /// writers (the sync daemon included); a placeholder is downloaded first, so callers check
    /// [`Self::download_state`] beforehand when they must not block.
    fn coordinated_read(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()>;

    /// Runs `accessor` under a coordinated write of `path` that replaces the item (the atomic
    /// rename of a save) and returns its result.
    fn coordinated_write(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()>;

    /// The current download state of `path`, read fresh (never a cached value). A missing file
    /// is `ErrorKind::NotFound`.
    fn download_state(&self, path: &Path) -> io::Result<DownloadState>;

    /// Asks the file provider to download `path`. Returns at once; the caller re-checks
    /// [`Self::download_state`] on its own schedule.
    fn request_download(&self, path: &Path) -> io::Result<()>;

    /// The unresolved conflict versions of `path`, oldest first as the system lists them; empty
    /// for a plain file.
    fn unresolved_conflicts(&self, path: &Path) -> io::Result<Vec<ConflictVersion>>;

    /// Marks exactly `versions` (as returned by [`Self::unresolved_conflicts`]) as resolved. A
    /// version that appeared after that listing is left alone, so it is never resolved unmerged.
    /// The system purges resolved versions from its version store itself.
    fn mark_conflicts_resolved(&self, path: &Path, versions: &[ConflictVersion]) -> io::Result<()>;

    /// Whether an iCloud account is signed in and iCloud Drive is on (for the UI only).
    fn icloud_available(&self) -> bool;

    /// Reports changes to `path` made by other processes (and new versions of it) to
    /// `on_change` until the returned subscription is dropped.
    fn present(&self, path: &Path, on_change: ChangeCallback) -> io::Result<Subscription>;
}

/// The coordinated-files service of the running OS. Not a [`crate::Platform`] field (that struct
/// keeps its final shape): only the vault-file I/O of the session uses it.
pub fn coordinated_files() -> Arc<dyn CoordinatedFiles> {
    #[cfg(target_os = "macos")]
    return Arc::new(crate::macos_icloud::MacCoordinatedFiles);
    #[cfg(not(target_os = "macos"))]
    return Arc::new(UnsupportedCoordinatedFiles);
}

/// The body for an OS without file coordination: reads and writes go straight to the file, no
/// item is ever ubiquitous or in conflict, and there is no presenter (`present` returns
/// `ErrorKind::Unsupported`, so the caller keeps its own change detection).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedCoordinatedFiles;

impl CoordinatedFiles for UnsupportedCoordinatedFiles {
    fn coordinated_read(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()> {
        accessor(path)
    }

    fn coordinated_write(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()> {
        accessor(path)
    }

    fn download_state(&self, path: &Path) -> io::Result<DownloadState> {
        std::fs::metadata(path).map(|_| DownloadState::NotUbiquitous)
    }

    fn request_download(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported("download requests"))
    }

    fn unresolved_conflicts(&self, _path: &Path) -> io::Result<Vec<ConflictVersion>> {
        Ok(Vec::new())
    }

    fn mark_conflicts_resolved(
        &self,
        _path: &Path,
        versions: &[ConflictVersion],
    ) -> io::Result<()> {
        if versions.is_empty() {
            Ok(())
        } else {
            Err(unsupported("conflict versions"))
        }
    }

    fn icloud_available(&self) -> bool {
        false
    }

    fn present(&self, _path: &Path, _on_change: ChangeCallback) -> io::Result<Subscription> {
        Err(unsupported("file presenters"))
    }
}

fn unsupported(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{what} are not supported on this platform"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_status_mapping_table() {
        let cases = [
            (false, None, DownloadState::NotUbiquitous),
            (false, Some(STATUS_CURRENT), DownloadState::NotUbiquitous),
            (true, Some(STATUS_CURRENT), DownloadState::Current),
            (true, Some(STATUS_DOWNLOADED), DownloadState::Downloaded),
            (
                true,
                Some(STATUS_NOT_DOWNLOADED),
                DownloadState::NotDownloaded,
            ),
            (true, Some("SomethingNew"), DownloadState::NotDownloaded),
            (true, None, DownloadState::NotDownloaded),
        ];
        for (ubiquitous, status, expected) in cases {
            assert_eq!(
                map_download_status(ubiquitous, status),
                expected,
                "{ubiquitous} {status:?}"
            );
        }
    }

    #[test]
    fn only_a_placeholder_is_not_local() {
        assert!(DownloadState::NotUbiquitous.is_local());
        assert!(DownloadState::Downloaded.is_local());
        assert!(DownloadState::Current.is_local());
        assert!(!DownloadState::NotDownloaded.is_local());
    }

    #[test]
    fn unsupported_body_passes_reads_and_writes_through() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        let files = UnsupportedCoordinatedFiles;
        files
            .coordinated_write(&path, &mut |p| std::fs::write(p, b"bytes"))
            .unwrap();
        let mut read = Vec::new();
        files
            .coordinated_read(&path, &mut |p| {
                read = std::fs::read(p)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(read, b"bytes");
        assert_eq!(
            files.download_state(&path).unwrap(),
            DownloadState::NotUbiquitous
        );
        assert!(files.unresolved_conflicts(&path).unwrap().is_empty());
        files.mark_conflicts_resolved(&path, &[]).unwrap();
        assert!(!files.icloud_available());
    }

    #[test]
    fn unsupported_body_reports_what_it_cannot_do() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing.kdbx");
        let files = UnsupportedCoordinatedFiles;
        assert_eq!(
            files.download_state(&missing).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            files.request_download(&missing).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        let version = ConflictVersion {
            path: missing.clone(),
            modified: None,
            saving_computer: None,
        };
        assert_eq!(
            files
                .mark_conflicts_resolved(&missing, &[version])
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
        let err = files.present(&missing, Arc::new(|| {})).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn accessor_errors_are_returned_unchanged() {
        let files = UnsupportedCoordinatedFiles;
        let err = files
            .coordinated_read(Path::new("x"), &mut |_| {
                Err(io::Error::new(io::ErrorKind::TimedOut, "slow"))
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
