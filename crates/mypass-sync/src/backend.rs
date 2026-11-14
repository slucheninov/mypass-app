//! The storage seam of the sync engine: one vault file in a place other software also writes to (a
//! secondary folder, iCloud Drive, a migration target).
//!
//! Every remote read is bounded and refuses anything that is not a regular file (R20): the file
//! is untrusted input, and a FIFO, a device or a huge file in a sync folder must not hang the app
//! or exhaust memory. Errors carry codes and paths only, never file bytes (R37).
use mypass_kdbx::{FileIo, KdbxError, MAX_VAULT_FILE_BYTES};
use std::{
    fmt,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

/// Why a storage operation failed. Carries codes and paths only, never file content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    /// The folder is missing, is not a directory or sits on an unplugged volume. Nothing is
    /// created: the backend never makes a missing folder.
    #[error("storage unavailable: {0}")]
    Unavailable(String),
    /// The file or its folder refuses writes.
    #[error("storage is read-only")]
    ReadOnly,
    /// Any other file-system failure.
    #[error("storage i/o error: {0}")]
    Io(String),
    /// The file is not available yet (an iCloud placeholder that did not download in time).
    #[error("storage timed out")]
    TimedOut,
    /// The file is larger than the read limit (`MAX_VAULT_FILE_BYTES` or a lower one).
    #[error("the file is larger than {max_bytes} bytes")]
    TooLarge {
        /// The limit that was exceeded.
        max_bytes: u64,
    },
    /// The path names a directory, FIFO, device or other non-regular file.
    #[error("not a regular file")]
    NotRegularFile,
    /// The folder or file name cannot name a vault file (relative folder, separators in the name).
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
}

/// One vault file in remote storage. `read` returns `Ok(None)` only when the file is missing; every
/// other failure is an error, so a caller never mistakes an unreadable file for an absent one.
pub trait StorageBackend: Send + Sync {
    /// Whether the file exists (a missing folder is `Unavailable`, not `false`).
    fn exists(&self) -> Result<bool, StorageError>;
    /// The file's modification time; `None` when the file is missing or the backend has none.
    fn modification_date(&self) -> Result<Option<SystemTime>, StorageError>;
    /// The file's bytes, bounded and regular-file only; `None` when the file is missing.
    fn read(&self) -> Result<Option<Vec<u8>>, StorageError>;
    /// Replace the file atomically (a reader sees the old or the new bytes, never a mix).
    fn write(&self, bytes: &[u8]) -> Result<(), StorageError>;
}

/// A [`StorageBackend`] over any [`FileIo`] and one path (R20): `LocalFolderBackend` uses it over
/// `PlainFileIo`, the iCloud targets over the iCloud `FileIo`.
pub struct FileIoBackend {
    io: Arc<dyn FileIo>,
    path: PathBuf,
    max_len: u64,
}

impl fmt::Debug for FileIoBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileIoBackend")
            .field("path", &self.path)
            .field("max_len", &self.max_len)
            .finish_non_exhaustive()
    }
}

impl FileIoBackend {
    /// A backend for `path` through `io`, reading at most [`MAX_VAULT_FILE_BYTES`].
    pub fn new(io: Arc<dyn FileIo>, path: impl Into<PathBuf>) -> Self {
        Self {
            io,
            path: path.into(),
            max_len: MAX_VAULT_FILE_BYTES,
        }
    }

    /// Lower the read limit (tests, small targets). The limit never rises above
    /// [`MAX_VAULT_FILE_BYTES`].
    pub fn with_read_limit(mut self, max_len: u64) -> Self {
        self.max_len = max_len.min(MAX_VAULT_FILE_BYTES);
        self
    }

    /// The file this backend reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The largest file `read` accepts, in bytes.
    pub fn read_limit(&self) -> u64 {
        self.max_len
    }
}

/// Flatten a `FileIo` failure. `NotFound` is handled by each caller before this.
fn io_message(kind: ErrorKind, message: &str) -> StorageError {
    StorageError::Io(format!("{kind:?}: {message}"))
}

fn common_error(e: KdbxError) -> StorageError {
    match e {
        KdbxError::Io { kind, message } => match kind {
            ErrorKind::TimedOut => StorageError::TimedOut,
            // A path component is a file, or the folder went away under us.
            ErrorKind::NotADirectory => StorageError::Unavailable(message),
            _ => io_message(kind, &message),
        },
        other => StorageError::Io(other.to_string()),
    }
}

fn read_error(e: KdbxError, max_bytes: u64) -> StorageError {
    match e {
        KdbxError::Io {
            kind: ErrorKind::FileTooLarge,
            ..
        } => StorageError::TooLarge { max_bytes },
        // `read_limited` reports a non-regular file as `InvalidInput`; a plain `read` of a
        // directory reports `IsADirectory`.
        KdbxError::Io {
            kind: ErrorKind::InvalidInput | ErrorKind::IsADirectory,
            ..
        } => StorageError::NotRegularFile,
        other => common_error(other),
    }
}

fn write_error(e: KdbxError, path: &Path) -> StorageError {
    match e {
        KdbxError::Io {
            kind: ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem,
            ..
        } => StorageError::ReadOnly,
        // The temp file could not be created: the folder is gone.
        KdbxError::Io {
            kind: ErrorKind::NotFound,
            ..
        } => StorageError::Unavailable(format!("folder missing for {}", path.display())),
        KdbxError::Io {
            kind: ErrorKind::IsADirectory,
            ..
        } => StorageError::NotRegularFile,
        other => common_error(other),
    }
}

fn is_not_found(e: &KdbxError) -> bool {
    matches!(
        e,
        KdbxError::Io {
            kind: ErrorKind::NotFound,
            ..
        }
    )
}

impl StorageBackend for FileIoBackend {
    fn exists(&self) -> Result<bool, StorageError> {
        match self.io.metadata(&self.path) {
            Ok(_) => Ok(true),
            Err(e) if is_not_found(&e) => Ok(false),
            Err(e) => Err(common_error(e)),
        }
    }

    fn modification_date(&self) -> Result<Option<SystemTime>, StorageError> {
        match self.io.metadata(&self.path) {
            Ok(meta) => Ok(meta.modified),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(common_error(e)),
        }
    }

    fn read(&self) -> Result<Option<Vec<u8>>, StorageError> {
        match self.io.read_limited(&self.path, self.max_len) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(read_error(e, self.max_len)),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), StorageError> {
        self.io
            .write_atomic(&self.path, bytes)
            .map_err(|e| write_error(e, &self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypass_kdbx::{test_util::FailingFileIo, FileMeta, PlainFileIo, VaultFile};
    use secrecy::SecretString;
    use std::{fs, sync::Mutex};

    /// Records every call and answers reads with `TimedOut` (an iCloud placeholder).
    struct TimedOutIo {
        calls: Mutex<Vec<&'static str>>,
    }
    impl FileIo for TimedOutIo {
        fn read(&self, _: &Path) -> mypass_kdbx::Result<Vec<u8>> {
            self.calls.lock().unwrap().push("read");
            Err(KdbxError::Io {
                kind: ErrorKind::TimedOut,
                message: "not downloaded yet".into(),
            })
        }
        fn read_limited(&self, path: &Path, _: u64) -> mypass_kdbx::Result<Vec<u8>> {
            self.read(path)
        }
        fn write_atomic(&self, _: &Path, _: &[u8]) -> mypass_kdbx::Result<()> {
            self.calls.lock().unwrap().push("write");
            Ok(())
        }
        fn metadata(&self, path: &Path) -> mypass_kdbx::Result<FileMeta> {
            self.calls.lock().unwrap().push("metadata");
            PlainFileIo.metadata(path)
        }
    }

    #[test]
    fn timed_out_read_surfaces_as_io_error_not_missing() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("vault.kdbx");
        fs::write(&path, b"placeholder").unwrap();
        let io = Arc::new(TimedOutIo {
            calls: Mutex::new(vec![]),
        });
        // Through the stage 1 seam: opening reports the timeout, never "missing" or "corrupt".
        let err = VaultFile::open_with(io.clone(), &path, &SecretString::from("pw")).unwrap_err();
        assert!(matches!(
            err,
            KdbxError::Io {
                kind: ErrorKind::TimedOut,
                ..
            }
        ));
        // Through the storage seam: `TimedOut`, not `Ok(None)` (which would mean "upload ours").
        let backend = FileIoBackend::new(io.clone(), &path);
        assert_eq!(backend.read(), Err(StorageError::TimedOut));
        assert!(!io.calls.lock().unwrap().contains(&"write"));
    }

    #[test]
    fn missing_file_reads_as_none_and_does_not_exist() {
        let d = tempfile::tempdir().unwrap();
        let backend = FileIoBackend::new(Arc::new(PlainFileIo), d.path().join("absent.kdbx"));
        assert_eq!(backend.read(), Ok(None));
        assert_eq!(backend.exists(), Ok(false));
        assert_eq!(backend.modification_date(), Ok(None));
    }

    #[test]
    fn oversized_file_is_too_large_and_directory_is_not_regular() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("vault.kdbx");
        fs::write(&path, [7u8; 32]).unwrap();
        let backend = FileIoBackend::new(Arc::new(PlainFileIo), &path).with_read_limit(31);
        assert_eq!(
            backend.read(),
            Err(StorageError::TooLarge { max_bytes: 31 })
        );
        assert_eq!(backend.exists(), Ok(true));
        let dir = FileIoBackend::new(Arc::new(PlainFileIo), d.path());
        assert_eq!(dir.read(), Err(StorageError::NotRegularFile));
    }

    #[test]
    fn read_limit_never_rises_above_the_named_cap() {
        let backend = FileIoBackend::new(Arc::new(PlainFileIo), "x").with_read_limit(u64::MAX);
        assert_eq!(backend.read_limit(), MAX_VAULT_FILE_BYTES);
    }

    #[test]
    fn injected_failures_map_to_storage_codes() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("vault.kdbx");
        fs::write(&path, b"x").unwrap();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        let backend = FileIoBackend::new(io.clone(), &path);
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        assert_eq!(backend.write(b"y"), Err(StorageError::ReadOnly));
        io.fail_write_atomic(ErrorKind::ReadOnlyFilesystem);
        assert_eq!(backend.write(b"y"), Err(StorageError::ReadOnly));
        io.fail_read(ErrorKind::TimedOut);
        assert_eq!(backend.read(), Err(StorageError::TimedOut));
        io.fail_read(ErrorKind::Other);
        assert!(matches!(backend.read(), Err(StorageError::Io(_))));
        io.fail_metadata(ErrorKind::TimedOut);
        assert_eq!(backend.exists(), Err(StorageError::TimedOut));
        assert_eq!(fs::read(&path).unwrap(), b"x");
    }
}
