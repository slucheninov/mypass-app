//! Atomic vault replacement and a conservative concurrent-change guard.
use crate::{Database, KdbxError, KdfLimits, Result};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime},
};
use uuid::Uuid;

/// Metadata and digest of the last successfully opened, saved or reloaded vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    /// Encoded file length in bytes.
    pub len: u64,
    /// Modification time, when supplied by the backend.
    pub modified: Option<SystemTime>,
    /// SHA-256 of the encoded vault, used when metadata ties.
    pub sha256: [u8; 32],
}
/// Result of a guarded save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Replacement and stamp refresh succeeded.
    Saved,
    /// The guard detected a change; no bytes were written.
    ChangedOnDisk,
}
/// File metadata used by the concurrent-change guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    /// Encoded file length in bytes.
    pub len: u64,
    /// Modification time, when supplied by the backend.
    pub modified: Option<SystemTime>,
}

/// The I/O boundary for vault bytes and metadata; implementations must not log bytes.
pub trait FileIo: Send + Sync {
    /// Read the encoded vault bytes.
    fn read(&self, path: &Path) -> Result<Vec<u8>>;
    /// Replace the vault atomically; an error can occur after replacement commits.
    /// Follow the permission and durability policy of [`write_atomic`].
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()>;
    /// Commit bytes checked against the caller's last read. Backends with transaction support
    /// override this to revalidate inside their write transaction. The default retains the
    /// best-effort guard used by plain local primary storage.
    fn write_checked(&self, path: &Path, expected: &[u8; 32], bytes: &[u8]) -> Result<SaveOutcome> {
        let _ = expected;
        self.write_atomic(path, bytes)?;
        Ok(SaveOutcome::Saved)
    }
    /// Read length and modification time, following the resolved target.
    fn metadata(&self, path: &Path) -> Result<FileMeta>;
    /// Open `path` for a bounded read, without blocking: a FIFO or a device never hangs the open.
    /// The returned type and length describe the OPENED handle (`fstat`), never a path checked
    /// before the open, so a path swapped for a FIFO or a device in between is still refused.
    ///
    /// The default refuses with `ErrorKind::Unsupported` (fail closed): a backend that cannot open
    /// without blocking and inspect the handle must not serve reads of untrusted files.
    fn open_read(&self, path: &Path) -> Result<OpenedFile> {
        let _ = path;
        Err(unsupported("this file backend has no bounded read"))
    }
    /// Read the encoded vault bytes of a regular file of at most `max_len` bytes.
    ///
    /// For vault files that other software writes (a sync folder, iCloud Drive, a migration
    /// target). Refusals are [`KdbxError::Io`] with `ErrorKind::FileTooLarge` past `max_len` and
    /// `ErrorKind::InvalidInput` for anything that is not a regular file (directory, FIFO,
    /// device); a missing file stays `ErrorKind::NotFound`. The default reads through
    /// [`FileIo::open_read`]: the type check is on the opened handle and the read goes through
    /// `take(max_len + 1)`, so neither a device nor a growing file can allocate past the limit.
    fn read_limited(&self, path: &Path, max_len: u64) -> Result<Vec<u8>> {
        read_opened(self.open_read(path)?, max_len)
    }
    /// Replace `path` with `bytes` only while it still holds the content whose SHA-256 is
    /// `expected` (`None`: only while the file is missing, never replacing one that appeared).
    /// The current content is read as by [`FileIo::read_limited`] with `max_len`.
    ///
    /// For files other software writes: a writer that changed the file before the final check
    /// gets [`ReplaceOutcome::Conflict`] (nothing written), and one that slipped in between the
    /// final check and the replacement is returned as [`ReplaceOutcome::Displaced`] and kept on
    /// disk instead of being destroyed. A volume that offers no way to replace a file without
    /// risking a racing writer's copy is refused with `ErrorKind::Unsupported` (see
    /// [`PlainFileIo`] for which volumes take which path).
    ///
    /// The default refuses with `ErrorKind::Unsupported` (fail closed).
    fn replace_if(
        &self,
        path: &Path,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
        max_len: u64,
    ) -> Result<ReplaceOutcome> {
        let _ = (path, expected, bytes, max_len);
        Err(unsupported("this file backend has no guarded replacement"))
    }
    /// Authenticates the confirmed copy with the unlocked vault's key under local KDF limits
    /// and checks that the root group UUID matches. This is read-only.
    fn authenticate_retained(
        &self,
        request: RetainedRemoval,
        password: &SecretString,
        root: Uuid,
    ) -> Result<AuthenticatedRetainedRemoval> {
        let _ = (request, password, root);
        Err(KdbxError::RetainedCopyRefused(
            RetainedRefusal::UnverifiedVault,
        ))
    }
    /// Deletes one authenticated retained copy only after re-verifying its name, type, link count,
    /// length, full SHA-256 and vault identity, and that the live vault is a different readable
    /// file (see [`remove_retained_copy`]). A refusal is [`KdbxError::RetainedCopyRefused`].
    ///
    /// The default refuses with `ErrorKind::Unsupported` (fail closed): a backend that cannot
    /// make these checks on its own files must never delete one.
    fn remove_retained(
        &self,
        removal: &AuthenticatedRetainedRemoval,
        known: &[RetainedIdentity],
    ) -> Result<()> {
        let _ = (removal, known);
        Err(unsupported(
            "this file backend cannot remove retained copies",
        ))
    }
}

/// A file opened for a bounded read ([`FileIo::open_read`]). `regular` and `len` describe the
/// opened handle, never a path checked before the open.
pub struct OpenedFile {
    reader: Box<dyn Read + Send>,
    regular: bool,
    len: u64,
}
impl OpenedFile {
    /// `regular` and `len` must come from the opened handle (`fstat`), not from the path.
    pub fn new(reader: Box<dyn Read + Send>, regular: bool, len: u64) -> Self {
        Self {
            reader,
            regular,
            len,
        }
    }
}
impl fmt::Debug for OpenedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenedFile")
            .field("regular", &self.regular)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// What a guarded replacement ([`FileIo::replace_if`]) did.
#[derive(Clone, PartialEq, Eq)]
pub enum ReplaceOutcome {
    /// The new bytes replaced exactly the expected content (or created the missing file).
    Replaced,
    /// The file no longer holds the expected content (or exists although it was expected to be
    /// missing); nothing was written.
    Conflict,
    /// Another writer replaced the file in the instant between the final check and the
    /// replacement. Its content was not destroyed: it is returned in `bytes` (empty when it
    /// could not be read back) and kept on disk at `kept`. The new bytes are normally in place,
    /// but not when yet another writer created the file meanwhile (move-aside path): read the
    /// path again before relying on what it holds.
    Displaced {
        /// The displaced content, bounded like [`FileIo::read_limited`].
        bytes: Vec<u8>,
        /// Where the displaced file was kept (a hidden name next to the target). A name that
        /// does not end in `.displaced` means no finalized name could be made: the file still sits
        /// under its working name.
        kept: PathBuf,
        /// A working name that could not be removed after the copy was published (same data as
        /// `kept`, under its working name): a warning the caller must surface.
        leftover: Option<PathBuf>,
    },
}
impl ReplaceOutcome {
    /// The path of a retained working file, including a name left after publication.
    /// Preserve the resolved directory so callers can check the file even if the vault
    /// path was a symlink or the link later points elsewhere.
    pub fn retained_working_path(&self) -> Option<&Path> {
        let Self::Displaced { kept, leftover, .. } = self else {
            return None;
        };
        leftover
            .as_deref()
            .or_else(|| (!is_published_name(kept)).then_some(kept.as_path()))
    }
}
/// Whether `kept` carries a finalized `.displaced` name.
fn is_published_name(kept: &Path) -> bool {
    kept.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(".displaced"))
}
impl fmt::Debug for ReplaceOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replaced => f.write_str("Replaced"),
            Self::Conflict => f.write_str("Conflict"),
            Self::Displaced {
                bytes,
                kept,
                leftover,
            } => f
                .debug_struct("Displaced")
                .field("len", &bytes.len())
                .field("kept", kept)
                .field("leftover", leftover)
                .finish(),
        }
    }
}

/// Production filesystem backend with atomic replacement.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainFileIo;
impl FileIo for PlainFileIo {
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        fs::read(path).map_err(io_error)
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        write_atomic(path, bytes)
    }
    fn metadata(&self, path: &Path) -> Result<FileMeta> {
        let meta = fs::metadata(path).map_err(io_error)?;
        Ok(FileMeta {
            len: meta.len(),
            modified: Some(meta.modified().map_err(io_error)?),
        })
    }
    fn open_read(&self, path: &Path) -> Result<OpenedFile> {
        let file = open_for_read(path).map_err(io_error)?;
        let meta = file.metadata().map_err(io_error)?;
        Ok(OpenedFile::new(Box::new(file), meta.is_file(), meta.len()))
    }
    /// The check of the current content and the replacement are not one atomic step on every
    /// file system. Invariant on every path: **no file that may hold another writer's vault is
    /// ever deleted or overwritten.** A file is deleted only once it was read back and proved to
    /// be the expected content (which the caller has already merged into `bytes`), and every
    /// rename goes to a fresh name of our own. Which path a volume takes:
    ///
    /// - **Atomic exchange: Unix where `renameat2(RENAME_EXCHANGE)` (Linux: ext4, xfs, btrfs,
    ///   tmpfs) or `renamex_np(RENAME_SWAP)` (macOS: APFS, HFS+) works.** The new file is
    ///   swapped in and whatever was at the path at that instant comes back under an
    ///   exchange working name that records our candidate's inode. A scan can distinguish
    ///   a stranded other writer's file from our uninstalled candidate even after a crash.
    ///   A competing copy is published under a fresh `.displaced` name with a no-replace
    ///   hard link. If publication fails, the outcome names the listed working copy.
    /// - **Move aside, then a no-replace link: volumes without the exchange that have hard
    ///   links (Windows NTFS and ReFS; SMB, NFS and other mounts that refuse the swap but
    ///   allow `link`).** After the final check, a probe link proves that hard links work
    ///   here. The current file is moved to a fresh `.aside` working name without replacing
    ///   a file already there (see `move_aside`), and the new file
    ///   is linked at the target unless a racing creator wins. The moved-aside file is removed
    ///   only when it proves to be the expected content. A competing version is published as
    ///   `.displaced`, when possible, and returned as [`ReplaceOutcome::Displaced`].
    ///   When installation fails, both restoring the original path and publishing a retained
    ///   copy are attempted. If restoration fails, the error names the surviving copy.
    ///   A reader can see an absent path while the move-aside operation is in progress.
    /// - **Neither (FAT, exFAT and network volumes without hard links):** refused with
    ///   `ErrorKind::Unsupported` before anything is renamed; the target is untouched and
    ///   only our temp file is removed. There is no safe way to replace a file there.
    ///
    /// Remaining window (documented residual): a writer that rewrites the old file **in
    /// place** (truncate and write on the same inode) after we read the swapped-out or
    /// moved-aside file back and judged it to be the expected content. Atomic writers (temp
    /// file plus rename: this app, KeePassXC, sync clients) are unaffected by that window.
    /// The Unix hard-link fallback also cannot atomically couple its source identity check
    /// with unlink. A path replacement in that final interval is outside its protection.
    ///
    /// A missing file (`expected` is `None`) is created without ever replacing one that appeared:
    /// `RENAME_NOREPLACE`/`RENAME_EXCL`, else a hard link (fails on an existing name), else the
    /// same `Unsupported` refusal; there is no check-then-rename.
    fn replace_if(
        &self,
        path: &Path,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
        max_len: u64,
    ) -> Result<ReplaceOutcome> {
        replace_if_production(path, expected, bytes, max_len)
    }
    fn authenticate_retained(
        &self,
        request: RetainedRemoval,
        password: &SecretString,
        root: Uuid,
    ) -> Result<AuthenticatedRetainedRemoval> {
        authenticate_retained_copy(request, password, root)
    }
    fn remove_retained(
        &self,
        removal: &AuthenticatedRetainedRemoval,
        known: &[RetainedIdentity],
    ) -> Result<()> {
        remove_retained_copy(removal, known)
    }
}
fn unsupported(message: &str) -> KdbxError {
    io_error(io::Error::new(io::ErrorKind::Unsupported, message))
}
fn too_large(max_len: u64) -> KdbxError {
    io_error(io::Error::new(
        io::ErrorKind::FileTooLarge,
        format!("file is larger than {max_len} bytes"),
    ))
}
fn not_regular() -> KdbxError {
    io_error(io::Error::new(
        io::ErrorKind::InvalidInput,
        "not a regular file",
    ))
}
/// Opens for reading without blocking: `O_NONBLOCK` makes the open of a FIFO without a writer
/// return at once (the handle check then refuses it), and `O_NOCTTY` keeps a terminal device
/// from becoming the controlling terminal. Regular-file reads are unaffected by `O_NONBLOCK`.
#[cfg(unix)]
fn open_for_read(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let flags = (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOCTTY).bits();
    let flags = i32::try_from(flags).map_err(|_| io::Error::other("open flags out of range"))?;
    OpenOptions::new().read(true).custom_flags(flags).open(path)
}
/// Windows has no FIFOs in the file namespace, and opening a file never waits for a writer, so
/// a path check before the open cannot be raced into a hang. It only turns a directory (whose
/// plain open fails with "access denied") into the "not a regular file" refusal; the handle is
/// still checked after the open.
#[cfg(not(unix))]
fn open_for_read(path: &Path) -> io::Result<fs::File> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    fs::File::open(path)
}
/// The bounded read behind [`FileIo::read_limited`]: refuses a non-regular handle before
/// reading, and never allocates more than `max_len + 1` bytes whatever the handle reports.
fn read_opened(opened: OpenedFile, max_len: u64) -> Result<Vec<u8>> {
    if !opened.regular {
        return Err(not_regular());
    }
    if opened.len > max_len {
        return Err(too_large(max_len));
    }
    let capacity = usize::try_from(opened.len.min(max_len)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    // One byte past the limit tells a file that grew after `fstat` from one that fits.
    opened
        .reader
        .take(max_len.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_len {
        return Err(too_large(max_len));
    }
    Ok(bytes)
}
pub(crate) fn io_error(e: io::Error) -> KdbxError {
    KdbxError::Io {
        kind: e.kind(),
        message: e.to_string(),
    }
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn stamp(meta: FileMeta, bytes: &[u8]) -> FileStamp {
    FileStamp {
        len: meta.len,
        modified: meta.modified,
        sha256: hash(bytes),
    }
}

/// Holds no password, plaintext or serialized vault bytes.
pub struct VaultFile {
    path: PathBuf,
    stamp: FileStamp,
    io: Arc<dyn FileIo>,
}
impl fmt::Debug for VaultFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultFile")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}
impl VaultFile {
    /// Resolve the path, authenticate the vault and record its stamp using [`PlainFileIo`].
    pub fn open(path: &Path, password: &SecretString) -> Result<(Self, Database)> {
        Self::open_with(Arc::new(PlainFileIo), path, password)
    }
    /// Open and authenticate using an injected I/O backend after resolving the path.
    pub fn open_with(
        io: Arc<dyn FileIo>,
        path: &Path,
        password: &SecretString,
    ) -> Result<(Self, Database)> {
        Self::open_with_limits(io, path, password, KdfLimits::LOCAL)
    }

    /// Open through an injected I/O backend using the selected KDF limits.
    pub fn open_with_limits(
        io: Arc<dyn FileIo>,
        path: &Path,
        password: &SecretString,
        limits: KdfLimits,
    ) -> Result<(Self, Database)> {
        Self::open_with_limits_checked(io, path, password, limits, &|| Ok(()))
    }

    /// Open with a caller-owned withdrawal fence. Checks before the read and again after it,
    /// before starting decryption; a KDF already in progress is not interrupted. The caller
    /// still owns any atomic installation fence after this method returns.
    pub fn open_with_checked<E: From<KdbxError>>(
        io: Arc<dyn FileIo>,
        path: &Path,
        password: &SecretString,
        check: &dyn Fn() -> std::result::Result<(), E>,
    ) -> std::result::Result<(Self, Database), E> {
        Self::open_with_limits_checked(io, path, password, KdfLimits::LOCAL, check)
    }

    /// Open with a withdrawal fence and the selected KDF limits.
    pub fn open_with_limits_checked<E: From<KdbxError>>(
        io: Arc<dyn FileIo>,
        path: &Path,
        password: &SecretString,
        limits: KdfLimits,
        check: &dyn Fn() -> std::result::Result<(), E>,
    ) -> std::result::Result<(Self, Database), E> {
        check()?;
        // Path identity is resolved once; every later byte/metadata operation uses the seam.
        let path = resolve_vault_path(path).map_err(io_error)?;
        check()?;
        let bytes = io.read(&path)?;
        check()?;
        let db = Database::open_with_limits(&bytes, password, limits)?;
        check()?;
        let stamp = stamp(io.metadata(&path)?, &bytes);
        Ok((Self { path, stamp, io }, db))
    }

    /// Return the resolved vault path.
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// The IO seam this file reads and writes through.
    pub fn io(&self) -> &Arc<dyn FileIo> {
        &self.io
    }
    /// Compare metadata and, when metadata ties, the encoded content digest.
    /// Missing files count as changed; other I/O errors propagate.
    pub fn has_changed_on_disk(&self) -> Result<bool> {
        let meta = match self.io.metadata(&self.path) {
            Err(KdbxError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            }) => return Ok(true),
            other => other?,
        };
        if meta.len != self.stamp.len || meta.modified != self.stamp.modified {
            return Ok(true);
        }
        // HFS+/FAT timestamps can have 1-2 s granularity. Hash even when metadata agrees.
        match self.io.read(&self.path) {
            Ok(bytes) => Ok(hash(&bytes) != self.stamp.sha256),
            Err(KdbxError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            }) => Ok(true),
            Err(e) => Err(e),
        }
    }
    /// Serialize first, then check for external changes immediately before replacement.
    /// A changed file returns [`SaveOutcome::ChangedOnDisk`] without writing anything.
    /// Best-effort guard, not a cross-process lock: another writer can race the check.
    ///
    /// On Unix the replacement is created 0o600 and narrowed to the target's owner
    /// read/write bits; wider modes lose group/other access, and a 0o400 target is
    /// replaced. Ownership, ACLs and xattrs are not carried over.
    /// An error after commit (directory sync or metadata refresh) keeps the old stamp;
    /// the caller must reload, because the disk may already contain the replacement.
    pub fn save(&mut self, db: &Database, password: &SecretString) -> Result<SaveOutcome> {
        self.save_serializing(|| db.save(password))
    }
    fn save_serializing(
        &mut self,
        serialize: impl FnOnce() -> Result<Vec<u8>>,
    ) -> Result<SaveOutcome> {
        let bytes = serialize()?;
        if self.has_changed_on_disk()? {
            return Ok(SaveOutcome::ChangedOnDisk);
        }
        self.save_checked_bytes(&bytes)
    }
    /// Writes already encrypted pending edits using the same transaction guard as `save`.
    pub fn save_checked_bytes(&mut self, bytes: &[u8]) -> Result<SaveOutcome> {
        let outcome = self
            .io
            .write_checked(&self.path, &self.stamp.sha256, bytes)?;
        if outcome == SaveOutcome::Saved {
            self.stamp = stamp(self.io.metadata(&self.path)?, bytes);
        }
        Ok(outcome)
    }
    /// Serialize and replace without checking for external changes.
    /// Uses the permission policy of [`write_atomic`]. An error after commit keeps
    /// the old stamp; the caller must reload before saving again.
    pub fn save_overwriting(&mut self, db: &Database, password: &SecretString) -> Result<()> {
        let bytes = db.save(password)?;
        self.save_bytes(&bytes)
    }
    fn save_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.io.write_atomic(&self.path, bytes)?;
        self.stamp = stamp(self.io.metadata(&self.path)?, bytes);
        Ok(())
    }
    /// Authenticate the current disk content and refresh the stamp only on success.
    pub fn reload(&mut self, password: &SecretString) -> Result<Database> {
        self.reload_with_limits(password, KdfLimits::LOCAL)
    }

    /// Reload using the selected KDF limits, refreshing the stamp only on success.
    pub fn reload_with_limits(
        &mut self,
        password: &SecretString,
        limits: KdfLimits,
    ) -> Result<Database> {
        let bytes = self.io.read(&self.path)?;
        let db = Database::open_with_limits(&bytes, password, limits)?;
        self.stamp = stamp(self.io.metadata(&self.path)?, &bytes);
        Ok(db)
    }
}

pub(crate) const RENAME_RETRY_SCHEDULE_MS: [u64; 6] = [50, 100, 200, 400, 800, 1600];
pub(crate) fn is_retryable_rename_error(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    if matches!(e.raw_os_error(), Some(5 | 32 | 33)) {
        return true;
    }
    false
}
fn retry_rename<T>(
    mut attempt: impl FnMut() -> io::Result<T>,
    mut sleep: impl FnMut(Duration),
) -> io::Result<T> {
    let mut delays = RENAME_RETRY_SCHEDULE_MS.into_iter();
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(e) => {
                if !is_retryable_rename_error(&e) {
                    return Err(e);
                }
                match delays.next() {
                    Some(ms) => sleep(Duration::from_millis(ms)),
                    None => return Err(e),
                }
            }
        }
    }
}
struct TempCleanup {
    path: PathBuf,
    armed: bool,
}
impl TempCleanup {
    /// The temp name was handed over (renamed or swapped): never delete what it names now.
    fn disarm(mut self) {
        self.armed = false;
    }
}
impl Drop for TempCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
/// The identity of a vault path: canonical when the file exists. A file that is not there (an
/// iCloud Drive placeholder `.name.icloud`, or a vault whose folder is offline and served from a
/// cache by the seam) resolves through its parent, and an absolute path whose parent is gone too
/// is kept as written, so the `FileIo` seam decides whether it can be read. A dangling symlink
/// stays an error. The seam's read still fails with `NotFound` for a plain missing file.
fn resolve_vault_path(path: &Path) -> io::Result<PathBuf> {
    let err = match fs::canonicalize(path) {
        Ok(real) => return Ok(real),
        Err(e) if e.kind() == io::ErrorKind::NotFound => e,
        Err(e) => return Err(e),
    };
    if fs::symlink_metadata(path).is_ok() {
        return Err(err);
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(err);
    };
    if parent.as_os_str().is_empty() {
        return Err(err);
    }
    match fs::canonicalize(parent) {
        Ok(real) => Ok(real.join(name)),
        Err(e) if e.kind() == io::ErrorKind::NotFound && path.is_absolute() => {
            Ok(path.to_path_buf())
        }
        Err(_) => Err(err),
    }
}
fn resolve_target(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(p) => Ok(p),
        // canonicalize follows dangling symlinks too: never replace a dangling link itself.
        Err(e) if e.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path.to_path_buf()),
            Ok(_) => Err(e),
            Err(other) => Err(other),
        },
        Err(e) => Err(e),
    }
}
/// Replace in the same directory, syncing content before rename and the directory afterwards.
/// On Unix the replacement is created 0o600, then narrowed to the existing target's
/// owner read/write bits (mode & 0o600). Wider modes lose group/other access; even
/// a 0o400 target is replaced. Ownership, ACLs and xattrs are not carried over.
/// Directory-sync Unsupported/InvalidInput errors are tolerated for filesystems
/// that cannot sync directories; durability across power loss is then not guaranteed.
/// Any other final directory sync failure occurs after replacement has committed: we report the error
/// but cannot safely roll back over a possible concurrent writer. The caller must reload.
/// Cleanup is best effort if the OS denies deletion too; no bytes are ever included in errors.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic_using(
        path,
        bytes,
        |source, target| fs::rename(source, target),
        std::thread::sleep,
    )
}
fn write_atomic_using(
    path: &Path,
    bytes: &[u8],
    rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    sleep: impl FnMut(Duration),
) -> Result<()> {
    write_atomic_with_sync(path, bytes, rename, sleep, sync_directory)
}
fn sync_directory(parent: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(parent).and_then(|dir| dir.sync_all())
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}
fn resolve_using(
    resolve: impl FnMut() -> io::Result<PathBuf>,
    sleep: impl FnMut(Duration),
) -> io::Result<PathBuf> {
    let mut resolve = resolve;
    if cfg!(windows) {
        retry_rename(resolve, sleep)
    } else {
        resolve()
    }
}
fn write_atomic_with_sync(
    path: &Path,
    bytes: &[u8],
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    mut sleep: impl FnMut(Duration),
    mut sync: impl FnMut(&Path) -> io::Result<()>,
) -> Result<()> {
    // Windows canonicalization opens a handle and can itself hit a sharing violation.
    let target = resolve_using(|| resolve_target(path), &mut sleep).map_err(io_error)?;
    let (parent, name) = split_target(&target)?;
    let temp = SiblingNames::new(parent, name)?.temp;
    let cleanup = write_temp_sibling(temp, &target, bytes)?;
    let temp = cleanup.path.clone();
    retry_rename(|| rename(&temp, &target), &mut sleep).map_err(io_error)?;
    // Rename transferred ownership; this name may now belong to another writer.
    cleanup.disarm();
    finish_directory_sync(sync(parent))
}
fn split_target(target: &Path) -> Result<(&Path, &std::ffi::OsStr)> {
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target.file_name().ok_or_else(|| {
        io_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing file name",
        ))
    })?;
    Ok((parent, name))
}
mod retained;
pub use retained::{
    authenticate_retained_copy, inspect_retained_copies, recorded_retained_copies,
    remove_retained_copy, set_retained_copy_listener, AuthenticatedRetainedRemoval,
    RetainedCopyCheck, RetainedCopyListener, RetainedIdentity, RetainedInspection,
    RetainedProvenance, RetainedRefusal, RetainedRemoval, RETAINED_IN_FLIGHT_GRACE,
};

/// The longest prefix of the target's file name used in a sibling name, in bytes. With the
/// `.<pid>.<nanos>.<n>.displaced` tail (at most 64 bytes) a sibling name stays far below the
/// 255-byte (or 255-UTF-16-unit) name limit of every common file system, whatever the length of
/// the target's name, so naming a displaced file never fails because the name is too long.
const SIBLING_STEM_MAX: usize = 96;

/// The hidden names one replacement uses next to its target, all sharing one unique token
/// `.<stem>.<pid>.<nanos>.<n>`: a process id, the clock and a per-process counter, so no two
/// replacements (in this process or another) ever pick the same name.
struct SiblingNames {
    parent: PathBuf,
    name: std::ffi::OsString,
    /// The new bytes, written and synced before anything is renamed.
    temp: PathBuf,
    /// Where a file that may hold another writer's vault is kept for good (a finalized
    /// competing copy listed by [`displaced_copies`]).
    displaced: PathBuf,
    /// Where the current target waits while the move-aside path installs the replacement. It is
    /// the rollback source and may be deleted when it proves to be the expected content, so it
    /// has its own suffix and is listed separately from kept competing copies.
    aside: PathBuf,
    /// Where the file swapped out by an atomic exchange is moved right after the swap.
    #[cfg(unix)]
    exchanged: PathBuf,
    /// A probe link proving that this volume has hard links (move-aside path).
    probe: PathBuf,
}
impl SiblingNames {
    fn new(parent: &Path, name: &std::ffi::OsStr) -> Result<Self> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| io_error(io::Error::other("system clock before epoch")))?
            .as_nanos();
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stem = name.to_string_lossy();
        let token = format!(
            ".{}.{}.{nanos}.{n}",
            sibling_stem(&stem),
            std::process::id()
        );
        Ok(Self {
            parent: parent.to_path_buf(),
            name: name.to_os_string(),
            temp: parent.join(format!("{token}.tmp")),
            displaced: parent.join(format!("{token}.displaced")),
            aside: parent.join(format!("{token}.aside")),
            #[cfg(unix)]
            exchanged: parent.join(format!("{token}.exchanged")),
            probe: parent.join(format!("{token}.link.tmp")),
        })
    }
}
impl SiblingNames {
    /// A set of names no replacement has used before (a fresh token), for when a planned name
    /// is already taken.
    fn fresh(&self) -> Result<Self> {
        Self::new(&self.parent, &self.name)
    }
}
/// The prefix of the target's file name that sibling names carry (at most [`SIBLING_STEM_MAX`]
/// bytes, cut on a character boundary).
fn sibling_stem(name: &str) -> &str {
    let mut cut = name.len().min(SIBLING_STEM_MAX);
    while !name.is_char_boundary(cut) {
        cut = cut.saturating_sub(1);
    }
    name.get(..cut).unwrap_or_default()
}

// A generated name is unique within this process, even across directories. Track it by
// name so scans of the same directory through an alias also skip an in-flight move-aside or
// exchange working file.
struct ActiveAside(OsString);

impl ActiveAside {
    fn names() -> MutexGuard<'static, Vec<OsString>> {
        static NAMES: Mutex<Vec<OsString>> = Mutex::new(Vec::new());
        NAMES.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn begin(path: &Path) -> Self {
        let name = path.file_name().unwrap_or_default().to_owned();
        Self::names().push(name.clone());
        Self(name)
    }

    fn contains(name: &OsStr) -> bool {
        Self::names().iter().any(|active| active == name)
    }
}

impl Drop for ActiveAside {
    fn drop(&mut self) {
        Self::names().retain(|active| *active != self.0);
    }
}

/// A replacement's kept copy or working file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplacedCopyKind {
    /// A competing version published under a finalized name.
    Displaced,
    /// A move-aside working file, which may remain after a failure or crash.
    Aside,
    /// The file an atomic exchange swapped out, still under its working name because it could
    /// not be published or removed (or a crash came first).
    Exchanged,
}

/// One retained vault copy next to the target vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplacedCopy {
    /// Where the copy is.
    pub path: PathBuf,
    /// Its length in bytes.
    pub len: u64,
    /// How the replacement named it.
    pub kind: DisplacedCopyKind,
}

/// Matches the [`SiblingNames`] stem and three all-digit counters. This is a name match:
/// long vault names with the same truncated stem share a listing, and manually created
/// matching files also appear. Listing never authorizes access, modification or deletion.
fn displaced_kind(target_name: &str, candidate: &str) -> Option<DisplacedCopyKind> {
    let rest = candidate
        .strip_prefix('.')?
        .strip_prefix(sibling_stem(target_name))?
        .strip_prefix('.')?;
    let (counters, suffix) = rest.rsplit_once('.')?;
    let count = if suffix == "exchanging" && cfg!(unix) {
        4
    } else {
        3
    };
    if counters.split('.').count() != count
        || !counters
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    match suffix {
        "displaced" => Some(DisplacedCopyKind::Displaced),
        "aside" => Some(DisplacedCopyKind::Aside),
        "exchanged" => Some(DisplacedCopyKind::Exchanged),
        "exchanging" if cfg!(unix) => Some(DisplacedCopyKind::Exchanged),
        _ => None,
    }
}

/// Whether this name and metadata can represent a retained vault. An exchange working
/// name encodes the inode of the candidate. While it still names that inode, it contains
/// only the uninstalled candidate, even after a crash. Ignore old empty reservations too.
fn is_retained_file(name: &OsStr, meta: &fs::Metadata) -> bool {
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Some(candidate_inode) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".exchanging"))
            .and_then(|stem| stem.rsplit_once('.'))
            .and_then(|(_, inode)| inode.parse::<u64>().ok())
        {
            return meta.ino() != candidate_inode;
        }
    }
    #[cfg(not(unix))]
    let _ = name;
    true
}

/// The competing copies, move-aside files and retained exchange copies next to a vault, sorted
/// by path. Active working names and uninstalled candidates are excluded. An unreadable
/// folder or entry is an error. A vanished entry is skipped. The scan is read-only.
pub fn displaced_copies(target: &Path) -> Result<Vec<DisplacedCopy>> {
    // Resolved exactly as the writer resolves it, so a vault reached through a symlink is
    // scanned where its copies are kept: next to the resolved file, under the resolved name.
    let target = resolve_using(|| resolve_target(target), std::thread::sleep).map_err(io_error)?;
    let (parent, name) = split_target(&target)?;
    let name = name.to_string_lossy();
    let entries = fs::read_dir(parent).map_err(io_error)?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io_error(e)),
        };
        let candidate = entry.file_name();
        let Some(kind) = displaced_kind(&name, &candidate.to_string_lossy()) else {
            continue;
        };
        if kind != DisplacedCopyKind::Displaced && ActiveAside::contains(&candidate) {
            continue;
        }
        // A symbolic link or anything but a regular file is not a kept copy: never listed.
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io_error(e)),
        };
        if is_retained_file(&candidate, &meta) {
            found.push(DisplacedCopy {
                path: entry.path(),
                len: meta.len(),
                kind,
            });
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// Writes `bytes` to the fresh file `temp` next to `target` and syncs it. The returned guard
/// deletes the temp file on drop until it is disarmed (after a rename handed the name over).
fn write_temp_sibling(temp: PathBuf, target: &Path, bytes: &[u8]) -> Result<TempCleanup> {
    let (file, cleanup) = create_temp_sibling(temp)?;
    finish_temp_write(file, target, bytes)?;
    Ok(cleanup)
}

fn create_temp_sibling(temp: PathBuf) -> Result<(fs::File, TempCleanup)> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Install cleanup only after create_new succeeds: never delete a pre-planted file.
    let file = options.open(&temp).map_err(io_error)?;
    let cleanup = TempCleanup {
        path: temp,
        armed: true,
    };
    Ok((file, cleanup))
}

fn finish_temp_write(mut file: fs::File, target: &Path, bytes: &[u8]) -> Result<()> {
    file.write_all(bytes).map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match fs::metadata(target) {
            // Preserve owner read/write restrictions; discard group/other and special bits.
            Ok(meta) => file
                .set_permissions(fs::Permissions::from_mode(
                    meta.permissions().mode() & 0o600,
                ))
                .map_err(io_error)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
    }
    // Only the Unix permission carry-over reads the target.
    #[cfg(not(unix))]
    let _ = target;
    file.sync_all().map_err(io_error)
}
fn finish_directory_sync(result: io::Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        // Some network/FUSE volumes and macOS filesystems cannot sync directories.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(io_error(e)),
    }
}

/// The current content of `path` (bounded, regular, read on the opened handle); `None` when the
/// file is missing.
fn read_current(path: &Path, max_len: u64) -> Result<Option<Vec<u8>>> {
    let file = match open_for_read(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(e)),
    };
    let meta = file.metadata().map_err(io_error)?;
    read_opened(
        OpenedFile::new(Box::new(file), meta.is_file(), meta.len()),
        max_len,
    )
    .map(Some)
}
/// Whether `path` names anything right now (a dangling symlink counts).
fn exists_now(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error(e)),
    }
}
/// The refusal of a volume that can neither exchange nor link: there is no way to replace a
/// file there without risking a racing writer's copy.
fn no_safe_replace(cause: &io::Error) -> KdbxError {
    io_error(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "this volume cannot replace a file without risking a concurrent copy (no atomic \
             exchange and no hard links: {:?})",
            cause.kind()
        ),
    ))
}

/// How the atomic rename step ended on this platform and volume.
#[cfg(unix)]
enum Swap {
    Done,
    /// The volume or kernel does not support the flag: use the move-aside path.
    Unsupported,
    /// `ENOENT` for an exchange (the target vanished), `EEXIST` for a no-replace rename.
    Refused,
}
#[cfg(unix)]
fn rename_with(from: &Path, to: &Path, flags: rustix::fs::RenameFlags) -> Result<Swap> {
    use rustix::io::Errno;
    match rustix::fs::renameat_with(rustix::fs::CWD, from, rustix::fs::CWD, to, flags) {
        Ok(()) => Ok(Swap::Done),
        Err(e) if e == Errno::NOENT || e == Errno::EXIST => Ok(Swap::Refused),
        Err(e)
            if e == Errno::INVAL
                || e == Errno::NOSYS
                || e == Errno::NOTSUP
                || e == Errno::OPNOTSUPP
                || e == Errno::XDEV =>
        {
            Ok(Swap::Unsupported)
        }
        Err(e) => Err(io_error(e.into())),
    }
}

/// The system calls of a guarded replacement that tests replace to inject races and failures.
/// Production uses [`replace_if_production`]; there is no `cfg(test)` branch in the replacement.
struct Seams<'a> {
    /// Runs right after the final check, before anything is renamed or linked.
    before_install: &'a mut dyn FnMut(),
    /// Runs after the current file was moved aside, before the new one is linked in.
    after_aside: &'a mut dyn FnMut(),
    /// Try `RENAME_EXCHANGE` / `RENAME_NOREPLACE` first (`false`: a volume without them).
    exchange: bool,
    /// A rename that never replaces an existing name: `AlreadyExists` when the name is taken,
    /// `Unsupported` where the platform or volume has no such rename (move aside, and the
    /// swapped-out file after an exchange).
    noreplace: &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    /// A hard link, failing on an existing name (the no-replace install).
    link: &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    /// Drops a name of a file whose data stays reachable under another name (the working name
    /// after a publication, or the verified expected content).
    unlink: &'a mut dyn FnMut(&Path) -> io::Result<()>,
    /// Syncs the folder after the replacement committed.
    sync_dir: &'a mut dyn FnMut(&Path) -> io::Result<()>,
    /// Sleeps between the retries of a rename or link that hit a sharing violation.
    sleep: &'a mut dyn FnMut(Duration),
}
/// `renameat2(RENAME_NOREPLACE)` (Linux) or `renamex_np(RENAME_EXCL)` (macOS): one atomic step
/// that fails with `EEXIST` instead of replacing a name that is taken. `std` has no such
/// rename, so it comes from `rustix`. A volume without the flag answers `Unsupported`.
#[cfg(unix)]
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::io::Errno;
    match rustix::fs::renameat_with(
        rustix::fs::CWD,
        from,
        rustix::fs::CWD,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    ) {
        Ok(()) => Ok(()),
        Err(e)
            if e == Errno::INVAL
                || e == Errno::NOSYS
                || e == Errno::NOTSUP
                || e == Errno::OPNOTSUPP
                || e == Errno::XDEV =>
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no-replace rename unsupported",
            ))
        }
        // `EEXIST` maps to `AlreadyExists`, `ENOENT` to `NotFound`.
        Err(e) => Err(e.into()),
    }
}
#[cfg(windows)]
use mypass_platform::windows::rename_noreplace;

#[cfg(not(any(windows, unix)))]
fn rename_noreplace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

/// [`replace_if_with`] over the real system calls.
fn replace_if_production(
    path: &Path,
    expected: Option<&[u8; 32]>,
    bytes: &[u8],
    max_len: u64,
) -> Result<ReplaceOutcome> {
    let (mut before_install, mut after_aside) = (|| {}, || {});
    let mut link = |from: &Path, to: &Path| fs::hard_link(from, to);
    let mut unlink = |at: &Path| drop_name(at);
    let mut sync_dir = |dir: &Path| sync_directory(dir);
    let mut sleep = std::thread::sleep;
    let mut seams = Seams {
        before_install: &mut before_install,
        after_aside: &mut after_aside,
        exchange: true,
        noreplace: &mut rename_noreplace,
        link: &mut link,
        unlink: &mut unlink,
        sync_dir: &mut sync_dir,
        sleep: &mut sleep,
    };
    replace_if_with(path, expected, bytes, max_len, &mut seams)
}

/// Inject a move-aside race or strand the moved copy by failing install and recovery links.
#[cfg(feature = "test-util")]
pub(crate) fn replace_if_injected(
    path: &Path,
    expected: &[u8; 32],
    bytes: &[u8],
    max_len: u64,
    strand: bool,
    mut before_aside: Option<crate::test_util::AsideHook>,
) -> Result<ReplaceOutcome> {
    let mut first_link = true;
    #[cfg(unix)]
    let force_link_aside = before_aside.is_some();
    let mut seams = Seams {
        before_install: &mut || {},
        after_aside: &mut || {},
        exchange: false,
        noreplace: &mut |from, to| {
            #[cfg(unix)]
            if force_link_aside {
                return Err(io::ErrorKind::Unsupported.into());
            }
            #[cfg(not(unix))]
            if let Some(hook) = before_aside.take() {
                hook(to);
            }
            rename_noreplace(from, to)
        },
        link: &mut |from, to| {
            let probe = std::mem::take(&mut first_link);
            #[cfg(unix)]
            if to.extension().is_some_and(|ext| ext == "aside") {
                if let Some(hook) = before_aside.take() {
                    hook(to);
                }
            }
            if strand && !probe {
                Err(io::Error::other("injected link failure"))
            } else {
                fs::hard_link(from, to)
            }
        },
        unlink: &mut drop_name,
        sync_dir: &mut sync_directory,
        sleep: &mut |_| {},
    };
    replace_if_with(path, Some(expected), bytes, max_len, &mut seams)
}

/// The guarded replacement behind [`PlainFileIo::replace_if`].
fn replace_if_with(
    path: &Path,
    expected: Option<&[u8; 32]>,
    bytes: &[u8],
    max_len: u64,
    seams: &mut Seams<'_>,
) -> Result<ReplaceOutcome> {
    // Resolved once: a symlink retargeted later cannot redirect the replacement to another file.
    let target = resolve_using(|| resolve_target(path), &mut *seams.sleep).map_err(io_error)?;
    let outcome = replace_resolved(&target, expected, bytes, max_len, seams);
    // Every copy this replacement retained gets its write-time identity now.
    retained::record_outcome(&target, &outcome);
    outcome
}

fn replace_resolved(
    target: &Path,
    expected: Option<&[u8; 32]>,
    bytes: &[u8],
    max_len: u64,
    seams: &mut Seams<'_>,
) -> Result<ReplaceOutcome> {
    let target = target.to_path_buf();
    let (parent, name) = split_target(&target)?;
    let names = SiblingNames::new(parent, name)?;
    #[cfg(unix)]
    let mut names = names;
    // The exchange flag is only read where `renameat_with` exists.
    #[cfg(not(unix))]
    let _ = seams.exchange;
    let (file, cleanup) = create_temp_sibling(names.temp.clone())?;
    #[cfg(unix)]
    let mut cleanup = cleanup;
    #[cfg(unix)]
    let exchange_working = if seams.exchange && expected.is_some() {
        prepare_exchange(&mut names, &mut cleanup, &file)?
    } else {
        None
    };
    // Write and sync the candidate under its working name before the final check.
    finish_temp_write(file, &target, bytes)?;
    let outcome = match expected {
        None => install_new(&target, &names, cleanup, seams)?,
        Some(expected) => replace_expected(
            &target,
            &names,
            cleanup,
            expected,
            max_len,
            seams,
            #[cfg(unix)]
            exchange_working.is_some(),
        )?,
    };
    if outcome != ReplaceOutcome::Conflict {
        // A failure here comes after the replacement committed; when a retained file sits under
        // a working name, the error must preserve that path.
        finish_directory_sync((seams.sync_dir)(parent)).map_err(|e| {
            match outcome.retained_working_path() {
                Some(path) => with_retained_path(e, path),
                None => e,
            }
        })?;
    }
    Ok(outcome)
}

/// Tag the candidate before writing, so the exchange working name is recognizable even
/// if a crash interrupts the write or the exchange.
#[cfg(unix)]
fn prepare_exchange(
    names: &mut SiblingNames,
    cleanup: &mut TempCleanup,
    file: &fs::File,
) -> Result<Option<ActiveAside>> {
    use std::os::unix::fs::MetadataExt;
    let inode = file.metadata().map_err(io_error)?.ino();
    let working = names.temp.with_extension(format!("{inode}.exchanging"));
    let active = ActiveAside::begin(&working);
    match rename_noreplace(&names.temp, &working) {
        Ok(()) => {
            cleanup.path = working.clone();
            names.temp = working;
            Ok(Some(active))
        }
        // The volume does not support the flag. Leave the candidate under its temp
        // name and continue through the move-aside path.
        Err(e) if e.kind() == io::ErrorKind::Unsupported => Ok(None),
        Err(e) => Err(io_error(e)),
    }
}

/// Only confirmed missing filesystem primitives are permanent capability failures.
fn link_failure(e: io::Error) -> KdbxError {
    let capability = matches!(
        e.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::CrossesDevices
    );
    #[cfg(unix)]
    let capability = capability
        || matches!(e.raw_os_error(), Some(code) if code == rustix::io::Errno::PERM.raw_os_error());
    #[cfg(windows)]
    let capability = capability || matches!(e.raw_os_error(), Some(1 | 17 | 50));
    if capability {
        no_safe_replace(&e)
    } else {
        io_error(e)
    }
}

/// Proves hard-link support before moving any target. Sharing violations use bounded backoff;
/// timeouts, permission and resource failures keep their original retryable classification.
fn probe_links(names: &SiblingNames, seams: &mut Seams<'_>) -> Result<()> {
    match retry_rename(
        || (seams.link)(&names.temp, &names.probe),
        &mut *seams.sleep,
    ) {
        Ok(()) => {
            let _ = fs::remove_file(&names.probe);
            Ok(())
        }
        Err(e) => Err(link_failure(e)),
    }
}

/// Creates the missing `target` from the temp file, never replacing a file that appeared.
fn install_new(
    target: &Path,
    names: &SiblingNames,
    cleanup: TempCleanup,
    seams: &mut Seams<'_>,
) -> Result<ReplaceOutcome> {
    if exists_now(target)? {
        return Ok(ReplaceOutcome::Conflict);
    }
    (seams.before_install)();
    #[cfg(unix)]
    if seams.exchange {
        match rename_with(&names.temp, target, rustix::fs::RenameFlags::NOREPLACE)? {
            Swap::Done => {
                cleanup.disarm();
                return Ok(ReplaceOutcome::Replaced);
            }
            Swap::Refused => return Ok(ReplaceOutcome::Conflict),
            Swap::Unsupported => {}
        }
    }
    // A hard link fails on an existing name. On success the temp name is a second link to the
    // same (our) file and `cleanup` removes that name only.
    let linked = retry_rename(|| (seams.link)(&names.temp, target), &mut *seams.sleep);
    drop(cleanup);
    match linked {
        Ok(()) => Ok(ReplaceOutcome::Replaced),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(ReplaceOutcome::Conflict),
        Err(e) => Err(link_failure(e)),
    }
}

/// Replaces `target` only while it holds `expected`; see [`PlainFileIo::replace_if`].
fn replace_expected(
    target: &Path,
    names: &SiblingNames,
    cleanup: TempCleanup,
    expected: &[u8; 32],
    max_len: u64,
    seams: &mut Seams<'_>,
    #[cfg(unix)] exchange_working: bool,
) -> Result<ReplaceOutcome> {
    // The final check: exactly the expected content, read on the opened handle.
    let Some(current) = read_current(target, max_len)? else {
        return Ok(ReplaceOutcome::Conflict);
    };
    if hash(&current) != *expected {
        return Ok(ReplaceOutcome::Conflict);
    }
    (seams.before_install)();
    #[cfg(unix)]
    if exchange_working {
        match rename_with(&names.temp, target, rustix::fs::RenameFlags::EXCHANGE)? {
            Swap::Done => {
                // The exchange working name now holds whatever was at the target at the instant of the swap,
                // possibly another writer's vault: it is no longer ours to delete.
                cleanup.disarm();
                // Moved to its own `.exchanged` name, hidden from scans until it settles.
                let (at, _active) = settle_exchanged(names, seams);
                return Ok(judge_swapped_out(
                    &at, names, expected, max_len, seams, true,
                ));
            }
            Swap::Refused => return Ok(ReplaceOutcome::Conflict),
            Swap::Unsupported => {}
        }
    }
    // Move-aside path. Nothing has been renamed yet; without hard links refuse here.
    probe_links(names, seams)?;
    // Capture whatever is at the target when the move lands, including another writer's
    // version. Our active working name stays hidden from scans until installation has settled.
    let Some((aside, _active_aside)) = move_aside(target, names, seams)? else {
        // The file vanished (a writer is mid-replace, or deleted it): nothing was moved.
        return Ok(ReplaceOutcome::Conflict);
    };
    let aside = aside.as_path();
    (seams.after_aside)();
    let linked = retry_rename(|| (seams.link)(&names.temp, target), &mut *seams.sleep);
    match linked {
        // Our bytes are at the path; the temp name is a second link `cleanup` removes.
        Ok(()) => Ok(judge_swapped_out(
            aside, names, expected, max_len, seams, true,
        )),
        // Another writer created the file while the path was empty: theirs stays, ours is not
        // installed. What we moved aside is judged like a swapped-out file.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(judge_swapped_out(
            aside, names, expected, max_len, seams, false,
        )),
        Err(e) => {
            // Ours is not installed. Restore the old path without replacing anything and
            // attempt to publish the moved-aside file. A successful restore makes any retained
            // name a second link to the vault, even when publication or cleanup fails.
            let restored = (seams.link)(aside, target).is_ok();
            drop(cleanup);
            let published = publish_displaced(aside, names, seams);
            Err(if restored {
                io_error(e)
            } else {
                let kept = published
                    .as_ref()
                    .map_or(aside, |p| p.leftover.as_deref().unwrap_or(&p.path));
                retained_error(&e, kept)
            })
        }
    }
}

/// How many `.displaced` (or `.aside`, `.exchanged`) names are tried when the planned one is
/// taken.
const PUBLISH_ATTEMPTS: usize = 8;

/// Moves the file at `target` to a fresh `.aside` working name **without ever replacing a file
/// that already has that name** (another program may have put its own file there first).
/// `Ok(None)`: the target vanished and nothing was moved. A taken name draws a fresh one; when
/// no name can be had, nothing is moved and the replacement fails closed.
///
/// Prefer the atomic no-replace move. Unix volumes without it can use an exclusive hard
/// link followed by unlink, with a source identity check before unlink. Without hard links
/// the earlier probe fails closed.
fn move_aside(
    target: &Path,
    names: &SiblingNames,
    seams: &mut Seams<'_>,
) -> Result<Option<(PathBuf, ActiveAside)>> {
    let mut candidate = names.aside.clone();
    for attempt in 0..PUBLISH_ATTEMPTS {
        if attempt > 0 {
            candidate = names.fresh()?.aside;
        }
        let active = ActiveAside::begin(&candidate);
        match retry_rename(|| (seams.noreplace)(target, &candidate), &mut *seams.sleep) {
            Ok(()) => return Ok(Some((candidate, active))),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.kind() == io::ErrorKind::Unsupported => {}
            Err(e) => return Err(io_error(e)),
        }
        #[cfg(not(unix))]
        return Err(unsupported("no-replace move unavailable"));
        #[cfg(unix)]
        match link_aside(target, &candidate, seams) {
            Ok(()) => return Ok(Some((candidate, active))),
            Err(KdbxError::Io {
                kind: io::ErrorKind::AlreadyExists,
                ..
            }) => continue,
            Err(KdbxError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            }) => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    Err(io_error(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free working name to move the vault aside",
    )))
}

/// Link the source without replacing the destination. Check the identity of both names
/// before unlinking the source: an atomic writer may have replaced either one while linking.
#[cfg(unix)]
fn link_aside(target: &Path, candidate: &Path, seams: &mut Seams<'_>) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    retry_rename(|| (seams.link)(target, candidate), &mut *seams.sleep).map_err(link_failure)?;
    let mut unlink_source = || {
        let linked = fs::symlink_metadata(candidate)?;
        let source = fs::symlink_metadata(target)?;
        if !linked.is_file()
            || !source.is_file()
            || source.dev() != linked.dev()
            || source.ino() != linked.ino()
        {
            return Err(io::Error::other(
                "source or working copy changed after linking",
            ));
        }
        (seams.unlink)(target)
    };
    // After a successful link, never retry elsewhere or clean up the working file on error.
    unlink_source().map_err(|e| retained_error(&e, candidate))
}

/// Move the swapped-out file to the ordinary `.exchanged` name without replacing anything.
/// If this fails, the original exchange working name already makes the copy discoverable.
#[cfg(unix)]
fn settle_exchanged(names: &SiblingNames, seams: &mut Seams<'_>) -> (PathBuf, Option<ActiveAside>) {
    let mut candidate = names.exchanged.clone();
    for attempt in 0..PUBLISH_ATTEMPTS {
        if attempt > 0 {
            let Ok(fresh) = names.fresh() else {
                break;
            };
            candidate = fresh.exchanged;
        }
        let active = ActiveAside::begin(&candidate);
        match retry_rename(
            || (seams.noreplace)(&names.temp, &candidate),
            &mut *seams.sleep,
        ) {
            Ok(()) => return (candidate, Some(active)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => break,
        }
    }
    (names.temp.clone(), None)
}

/// Removes one name of a file (the verified expected content). Its single call site keeps the
/// mutation guard's allowance at one.
fn drop_name(at: &Path) -> io::Result<()> {
    fs::remove_file(at)
}

/// A copy published under a listed name by [`publish_displaced`].
struct Published {
    /// The `.displaced` name now holding the data.
    path: PathBuf,
    /// The working name that could not be removed afterwards (a warning, never silent).
    leftover: Option<PathBuf>,
}

/// Publishes the settled file `at` under a `.displaced` name without ever replacing anything: a
/// no-replace hard link (it fails with `AlreadyExists` on an occupied name, so another
/// process's retained vault is never overwritten), a fresh unique name when the planned one is
/// taken, and only after the link holds is the working name dropped (the data stays reachable
/// under the new name). A working name that cannot be dropped is reported as `leftover`, and
/// the copy stays published. `None` when no name could be made; the file then stays where it is.
fn publish_displaced(at: &Path, names: &SiblingNames, seams: &mut Seams<'_>) -> Option<Published> {
    let mut candidate = names.displaced.clone();
    for _ in 0..PUBLISH_ATTEMPTS {
        match retry_rename(|| (seams.link)(at, &candidate), &mut *seams.sleep) {
            Ok(()) => {
                let leftover = (seams.unlink)(at).is_err().then(|| at.to_path_buf());
                return Some(Published {
                    path: candidate,
                    leftover,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                candidate = names.fresh().ok()?.displaced;
            }
            Err(_) => return None,
        }
    }
    None
}

/// The install failed and the original path could not be restored. Keep the exact path
/// internally even though the error display shows just the file name.
fn retained_error(e: &io::Error, kept: &Path) -> KdbxError {
    KdbxError::VaultRetained {
        kind: e.kind(),
        path: kept.to_path_buf(),
        message: e.to_string(),
    }
}

/// `e` with the path of the retained file attached (an error of another kind is returned as is).
fn with_retained_path(e: KdbxError, path: &Path) -> KdbxError {
    match e {
        KdbxError::Io { kind, message } => KdbxError::VaultRetained {
            kind,
            path: path.to_path_buf(),
            message,
        },
        other => other,
    }
}

/// Judges the file that was at the target the instant ours went in (`at`: the `.exchanged` name,
/// or the exchange working name when it could not be moved there; the `.aside` name
/// after a move aside). Only a file read back and proved to
/// be the expected content is removed; anything else (another writer's version, or a file that
/// cannot be read) is kept and returned as `Displaced`. Never fails: a file that cannot be
/// renamed to the `.displaced` name stays where it is (under its working name). `installed` tells whether our bytes are
/// at the path (`false`: another writer created it meanwhile).
fn judge_swapped_out(
    at: &Path,
    names: &SiblingNames,
    expected: &[u8; 32],
    max_len: u64,
    seams: &mut Seams<'_>,
    installed: bool,
) -> ReplaceOutcome {
    let content = read_current(at, max_len).ok().flatten();
    if content
        .as_ref()
        .is_some_and(|bytes| hash(bytes) == *expected)
    {
        // The content we checked and the caller already merged into the new bytes.
        // Best effort: the content is the caller's own, already merged into the new bytes; a
        // leftover working name only holds a copy of what the vault now contains.
        let _ = (seams.unlink)(at);
        return if installed {
            ReplaceOutcome::Replaced
        } else {
            ReplaceOutcome::Conflict
        };
    }
    // Settled: the file is kept for good, so it takes a finalized `.displaced` name, never
    // replacing a file that is already there.
    // Keep it under its current, recognisable name rather than lose it when no name can be made.
    let (kept, leftover) = match publish_displaced(at, names, seams) {
        Some(p) => (p.path, p.leftover),
        None => (at.to_path_buf(), None),
    };
    ReplaceOutcome::Displaced {
        bytes: content.unwrap_or_default(),
        kept,
        leftover,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_util::FailingFileIo, Database, KdbxError};
    use secrecy::SecretString;
    use std::{
        fs,
        io::{Error, ErrorKind},
        sync::Arc,
        time::{Duration, SystemTime},
    };
    use tempfile::TempDir;

    // Records the guard and commit boundary without using a clock or background worker.
    struct RecordingIo {
        events: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }
    impl FileIo for RecordingIo {
        fn read(&self, path: &Path) -> Result<Vec<u8>> {
            self.events.lock().unwrap().push("read");
            PlainFileIo.read(path)
        }
        fn metadata(&self, path: &Path) -> Result<FileMeta> {
            self.events.lock().unwrap().push("metadata");
            PlainFileIo.metadata(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
            self.events.lock().unwrap().push("write");
            PlainFileIo.write_atomic(path, bytes)
        }
    }
    #[test]
    fn guard_runs_after_serialisation_immediately_before_write() {
        let (_d, mut f, db) = fixture();
        let events = Arc::new(std::sync::Mutex::new(vec![]));
        f.io = Arc::new(RecordingIo {
            events: events.clone(),
        });
        assert_eq!(
            f.save_serializing(|| {
                let bytes = db.save(&password())?;
                events.lock().unwrap().push("serialized");
                Ok(bytes)
            })
            .unwrap(),
            SaveOutcome::Saved
        );
        assert_eq!(
            *events.lock().unwrap(),
            ["serialized", "metadata", "read", "write", "metadata"]
        );
    }
    #[test]
    fn change_during_serialisation_is_not_overwritten() {
        let (_d, mut f, db, io) = injected();
        let path = f.path().to_owned();
        let old = f.stamp.clone();
        assert_eq!(
            f.save_serializing(|| {
                let bytes = db.save(&password())?;
                fs::write(&path, b"external during serialization").unwrap();
                Ok(bytes)
            })
            .unwrap(),
            SaveOutcome::ChangedOnDisk
        );
        assert_eq!(fs::read(&path).unwrap(), b"external during serialization");
        assert_eq!(io.write_calls(), 0);
        assert_eq!(f.stamp, old);
    }
    struct PostCommitIo {
        inner: Arc<FailingFileIo>,
        sync_error: Option<ErrorKind>,
    }
    impl FileIo for PostCommitIo {
        fn read(&self, path: &Path) -> Result<Vec<u8>> {
            self.inner.read(path)
        }
        fn metadata(&self, path: &Path) -> Result<FileMeta> {
            self.inner.metadata(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
            if let Some(kind) = self.sync_error {
                write_atomic_with_sync(
                    path,
                    bytes,
                    |s, t| fs::rename(s, t),
                    |_| panic!("sleep"),
                    |_| Err(kind.into()),
                )
            } else {
                self.inner.write_atomic(path, bytes)?;
                self.inner.fail_metadata(ErrorKind::PermissionDenied);
                Ok(())
            }
        }
    }
    #[test]
    fn post_commit_metadata_failure_forces_reload_on_next_save() {
        let (_d, mut f, db, io) = injected();
        let old = f.stamp.clone();
        let before = fs::read(f.path()).unwrap();
        f.io = Arc::new(PostCommitIo {
            inner: io.clone(),
            sync_error: None,
        });
        io_kind(f.save(&db, &password()), ErrorKind::PermissionDenied);
        assert_ne!(fs::read(f.path()).unwrap(), before);
        assert_eq!(f.stamp, old);
        io.heal();
        assert!(f.has_changed_on_disk().unwrap());
        assert_eq!(
            f.save(&db, &password()).unwrap(),
            SaveOutcome::ChangedOnDisk
        );
        assert_eq!(io.write_calls(), 1);
    }
    #[test]
    fn directory_sync_unsupported_and_invalid_input_are_success() {
        for kind in [ErrorKind::Unsupported, ErrorKind::InvalidInput] {
            let (_d, mut f, db, io) = injected();
            let old = f.stamp.clone();
            f.io = Arc::new(PostCommitIo {
                inner: io,
                sync_error: Some(kind),
            });
            assert_eq!(f.save(&db, &password()).unwrap(), SaveOutcome::Saved);
            assert_ne!(f.stamp, old);
            assert!(!f.has_changed_on_disk().unwrap());
        }
    }
    #[test]
    fn directory_sync_other_error_keeps_old_stamp_after_commit() {
        let (_d, mut f, db, io) = injected();
        let old = f.stamp.clone();
        let before = fs::read(f.path()).unwrap();
        f.io = Arc::new(PostCommitIo {
            inner: io,
            sync_error: Some(ErrorKind::PermissionDenied),
        });
        io_kind(f.save(&db, &password()), ErrorKind::PermissionDenied);
        assert_ne!(fs::read(f.path()).unwrap(), before);
        assert_eq!(f.stamp, old);
        assert!(f.has_changed_on_disk().unwrap());
        assert_eq!(
            f.save(&db, &password()).unwrap(),
            SaveOutcome::ChangedOnDisk
        );
    }
    #[test]
    fn cleanup_does_not_remove_reused_temp_path_after_rename() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("vault");
        let mut reused = None;
        write_atomic_using(
            &path,
            b"new",
            |source, target| {
                fs::rename(source, target)?;
                fs::write(source, b"another owner")?;
                reused = Some(source.to_owned());
                Ok(())
            },
            |_| panic!("sleep"),
        )
        .unwrap();
        assert_eq!(fs::read(reused.unwrap()).unwrap(), b"another owner");
        assert_eq!(fs::read(path).unwrap(), b"new");
    }
    #[cfg(unix)]
    #[test]
    fn unix_resolution_permission_denied_never_sleeps() {
        let mut calls = 0;
        let mut sleeps = vec![];
        let result = resolve_using(
            || {
                calls += 1;
                Err(ErrorKind::PermissionDenied.into())
            },
            |delay| sleeps.push(delay),
        );
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
        assert_eq!(calls, 1);
        assert!(sleeps.is_empty());
    }

    fn password() -> SecretString {
        "synthetic-test-password".into()
    }
    fn fixture() -> (TempDir, VaultFile, Database) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.kdbx");
        let db = Database::new("synthetic");
        fs::write(&path, db.save(&password()).unwrap()).unwrap();
        let (file, db) = VaultFile::open(&path, &password()).unwrap();
        (dir, file, db)
    }
    fn injected() -> (TempDir, VaultFile, Database, Arc<FailingFileIo>) {
        let (dir, file, _) = fixture();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        let (file, db) = VaultFile::open_with(io.clone(), file.path(), &password()).unwrap();
        (dir, file, db, io)
    }
    fn only_target(dir: &TempDir) {
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    fn io_kind<T: std::fmt::Debug>(result: crate::Result<T>, expected: ErrorKind) {
        assert!(matches!(result, Err(KdbxError::Io {kind, ..}) if kind == expected));
    }
    /// The retained file name (and failure kind) of a [`KdbxError::VaultRetained`].
    fn retained<T: std::fmt::Debug>(result: crate::Result<T>) -> (ErrorKind, String) {
        match result {
            Err(KdbxError::VaultRetained { kind, path, .. }) => (
                kind,
                path.file_name().unwrap().to_string_lossy().into_owned(),
            ),
            other => panic!("expected VaultRetained, got {other:?}"),
        }
    }
    #[test]
    fn open_records_stamp_and_reports_unchanged() {
        let (_d, f, _) = fixture();
        assert!(!f.has_changed_on_disk().unwrap());
        assert_eq!(f.stamp.len, fs::metadata(f.path()).unwrap().len());
        assert_eq!(f.stamp.sha256, hash(&fs::read(f.path()).unwrap()));
        assert!(!format!("{f:?}").contains("synthetic-test-password"));
    }
    #[test]
    fn external_overwrite_with_different_len_is_detected() {
        let (_d, f, _) = fixture();
        fs::write(f.path(), b"external").unwrap();
        assert!(f.has_changed_on_disk().unwrap());
    }
    #[test]
    fn external_overwrite_same_len_different_bytes_is_detected() {
        let (_d, f, _) = fixture();
        let mut bytes = fs::read(f.path()).unwrap();
        bytes[0] ^= 1;
        fs::write(f.path(), bytes).unwrap();
        fs::File::options()
            .write(true)
            .open(f.path())
            .unwrap()
            .set_modified(f.stamp.modified.unwrap())
            .unwrap();
        assert!(f.has_changed_on_disk().unwrap());
    }
    #[test]
    fn touch_without_content_change_counts_as_changed_via_mtime() {
        let (_d, f, _) = fixture();
        fs::File::options()
            .write(true)
            .open(f.path())
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(100))
            .unwrap();
        assert!(f.has_changed_on_disk().unwrap()); // Conservative even when content is unchanged.
    }
    #[test]
    fn deleted_file_counts_as_changed() {
        let (_d, f, _) = fixture();
        fs::remove_file(f.path()).unwrap();
        assert!(f.has_changed_on_disk().unwrap());
    }
    #[test]
    fn save_refuses_when_changed_and_leaves_file_untouched() {
        let (_d, mut f, db, io) = injected();
        fs::write(f.path(), b"external").unwrap();
        assert_eq!(
            f.save(&db, &password()).unwrap(),
            SaveOutcome::ChangedOnDisk
        );
        assert_eq!(fs::read(f.path()).unwrap(), b"external");
        assert_eq!(io.write_calls(), 0);
    }
    #[test]
    fn save_writes_and_second_save_is_unchanged() {
        let (_d, mut f, db) = fixture();
        let old = f.stamp.clone();
        assert_eq!(f.save(&db, &password()).unwrap(), SaveOutcome::Saved);
        assert_ne!(old.sha256, f.stamp.sha256);
        assert!(!f.has_changed_on_disk().unwrap());
        assert_eq!(f.save(&db, &password()).unwrap(), SaveOutcome::Saved);
        assert!(!f.has_changed_on_disk().unwrap());
    }
    #[test]
    fn save_overwriting_replaces_external_change() {
        let (_d, mut f, db) = fixture();
        fs::write(f.path(), b"external").unwrap();
        f.save_overwriting(&db, &password()).unwrap();
        assert!(!f.has_changed_on_disk().unwrap());
        assert_eq!(
            Database::open(&fs::read(f.path()).unwrap(), &password()).unwrap(),
            db
        );
    }
    #[test]
    fn reload_returns_external_content_and_refreshes_stamp() {
        let (_d, mut f, _) = fixture();
        let db = Database::new("external");
        fs::write(f.path(), db.save(&password()).unwrap()).unwrap();
        assert_eq!(f.reload(&password()).unwrap(), db);
        assert!(!f.has_changed_on_disk().unwrap());
    }
    #[test]
    fn write_atomic_replaces_existing_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(fs::read(p).unwrap(), b"new");
    }
    #[test]
    fn write_atomic_leaves_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        write_atomic(&d.path().join("vault"), b"new").unwrap();
        only_target(&d);
    }
    #[cfg(unix)]
    #[test]
    fn write_atomic_failure_leaves_original_intact() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = write_atomic(&p, b"new");
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&p).unwrap(), b"old");
        only_target(&d);
    }
    #[cfg(unix)]
    #[test]
    fn write_atomic_follows_symlink() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        let link = d.path().join("link");
        fs::write(&p, b"old").unwrap();
        std::os::unix::fs::symlink(&p, &link).unwrap();
        write_atomic(&link, b"new").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&p).unwrap(), b"new");
        fs::write(&p, Database::new("synthetic").save(&password()).unwrap()).unwrap();
        let (f, _) = VaultFile::open(&link, &password()).unwrap();
        assert_eq!(f.path(), fs::canonicalize(p).unwrap());
    }
    #[test]
    fn write_atomic_works_with_path_containing_spaces_and_cyrillic() {
        let d = tempfile::tempdir().unwrap();
        let p = d
            .path()
            .join("\u{43c}\u{43e}\u{439} \u{441}\u{435}\u{439}\u{444}");
        fs::create_dir(&p).unwrap();
        let p = p.join("a b.kdbx");
        write_atomic(&p, b"new").unwrap();
        assert_eq!(fs::read(p).unwrap(), b"new");
    }
    #[test]
    fn crlf_bytes_are_written_verbatim() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        write_atomic(&p, b"\0\r\n\xff").unwrap();
        assert_eq!(fs::read(p).unwrap(), b"\0\r\n\xff");
    }
    #[test]
    fn wrong_password_open_does_not_create_vaultfile() {
        let (_d, f, _) = fixture();
        let before = fs::read(f.path()).unwrap();
        assert!(matches!(
            VaultFile::open(f.path(), &"wrong".into()),
            Err(KdbxError::WrongPassword)
        ));
        assert_eq!(fs::read(f.path()).unwrap(), before);
    }
    #[test]
    fn retryable_error_set() {
        assert!(is_retryable_rename_error(&Error::from(
            ErrorKind::PermissionDenied
        )));
        for kind in [ErrorKind::NotFound, ErrorKind::InvalidData] {
            assert!(!is_retryable_rename_error(&Error::from(kind)));
        }
        #[cfg(windows)]
        {
            for n in [5, 32, 33] {
                assert!(is_retryable_rename_error(&Error::from_raw_os_error(n)));
            }
            assert!(!is_retryable_rename_error(&Error::from_raw_os_error(2)));
        }
    }
    #[test]
    fn retry_schedule_sums_to_about_three_seconds() {
        assert_eq!(RENAME_RETRY_SCHEDULE_MS, [50, 100, 200, 400, 800, 1600]);
        assert_eq!(RENAME_RETRY_SCHEDULE_MS.iter().sum::<u64>(), 3150);
        assert!(RENAME_RETRY_SCHEDULE_MS.windows(2).all(|v| v[0] < v[1]));
    }
    #[test]
    fn retry_stops_on_first_success() {
        let mut calls = 0;
        let mut sleeps = vec![];
        retry_rename(
            || {
                calls += 1;
                if calls < 3 {
                    Err(ErrorKind::PermissionDenied.into())
                } else {
                    Ok(())
                }
            },
            |d| sleeps.push(d),
        )
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(
            sleeps,
            [Duration::from_millis(50), Duration::from_millis(100)]
        );
    }
    #[test]
    fn retry_exhaustion_returns_last_error_after_all_sleeps() {
        let mut calls = 0;
        let mut sleeps = vec![];
        let e = retry_rename(
            || {
                calls += 1;
                Err::<(), _>(Error::new(ErrorKind::PermissionDenied, calls.to_string()))
            },
            |d| sleeps.push(d),
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "7");
        assert_eq!(calls, 7);
        assert_eq!(sleeps, RENAME_RETRY_SCHEDULE_MS.map(Duration::from_millis));
    }
    #[test]
    fn non_retryable_error_fails_immediately_without_sleeping() {
        let mut calls = 0;
        let e = retry_rename(
            || {
                calls += 1;
                Err::<(), Error>(ErrorKind::NotFound.into())
            },
            |_| panic!("unexpected sleep"),
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(e.kind(), ErrorKind::NotFound);
    }
    #[test]
    fn plain_file_io_read_write_metadata_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        PlainFileIo.write_atomic(&p, b"new").unwrap();
        assert_eq!(PlainFileIo.read(&p).unwrap(), b"new");
        assert_eq!(PlainFileIo.metadata(&p).unwrap().len, 3);
    }
    #[test]
    fn plain_file_io_metadata_of_missing_file_is_not_found() {
        let d = tempfile::tempdir().unwrap();
        io_kind(
            PlainFileIo.metadata(&d.path().join("absent")),
            ErrorKind::NotFound,
        );
    }
    #[test]
    fn open_with_uses_the_injected_io() {
        let (_d, _f, _db, io) = injected();
        assert!(io.read_calls() >= 1);
        assert!(io.metadata_calls() >= 1);
    }
    #[test]
    fn failing_write_leaves_stamp_and_file_untouched() {
        let (_d, mut f, db, io) = injected();
        let stamp = f.stamp.clone();
        let bytes = fs::read(f.path()).unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        io_kind(f.save(&db, &password()), ErrorKind::PermissionDenied);
        assert_eq!(io.write_calls(), 1);
        assert_eq!(stamp, f.stamp);
        assert_eq!(fs::read(f.path()).unwrap(), bytes);
        assert!(!f.has_changed_on_disk().unwrap());
    }
    #[test]
    fn failing_read_on_reload_returns_io_error() {
        let (_d, mut f, _, io) = injected();
        let stamp = f.stamp.clone();
        io.fail_read(ErrorKind::PermissionDenied);
        io_kind(f.reload(&password()), ErrorKind::PermissionDenied);
        assert_eq!(stamp, f.stamp);
    }
    #[test]
    fn failing_metadata_makes_has_changed_on_disk_error() {
        let (_d, f, _, io) = injected();
        io.fail_metadata(ErrorKind::PermissionDenied);
        io_kind(f.has_changed_on_disk(), ErrorKind::PermissionDenied);
    }
    #[test]
    fn heal_restores_normal_behaviour() {
        let (_d, mut f, db, io) = injected();
        io.fail_read(ErrorKind::PermissionDenied);
        io.fail_metadata(ErrorKind::PermissionDenied);
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        io.heal();
        assert_eq!(f.save(&db, &password()).unwrap(), SaveOutcome::Saved);
        f.reload(&password()).unwrap();
        assert!(!f.has_changed_on_disk().unwrap());
    }
    #[cfg(unix)]
    #[test]
    fn new_temp_is_private_and_replace_preserves_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        write_atomic(&p, b"secret").unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&p, fs::Permissions::from_mode(0o400)).unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o400
        );
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn rename_failure_cleans_temp_after_creation() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::create_dir(&p).unwrap();
        assert!(write_atomic(&p, b"new").is_err());
        assert!(p.is_dir());
        only_target(&d);
    }
    #[test]
    fn retry_exhaustion_cleans_temp_and_keeps_original() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let mut sleeps = vec![];
        let mut attempts = 0;
        let result = write_atomic_using(
            &p,
            b"new",
            |source, target| {
                assert_eq!(fs::read(source).unwrap(), b"new");
                assert_eq!(target, fs::canonicalize(&p).unwrap());
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(
                        fs::metadata(source).unwrap().permissions().mode() & 0o777,
                        0o600
                    );
                }
                attempts += 1;
                Err(ErrorKind::PermissionDenied.into())
            },
            |delay| sleeps.push(delay),
        );
        io_kind(result, ErrorKind::PermissionDenied);
        assert_eq!(attempts, 7);
        assert_eq!(sleeps, RENAME_RETRY_SCHEDULE_MS.map(Duration::from_millis));
        assert_eq!(fs::read(&p).unwrap(), b"old");
        only_target(&d);
    }
    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_not_replaced() {
        let d = tempfile::tempdir().unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(d.path().join("missing"), &link).unwrap();
        io_kind(write_atomic(&link, b"secret"), ErrorKind::NotFound);
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        only_target(&d);
    }
    #[cfg(windows)]
    #[test]
    fn rename_over_open_reader_succeeds() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let _reader = fs::File::open(&p).unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        only_target(&d);
    }
    #[cfg(windows)]
    #[test]
    fn rename_over_exclusive_handle_is_retried_then_fails_cleanly() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&p)
            .unwrap();
        assert!(matches!(
            write_atomic(&p, b"new"),
            Err(KdbxError::Io { .. })
        ));
        drop(handle);
        assert_eq!(fs::read(&p).unwrap(), b"old");
        only_target(&d);
    }
    #[cfg(windows)]
    #[test]
    fn rename_succeeds_when_exclusive_handle_released_mid_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&p)
            .unwrap();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(handle);
        });
        let result = write_atomic(&p, b"new");
        worker.join().unwrap();
        result.unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        only_target(&d);
    }

    // --- Guarded replacement (`replace_if`) ---

    const LIMIT: u64 = 1 << 20;

    /// A guarded replacement with injected races and failures (the [`Seams`]); by default the
    /// real system calls and no sleeping.
    type Hook<'a> = Box<dyn FnMut() + 'a>;
    type Call<'a> = Box<dyn FnMut(&Path, &Path) -> io::Result<()> + 'a>;
    type Call1<'a> = Box<dyn FnMut(&Path) -> io::Result<()> + 'a>;
    struct Race<'a> {
        exchange: bool,
        before: Hook<'a>,
        after_aside: Hook<'a>,
        noreplace: Call<'a>,
        link: Call<'a>,
        unlink: Call1<'a>,
        sync_dir: Call1<'a>,
        sleep: Box<dyn FnMut(Duration) + 'a>,
    }
    impl<'a> Race<'a> {
        fn new(exchange: bool) -> Self {
            Self {
                exchange,
                before: Box::new(|| {}),
                after_aside: Box::new(|| {}),
                noreplace: Box::new(|a: &Path, b: &Path| rename_noreplace(a, b)),
                link: Box::new(|a: &Path, b: &Path| fs::hard_link(a, b)),
                unlink: Box::new(|a: &Path| drop_name(a)),
                sync_dir: Box::new(|a: &Path| sync_directory(a)),
                sleep: Box::new(|_| {}),
            }
        }
        fn run(
            &mut self,
            p: &Path,
            expected: Option<&[u8]>,
            bytes: &[u8],
        ) -> crate::Result<ReplaceOutcome> {
            let expected = expected.map(hash);
            let mut seams = Seams {
                before_install: &mut *self.before,
                after_aside: &mut *self.after_aside,
                exchange: self.exchange,
                noreplace: &mut *self.noreplace,
                link: &mut *self.link,
                unlink: &mut *self.unlink,
                sync_dir: &mut *self.sync_dir,
                sleep: &mut *self.sleep,
            };
            replace_if_with(p, expected.as_ref(), bytes, LIMIT, &mut seams)
        }
    }
    fn replace(
        p: &Path,
        expected: Option<&[u8]>,
        bytes: &[u8],
        hook: &mut dyn FnMut(),
        exchange: bool,
    ) -> ReplaceOutcome {
        let mut race = Race::new(exchange);
        race.before = Box::new(hook);
        race.run(p, expected, bytes).unwrap()
    }
    fn modes() -> Vec<bool> {
        if cfg!(unix) {
            vec![true, false]
        } else {
            vec![false]
        }
    }
    /// Every name in `dir` with its content, sorted.
    fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut all: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        all.sort();
        all
    }
    /// Whether some file in `dir` holds `bytes`.
    fn holds(dir: &Path, bytes: &[u8]) -> bool {
        listing(dir).iter().any(|(_, b)| b == bytes)
    }
    fn unsupported_link() -> Call<'static> {
        Box::new(|_: &Path, _: &Path| Err(Error::from(ErrorKind::Unsupported)))
    }
    #[test]
    fn replace_if_writes_only_over_the_expected_content() {
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault");
            fs::write(&p, b"old").unwrap();
            assert_eq!(
                replace(&p, Some(b"other"), b"new", &mut || {}, exchange),
                ReplaceOutcome::Conflict
            );
            assert_eq!(fs::read(&p).unwrap(), b"old");
            only_target(&d);
            assert_eq!(
                replace(&p, Some(b"old"), b"new", &mut || {}, exchange),
                ReplaceOutcome::Replaced
            );
            assert_eq!(fs::read(&p).unwrap(), b"new");
            only_target(&d);
            // Through the trait as well.
            assert_eq!(
                PlainFileIo
                    .replace_if(&p, Some(&hash(b"new")), b"newer", LIMIT)
                    .unwrap(),
                ReplaceOutcome::Replaced
            );
            assert_eq!(fs::read(&p).unwrap(), b"newer");
            only_target(&d);
        }
    }
    #[test]
    fn replace_if_missing_target_is_created_but_never_clobbered() {
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault");
            assert_eq!(
                replace(&p, None, b"ours", &mut || {}, exchange),
                ReplaceOutcome::Replaced
            );
            assert_eq!(fs::read(&p).unwrap(), b"ours");
            only_target(&d);
            // A file that exists is never replaced by an initial upload.
            assert_eq!(
                replace(&p, None, b"again", &mut || {}, exchange),
                ReplaceOutcome::Conflict
            );
            assert_eq!(fs::read(&p).unwrap(), b"ours");
            only_target(&d);
        }
    }
    #[test]
    fn file_created_during_an_initial_upload_is_never_clobbered() {
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault");
            let racer = p.clone();
            let outcome = replace(
                &p,
                None,
                b"ours",
                &mut || write_atomic(&racer, b"theirs").unwrap(),
                exchange,
            );
            assert_eq!(outcome, ReplaceOutcome::Conflict, "exchange={exchange}");
            assert_eq!(fs::read(&p).unwrap(), b"theirs");
            only_target(&d);
        }
    }
    #[test]
    fn writer_racing_the_final_step_is_kept_and_returned_not_destroyed() {
        // Both the exchange and the move-aside path: a writer committing between the final
        // check and the replacement is kept, never overwritten.
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault");
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let outcome = replace(
                &p,
                Some(b"old"),
                b"new",
                &mut || write_atomic(&racer, b"theirs").unwrap(),
                exchange,
            );
            let ReplaceOutcome::Displaced { bytes, kept, .. } = outcome else {
                panic!("expected Displaced, got {outcome:?} (exchange={exchange})");
            };
            assert_eq!(bytes, b"theirs");
            assert_eq!(fs::read(&p).unwrap(), b"new");
            assert_eq!(fs::read(&kept).unwrap(), b"theirs", "kept on disk");
            assert!(kept.to_string_lossy().ends_with(".displaced"));
            assert_eq!(
                kept.parent(),
                Some(d.path().canonicalize().unwrap().as_path())
            );
            assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
        }
    }
    #[test]
    fn a_real_displaced_copy_is_listed_and_nothing_is_removed() {
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault.kdbx");
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let outcome = replace(
                &p,
                Some(b"old"),
                b"new",
                &mut || write_atomic(&racer, b"theirs").unwrap(),
                exchange,
            );
            let ReplaceOutcome::Displaced { kept, .. } = outcome else {
                panic!("expected Displaced, got {outcome:?}");
            };
            // Another vault and a temp file are excluded; a working name is included separately.
            fs::write(d.path().join("other.kdbx"), b"x").unwrap();
            fs::write(d.path().join(".other.kdbx.1.2.3.displaced"), b"x").unwrap();
            fs::write(d.path().join(".vault.kdbx.1.2.3.tmp"), b"x").unwrap();
            fs::write(d.path().join(".vault.kdbx.1.2.3.aside"), b"x").unwrap();
            fs::write(d.path().join(".vault.kdbx.1.2.4.exchanged"), b"x").unwrap();
            let before = listing(d.path());
            let listed = displaced_copies(&p).unwrap();
            assert_eq!(listed.len(), 3, "exchange={exchange}");
            assert!(listed.iter().any(|c| c.kind == DisplacedCopyKind::Exchanged
                && c.path.ends_with(".vault.kdbx.1.2.4.exchanged")));
            let displaced = listed
                .iter()
                .find(|c| c.kind == DisplacedCopyKind::Displaced)
                .unwrap();
            let aside = listed
                .iter()
                .find(|c| c.kind == DisplacedCopyKind::Aside)
                .unwrap();
            assert_eq!(displaced.path, kept);
            assert_eq!(displaced.len, 6);
            assert!(aside.path.ends_with(".vault.kdbx.1.2.3.aside"));
            assert_eq!(listing(d.path()), before, "listing changes nothing");
            assert_eq!(fs::read(&p).unwrap(), b"new");
        }
    }
    #[test]
    fn displaced_names_are_matched_strictly() {
        assert_eq!(
            displaced_kind("v.kdbx", ".v.kdbx.12.345.6.displaced"),
            Some(DisplacedCopyKind::Displaced)
        );
        assert_eq!(
            displaced_kind("v.kdbx", ".v.kdbx.12.345.6.aside"),
            Some(DisplacedCopyKind::Aside)
        );
        assert_eq!(
            displaced_kind("v.kdbx", ".v.kdbx.12.345.6.exchanged"),
            Some(DisplacedCopyKind::Exchanged)
        );
        for bad in [
            "v.kdbx.12.345.6.displaced",
            ".v.kdbx.12.345.displaced",
            ".v.kdbx.12.345.6.7.displaced",
            ".v.kdbx.12.x.6.displaced",
            ".v.kdbx..345.6.displaced",
            ".v.kdbx.12.345.6.displaced.bak",
            ".v.kdbx2.12.345.6.displaced",
            ".w.kdbx.12.345.6.displaced",
            ".v.kdbx.12.345.6.tmp",
            ".v.kdbx.12.345.6.link.tmp",
            ".w.kdbx.12.345.6.aside",
            ".w.kdbx.12.345.6.exchanged",
        ] {
            assert_eq!(displaced_kind("v.kdbx", bad), None, "{bad}");
        }
        // A name longer than the stem limit matches by its cut prefix, like it is generated:
        // two long names with the same first bytes share the listing (documented limit).
        let long = "é".repeat(80);
        let cut = sibling_stem(&long);
        assert!(cut.len() <= SIBLING_STEM_MAX && long.starts_with(cut));
        assert!(displaced_kind(&long, &format!(".{cut}.1.2.3.displaced")).is_some());
        let other = format!("{long}-other");
        assert!(displaced_kind(&other, &format!(".{cut}.1.2.3.displaced")).is_some());
    }
    #[test]
    fn a_missing_or_unreadable_folder_is_an_error_not_an_empty_list() {
        let d = tempfile::tempdir().unwrap();
        // An unplugged or removed folder: the scan cannot say "no copies".
        let missing = d.path().join("nope").join("vault.kdbx");
        assert!(displaced_copies(&missing).is_err());
        // A folder name that is a file cannot be read either.
        let file = d.path().join("plain");
        fs::write(&file, b"x").unwrap();
        assert!(displaced_copies(&file.join("vault.kdbx")).is_err());
        // A target without a file name cannot be resolved.
        assert!(displaced_copies(Path::new("/")).is_err());
        // A genuinely empty folder is an empty list.
        let ok = d.path().join("vault.kdbx");
        assert_eq!(displaced_copies(&ok).unwrap(), Vec::new());
    }
    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_read_is_an_error_not_an_empty_list() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("locked");
        fs::create_dir(&dir).unwrap();
        let p = dir.join("vault.kdbx");
        fs::write(&p, b"v").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
        let denied = fs::read_dir(&dir).is_err();
        let result = displaced_copies(&p);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        // Running as root reads the folder anyway; then there is nothing to assert.
        if denied {
            assert!(result.is_err());
        }
        assert_eq!(displaced_copies(&p).unwrap(), Vec::new());
    }
    #[cfg(unix)]
    #[test]
    fn a_symlink_with_a_displaced_name_is_not_listed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault.kdbx");
        fs::write(&p, b"v").unwrap();
        let outside = d.path().join("precious");
        fs::write(&outside, b"keep").unwrap();
        for suffix in ["displaced", "aside"] {
            let link = d.path().join(format!(".vault.kdbx.1.2.3.{suffix}"));
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(displaced_copies(&p).unwrap().is_empty());
            assert!(fs::symlink_metadata(&link).is_ok());
        }
    }
    #[test]
    fn a_file_in_flight_on_the_move_aside_path_is_never_listed() {
        // Our own active move-aside file is still hidden. After publication the competing
        // copy is visible.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault.kdbx");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let probe = p.clone();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
        let sink = seen.clone();
        let mut race = Race::new(false);
        race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
        race.after_aside = Box::new(move || {
            *sink.borrow_mut() = Some(displaced_copies(&probe).unwrap());
        });
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        drop(race);
        assert!(matches!(outcome, ReplaceOutcome::Displaced { .. }));
        assert_eq!(seen.borrow().as_ref().map(Vec::len), Some(0), "in flight");
        let settled = displaced_copies(&p).unwrap();
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].kind, DisplacedCopyKind::Displaced);
        assert_eq!(fs::read(&p).unwrap(), b"new");
    }
    #[test]
    fn a_failed_install_that_cannot_restore_names_the_published_copy_in_the_error() {
        // The install link and the restoring link both fail: the moved-aside vault may be the
        // only copy, so it is published as a listed `.displaced` copy, and the error carries
        // that file name (never a folder) so the user can find the vault.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault.kdbx");
        fs::write(&p, b"old").unwrap();
        let mut race = Race::new(false);
        let mut calls = 0;
        race.link = Box::new(move |a: &Path, b: &Path| {
            calls += 1;
            // 1 probe, 2 install, 3 restore fail; 4 (the publication) works.
            if calls == 2 || calls == 3 {
                return Err(Error::other("injected link failure"));
            }
            fs::hard_link(a, b)
        });
        let (kind, name) = retained(race.run(&p, Some(b"old"), b"new"));
        assert_eq!(kind, ErrorKind::Other);
        let listed = displaced_copies(&p).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(fs::read(&listed[0].path).unwrap(), b"old");
        assert_eq!(listed[0].path.file_name().unwrap().to_string_lossy(), name);
        assert!(!name.contains(std::path::MAIN_SEPARATOR));
        assert!(!p.exists());
        let names: Vec<_> = listing(d.path()).into_iter().map(|(n, _)| n).collect();
        assert!(!names.iter().any(|n| n.ends_with(".aside")), "{names:?}");
    }
    #[test]
    fn a_failure_that_cannot_even_publish_names_the_retained_file_in_the_error() {
        // Install, restore and publication all fail: the only copy stays under its working
        // name and the error says which file (the name only, no folder) holds it.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault.kdbx");
        fs::write(&p, b"old").unwrap();
        let mut race = Race::new(false);
        let mut calls = 0;
        race.link = Box::new(move |a: &Path, b: &Path| {
            calls += 1;
            if calls >= 2 {
                return Err(Error::other("injected link failure"));
            }
            fs::hard_link(a, b)
        });
        let (_, name) = retained(race.run(&p, Some(b"old"), b"new"));
        let kept: Vec<_> = listing(d.path())
            .into_iter()
            .filter(|(n, _)| n.ends_with(".aside"))
            .collect();
        assert_eq!(kept.len(), 1, "the vault is still on disk");
        assert_eq!(kept[0].1, b"old");
        assert_eq!(kept[0].0, name);
    }
    #[test]
    fn a_restored_install_failure_does_not_claim_the_vault_was_moved() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault.kdbx");
        fs::write(&p, b"old").unwrap();
        let mut race = Race::new(false);
        let mut calls = 0;
        race.link = Box::new(move |a: &Path, b: &Path| {
            calls += 1;
            // Probe and restore succeed. Install and publication fail.
            if calls == 2 || calls >= 4 {
                return Err(Error::other("injected link failure"));
            }
            fs::hard_link(a, b)
        });
        assert_eq!(
            race.run(&p, Some(b"old"), b"new"),
            Err(KdbxError::Io {
                kind: ErrorKind::Other,
                message: "injected link failure".into(),
            }),
        );
        assert_eq!(fs::read(&p).unwrap(), b"old");
    }

    #[test]
    fn a_failed_cleanup_after_publication_is_reported_not_silent() {
        // The copy is published, but its working name cannot be unlinked: the hidden name is a
        // second alias of the same data, so the outcome carries its path.
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault.kdbx");
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let mut race = Race::new(exchange);
            race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
            race.unlink = Box::new(|_: &Path| Err(Error::from(ErrorKind::PermissionDenied)));
            let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
            let ReplaceOutcome::Displaced { kept, leftover, .. } = &outcome else {
                panic!("expected Displaced, got {outcome:?} (exchange={exchange})");
            };
            assert!(kept.to_string_lossy().ends_with(".displaced"), "{kept:?}");
            let leftover = leftover.as_ref().expect("the leftover working name");
            assert!(leftover.exists(), "{leftover:?}");
            assert_eq!(fs::read(leftover).unwrap(), b"theirs");
            assert_eq!(outcome.retained_working_path(), Some(leftover.as_path()));
            assert_eq!(fs::read(kept).unwrap(), b"theirs");
            // The leftover is listed too: `.aside` after a move aside, `.exchanged` after an
            // exchange (never the temp name).
            let listed = displaced_copies(&p).unwrap();
            assert_eq!(listed.len(), 2, "exchange={exchange}");
            assert!(listed.iter().any(|copy| copy.path == *kept));
            assert!(listed.iter().any(|copy| copy.path == *leftover));
        }
    }
    #[test]
    fn a_failed_cleanup_after_a_restored_install_failure_keeps_both_links() {
        // Install fails, the restore works, the publication works, but the working name stays:
        // the vault is safe at its path and the working name still holds the same bytes.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault.kdbx");
        fs::write(&p, b"old").unwrap();
        let mut race = Race::new(false);
        let mut calls = 0;
        race.link = Box::new(move |a: &Path, b: &Path| {
            calls += 1;
            if calls == 2 {
                return Err(Error::other("injected link failure"));
            }
            fs::hard_link(a, b)
        });
        race.unlink = Box::new(|_: &Path| Err(Error::from(ErrorKind::PermissionDenied)));
        assert!(matches!(
            race.run(&p, Some(b"old"), b"new"),
            Err(KdbxError::Io {
                kind: ErrorKind::Other,
                ..
            })
        ));
        assert_eq!(fs::read(&p).unwrap(), b"old");
        let files = listing(d.path());
        for suffix in [".aside", ".displaced"] {
            assert!(files
                .iter()
                .any(|(name, bytes)| name.ends_with(suffix) && bytes == b"old"));
        }
    }
    #[test]
    fn a_directory_sync_failure_after_an_unlisted_retention_keeps_its_name() {
        // Publication fails (the copy stays under its working name) and the final folder sync
        // then fails too: the error, not the outcome, is all the caller sees, so it names the file.
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault.kdbx");
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let mut race = Race::new(exchange);
            race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
            let mut calls = 0;
            race.link = Box::new(move |a: &Path, b: &Path| {
                calls += 1;
                // Move-aside: 1 is the probe, 2 the install; every later call is a publication.
                let first_publication = if exchange { 1 } else { 3 };
                if calls >= first_publication {
                    return Err(Error::other("injected link failure"));
                }
                fs::hard_link(a, b)
            });
            race.sync_dir = Box::new(|_: &Path| Err(Error::other("injected sync failure")));
            let (kind, name) = retained(race.run(&p, Some(b"old"), b"new"));
            assert_eq!(kind, ErrorKind::Other, "exchange={exchange}");
            assert!(
                name.ends_with(if exchange { ".exchanged" } else { ".aside" }),
                "{name}"
            );
            assert_eq!(fs::read(d.path().join(&name)).unwrap(), b"theirs");
        }
    }
    #[test]
    fn a_directory_sync_failure_with_everything_listed_stays_a_plain_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault.kdbx");
        fs::write(&p, b"old").unwrap();
        let mut race = Race::new(false);
        race.sync_dir = Box::new(|_: &Path| Err(Error::other("injected sync failure")));
        io_kind(race.run(&p, Some(b"old"), b"new"), ErrorKind::Other);
    }
    #[cfg(unix)]
    #[test]
    fn a_dangling_secondary_link_after_failed_install_and_restore_still_reports() {
        // The secondary vault is a symlink to a file in another folder. The install and the
        // restore both fail, so the resolved file is gone and the link dangles. The writer's error
        // names the published copy, and the scan of the configured path is an error (the
        // location is unavailable), never an empty list.
        let d = tempfile::tempdir().unwrap();
        let real_dir = d.path().join("real");
        let link_dir = d.path().join("link");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&link_dir).unwrap();
        let real = real_dir.join("actual.kdbx");
        let link = link_dir.join("secondary.kdbx");
        fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut race = Race::new(false);
        let mut calls = 0;
        race.link = Box::new(move |a: &Path, b: &Path| {
            calls += 1;
            if calls == 2 || calls == 3 {
                return Err(Error::other("injected link failure"));
            }
            fs::hard_link(a, b)
        });
        let (_, name) = retained(race.run(&link, Some(b"old"), b"new"));
        assert_eq!(fs::read(real_dir.join(&name)).unwrap(), b"old");
        assert!(!real.exists(), "the link dangles");
        assert!(displaced_copies(&link).is_err());
    }
    #[test]
    fn publication_never_overwrites_an_occupied_displaced_name() {
        // Another process created the planned `.displaced` name between the final check and the
        // promotion. Its bytes survive, and ours go to a fresh unique name; both are listed.
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault.kdbx");
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let mut race = Race::new(exchange);
            race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
            let mut occupied = Vec::new();
            race.link = Box::new(move |a: &Path, b: &Path| {
                if b.to_string_lossy().ends_with(".displaced") && occupied.is_empty() {
                    fs::write(b, b"someone else's vault").unwrap();
                    occupied.push(b.to_path_buf());
                }
                fs::hard_link(a, b)
            });
            let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
            let ReplaceOutcome::Displaced { bytes, kept, .. } = outcome else {
                panic!("expected Displaced, got {outcome:?} (exchange={exchange})");
            };
            assert_eq!(bytes, b"theirs");
            assert_eq!(fs::read(&kept).unwrap(), b"theirs");
            let listed = displaced_copies(&p).unwrap();
            assert_eq!(listed.len(), 2, "exchange={exchange}");
            let mut contents: Vec<_> = listed.iter().map(|c| fs::read(&c.path).unwrap()).collect();
            contents.sort();
            assert_eq!(
                contents,
                [b"someone else's vault".to_vec(), b"theirs".to_vec()]
            );
            assert_eq!(fs::read(&p).unwrap(), b"new");
        }
    }
    #[cfg(unix)]
    #[test]
    fn a_vault_reached_through_a_symlink_is_scanned_where_its_copies_are_kept() {
        // The secondary vault is a symlink in one folder to a differently named file in another.
        // The writer retains a competing copy next to the RESOLVED file; the scan finds it.
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let real_dir = d.path().join("real");
            let link_dir = d.path().join("link");
            fs::create_dir_all(&real_dir).unwrap();
            fs::create_dir_all(&link_dir).unwrap();
            let real = real_dir.join("actual.kdbx");
            let link = link_dir.join("secondary.kdbx");
            fs::write(&real, b"old").unwrap();
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let racer = real.clone();
            let outcome = replace(
                &link,
                Some(b"old"),
                b"new",
                &mut || write_atomic(&racer, b"theirs").unwrap(),
                exchange,
            );
            assert!(
                matches!(outcome, ReplaceOutcome::Displaced { .. }),
                "{outcome:?}"
            );
            let listed = displaced_copies(&link).unwrap();
            assert_eq!(listed.len(), 1, "exchange={exchange}");
            assert_eq!(fs::read(&listed[0].path).unwrap(), b"theirs");
            assert_eq!(
                listed[0].path.parent(),
                Some(real_dir.canonicalize().unwrap().as_path())
            );
            assert_eq!(listed, displaced_copies(&real).unwrap());
            assert!(
                fs::read_dir(&link_dir).unwrap().count() == 1,
                "nothing next to the link"
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn failed_displaced_publication_after_an_exchange_keeps_the_competing_copy() {
        // N1: the swap succeeded, so the temp name held the competing vault and it was moved to
        // its `.exchanged` name; publishing it as `.displaced` fails. It must stay on disk
        // (under the `.exchanged` name, listed) and be returned.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let mut race = Race::new(true);
        race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
        race.link = Box::new(|_: &Path, _: &Path| Err(Error::other("injected link failure")));
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        // The surviving name is a working name, so the outcome also says which file to look for.
        let working = outcome.retained_working_path().unwrap().to_path_buf();
        let ReplaceOutcome::Displaced { bytes, kept, .. } = outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        assert_eq!(kept, working);
        assert_eq!(bytes, b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"new");
        assert_eq!(
            fs::read(&kept).unwrap(),
            b"theirs",
            "competing copy was deleted after the displaced rename failed"
        );
        assert!(kept.to_string_lossy().ends_with(".exchanged"), "{kept:?}");
        assert!(holds(d.path(), b"theirs"));
        let listed = displaced_copies(&p).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, DisplacedCopyKind::Exchanged);
        assert_eq!(listed[0].path, kept);
    }
    #[cfg(unix)]
    #[test]
    fn an_exchange_copy_that_cannot_be_moved_or_published_is_listed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let mut race = Race::new(true);
        race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
        race.noreplace = Box::new(|_: &Path, _: &Path| Err(Error::other("injected")));
        race.link = Box::new(|_: &Path, _: &Path| Err(Error::other("injected link failure")));
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        let ReplaceOutcome::Displaced { kept, .. } = &outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        assert_eq!(
            displaced_copies(&p).unwrap().first().map(|copy| &copy.path),
            Some(kept)
        );
        assert_eq!(outcome.retained_working_path(), Some(kept.as_path()));
        assert_eq!(fs::read(kept).unwrap(), b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"new");
    }
    #[cfg(unix)]
    #[test]
    fn the_exchange_working_name_lists_a_post_swap_crash_only() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault.kdbx");
        fs::write(&vault, b"original vault").unwrap();

        // Simulate a crash with the fully written candidate still under its working name.
        let mut names = SiblingNames::new(dir.path(), vault.file_name().unwrap()).unwrap();
        let (file, mut candidate) = create_temp_sibling(names.temp.clone()).unwrap();
        let active = prepare_exchange(&mut names, &mut candidate, &file)
            .unwrap()
            .unwrap();
        finish_temp_write(file, &vault, b"abandoned vault").unwrap();
        candidate.disarm();
        drop(active);
        assert!(displaced_copies(&vault).unwrap().is_empty());
        assert_eq!(fs::read(&names.temp).unwrap(), b"abandoned vault");

        let mut race = Race::new(true);
        race.before = Box::new(|| write_atomic(&vault, b"other writer's vault").unwrap());
        // The exchange has disarmed the temp cleanup by the time it calls the second rename.
        race.noreplace = Box::new(|_, _| panic!("simulated exit after exchange"));
        assert!(catch_unwind(AssertUnwindSafe(|| {
            let _ = race.run(&vault, Some(b"original vault"), b"our vault");
        }))
        .is_err());
        let listed = displaced_copies(&vault).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, DisplacedCopyKind::Exchanged);
        assert_eq!(fs::read(&listed[0].path).unwrap(), b"other writer's vault");
        assert_eq!(fs::read(&vault).unwrap(), b"our vault");
    }

    #[cfg(unix)]
    #[test]
    fn an_atomic_writer_between_link_and_identity_check_leaves_both_versions() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault.kdbx");
        fs::write(&vault, b"old vault").unwrap();
        let mut race = Race::new(false);
        race.noreplace = unsupported_link();
        race.link = Box::new(|from, to| {
            fs::hard_link(from, to)?;
            if to.extension().is_some_and(|ext| ext == "aside") {
                write_atomic(from, b"other writer's vault").unwrap();
            }
            Ok(())
        });
        let (_, kept) = retained(race.run(&vault, Some(b"old vault"), b"our vault"));
        assert_eq!(fs::read(&vault).unwrap(), b"other writer's vault");
        let copies = displaced_copies(&vault).unwrap();
        assert_eq!(copies.len(), 1);
        assert!(copies[0].path.ends_with(kept));
        assert_eq!(fs::read(&copies[0].path).unwrap(), b"old vault");
    }

    #[test]
    fn a_crash_left_empty_aside_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault.kdbx");
        fs::write(&vault, b"live vault").unwrap();
        fs::write(dir.path().join(".vault.kdbx.1.2.3.aside"), b"").unwrap();
        assert!(displaced_copies(&vault).unwrap().is_empty());
    }

    #[test]
    fn a_writer_at_the_fallback_move_boundary_keeps_its_distinct_bytes() {
        use crate::test_util::FailingFileIo;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault.kdbx");
        fs::write(&vault, b"old vault").unwrap();
        let planted = Arc::new(Mutex::new(None));
        let sink = planted.clone();
        let io = FailingFileIo::new(Arc::new(PlainFileIo));
        io.before_next_aside_move(move |at| {
            let incoming = at.with_extension("incoming");
            fs::write(&incoming, b"another writer's file").unwrap();
            fs::rename(incoming, at).unwrap();
            *sink.lock().unwrap() = Some(at.to_path_buf());
        });
        assert_eq!(
            io.replace_if(&vault, Some(&hash(b"old vault")), b"new vault", LIMIT)
                .unwrap(),
            ReplaceOutcome::Replaced
        );
        let planted = planted.lock().unwrap();
        assert_eq!(
            fs::read(planted.as_ref().unwrap()).unwrap(),
            b"another writer's file"
        );
        assert_eq!(fs::read(vault).unwrap(), b"new vault");
    }

    #[cfg(unix)]
    #[test]
    fn an_exchange_copy_is_hidden_while_in_flight_and_a_taken_exchanged_name_is_never_replaced() {
        // Another program's file already sits under the planned `.exchanged` name: the swapped
        // out file takes a fresh name and the planted file keeps its distinct bytes. While the
        // swapped-out file is being judged, it is not listed.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let probe = p.clone();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        let planted = std::rc::Rc::new(std::cell::RefCell::new(None));
        let plant = planted.clone();
        let mut race = Race::new(true);
        race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
        race.noreplace = Box::new(move |a: &Path, b: &Path| {
            if plant.borrow().is_none() {
                fs::write(b, b"planted by another program").unwrap();
                *plant.borrow_mut() = Some(b.to_path_buf());
            }
            rename_noreplace(a, b)?;
            sink.borrow_mut().push(displaced_copies(&probe).unwrap());
            Ok(())
        });
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        drop(race);
        let ReplaceOutcome::Displaced { kept, .. } = &outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        let planted = planted.borrow().clone().unwrap();
        assert!(planted.to_string_lossy().ends_with(".exchanged"));
        assert_eq!(
            fs::read(&planted).unwrap(),
            b"planted by another program",
            "the planted file was overwritten"
        );
        assert_eq!(fs::read(kept).unwrap(), b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"new");
        // In flight: only the planted file (another program's) is listed, not ours.
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].len(), 1);
        assert_eq!(seen[0][0].path, planted);
    }
    #[test]
    fn a_name_at_the_length_limit_still_keeps_the_displaced_copy() {
        // The re-review reproduction: a name whose `.tmp` sibling fits the 255-byte limit but
        // whose `.displaced` sibling did not. Sibling names are now bounded.
        for exchange in modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("v".repeat(250));
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let outcome = replace(
                &p,
                Some(b"old"),
                b"new",
                &mut || write_atomic(&racer, b"theirs").unwrap(),
                exchange,
            );
            let ReplaceOutcome::Displaced { kept, .. } = outcome else {
                panic!("expected Displaced, got {outcome:?} (exchange={exchange})");
            };
            assert_eq!(fs::read(&kept).unwrap(), b"theirs");
            assert!(kept.to_string_lossy().ends_with(".displaced"));
            assert_eq!(fs::read(&p).unwrap(), b"new");
            assert_eq!(
                replace(&p, Some(b"new"), b"newer", &mut || {}, exchange),
                ReplaceOutcome::Replaced
            );
        }
    }
    #[test]
    fn writer_creating_the_file_while_it_is_moved_aside_wins_and_nothing_is_lost() {
        // Move-aside path: a writer creates the file in the instant the path is empty. Ours is
        // not linked over it; the moved-aside expected content (already merged by the caller)
        // is removed.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let mut race = Race::new(false);
        race.after_aside = Box::new(move || write_atomic(&racer, b"third").unwrap());
        assert_eq!(
            race.run(&p, Some(b"old"), b"new").unwrap(),
            ReplaceOutcome::Conflict
        );
        assert_eq!(fs::read(&p).unwrap(), b"third");
        only_target(&d);
    }
    #[test]
    fn two_racing_writers_around_the_move_aside_are_both_kept() {
        // One writer commits after the final check (moved aside, foreign), another creates the
        // file while the path is empty (wins the path). Both survive; ours is not installed.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let (r1, r2) = (p.clone(), p.clone());
        let mut race = Race::new(false);
        race.before = Box::new(move || write_atomic(&r1, b"theirs").unwrap());
        race.after_aside = Box::new(move || write_atomic(&r2, b"third").unwrap());
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        let ReplaceOutcome::Displaced { bytes, kept, .. } = outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        assert_eq!(bytes, b"theirs");
        assert_eq!(fs::read(&kept).unwrap(), b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"third");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
    }
    #[test]
    fn transient_link_errors_remain_retryable_and_recover() {
        for existing in [false, true] {
            for kind in [
                ErrorKind::TimedOut,
                ErrorKind::StorageFull,
                ErrorKind::PermissionDenied,
            ] {
                let d = tempfile::tempdir().unwrap();
                let p = d.path().join("vault");
                if existing {
                    fs::write(&p, b"old").unwrap();
                }
                let mut race = Race::new(false);
                race.link = Box::new(move |_, _| Err(io::Error::from(kind)));
                io_kind(
                    race.run(&p, existing.then_some(b"old".as_slice()), b"new"),
                    kind,
                );
                if existing {
                    assert_eq!(fs::read(&p).unwrap(), b"old");
                } else {
                    assert!(!p.exists());
                }
                race.link = Box::new(|a, b| fs::hard_link(a, b));
                assert_eq!(
                    race.run(&p, existing.then_some(b"old".as_slice()), b"new")
                        .unwrap(),
                    ReplaceOutcome::Replaced
                );
                assert_eq!(fs::read(&p).unwrap(), b"new");
            }
        }
    }

    #[test]
    fn probe_retries_a_single_sharing_failure() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let mut race = Race::new(false);
        let mut calls = 0;
        race.link = Box::new(move |a, b| {
            calls += 1;
            if calls == 1 {
                Err(io::Error::from(ErrorKind::PermissionDenied))
            } else {
                fs::hard_link(a, b)
            }
        });
        assert_eq!(
            race.run(&p, Some(b"old"), b"new").unwrap(),
            ReplaceOutcome::Replaced
        );
        assert_eq!(fs::read(&p).unwrap(), b"new");
    }

    #[test]
    fn a_volume_without_exchange_or_hard_links_is_refused_untouched() {
        // C2: no blind rename over the target. Without an atomic exchange or hard links the
        // replacement is refused before anything is renamed.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let mut race = Race::new(false);
        race.link = unsupported_link();
        io_kind(race.run(&p, Some(b"old"), b"new"), ErrorKind::Unsupported);
        assert_eq!(fs::read(&p).unwrap(), b"old");
        only_target(&d);
        // An initial upload is refused the same way (no check-then-rename).
        let q = d.path().join("fresh");
        let mut race = Race::new(false);
        race.link = unsupported_link();
        io_kind(race.run(&q, None, b"ours"), ErrorKind::Unsupported);
        assert!(!q.exists());
        only_target(&d);
    }
    #[test]
    fn a_writer_during_a_move_aside_retry_is_moved_aside_not_overwritten() {
        // The move aside hits a sharing violation; a writer replaces the file while we wait.
        // The retry moves the writer's file aside (whatever is there), and it is kept.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let mut race = Race::new(false);
        let mut failures = 1;
        race.noreplace = Box::new(move |a: &Path, b: &Path| {
            if failures > 0 {
                failures -= 1;
                return Err(Error::from(ErrorKind::PermissionDenied));
            }
            rename_noreplace(a, b)
        });
        race.sleep = Box::new(move |_| write_atomic(&racer, b"theirs").unwrap());
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        let ReplaceOutcome::Displaced { bytes, kept, .. } = outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        assert_eq!(bytes, b"theirs");
        assert_eq!(fs::read(&kept).unwrap(), b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"new");
    }
    #[test]
    fn a_failed_move_aside_touches_nothing() {
        // Fail both the atomic move and the hard-link fallback before they take effect.
        for &fallback in move_aside_modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault");
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let mut race = Race::new(false);
            race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
            if fallback {
                race.noreplace = unsupported_link();
                race.link = Box::new(|from, to| {
                    if to.extension().is_some_and(|ext| ext == "aside") {
                        return Err(Error::other("injected"));
                    }
                    fs::hard_link(from, to)
                });
            } else {
                race.noreplace = Box::new(|_: &Path, _: &Path| Err(Error::other("injected")));
            }
            io_kind(race.run(&p, Some(b"old"), b"new"), ErrorKind::Other);
            assert_eq!(fs::read(&p).unwrap(), b"theirs");
            only_target(&d);
        }
    }
    /// Plants another program's file under every working name the move aside tries first
    /// (`plant_all`: under every name), then delegates to the no-replace rename, or reports it
    /// unsupported so the move falls back to a hard link.
    fn planting(
        fallback: bool,
        plant_all: bool,
        planted: std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>>,
    ) -> Call<'static> {
        Box::new(move |a: &Path, b: &Path| {
            if plant_all || planted.borrow().is_empty() {
                fs::write(b, b"another program's file").unwrap();
                planted.borrow_mut().push(b.to_path_buf());
            }
            if fallback {
                Err(Error::from(ErrorKind::Unsupported))
            } else {
                rename_noreplace(a, b)
            }
        })
    }
    fn move_aside_modes() -> &'static [bool] {
        if cfg!(unix) {
            &[false, true]
        } else {
            &[false]
        }
    }

    #[test]
    fn a_file_already_under_the_aside_name_is_never_overwritten() {
        // V1: another program put its own file under the exact working name first. The move
        // aside never replaces it and takes a fresh name instead.
        for &fallback in move_aside_modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault.kdbx");
            fs::write(&p, b"old").unwrap();
            let planted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let mut race = Race::new(false);
            race.noreplace = planting(fallback, false, planted.clone());
            let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
            drop(race);
            assert_eq!(outcome, ReplaceOutcome::Replaced, "fallback={fallback}");
            assert_eq!(fs::read(&p).unwrap(), b"new");
            let planted = planted.borrow();
            assert_eq!(planted.len(), 1);
            assert!(planted[0].to_string_lossy().ends_with(".aside"));
            assert_eq!(fs::read(&planted[0]).unwrap(), b"another program's file");
            // Only the vault and the planted file remain: our working name was released.
            let names: Vec<_> = listing(d.path()).into_iter().map(|(n, _)| n).collect();
            assert_eq!(names.len(), 2, "{names:?}");
        }
    }
    #[test]
    fn the_move_aside_fails_closed_when_every_working_name_is_taken() {
        for &fallback in move_aside_modes() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault.kdbx");
            fs::write(&p, b"old").unwrap();
            let planted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let mut race = Race::new(false);
            race.noreplace = planting(fallback, true, planted.clone());
            io_kind(race.run(&p, Some(b"old"), b"new"), ErrorKind::AlreadyExists);
            drop(race);
            assert_eq!(fs::read(&p).unwrap(), b"old", "the vault is untouched");
            let planted = planted.borrow();
            assert_eq!(planted.len(), PUBLISH_ATTEMPTS);
            for path in planted.iter() {
                assert_eq!(fs::read(path).unwrap(), b"another program's file");
            }
            assert!(!holds(d.path(), b"new"), "our temp file is removed");
            assert_eq!(listing(d.path()).len(), PUBLISH_ATTEMPTS + 1);
        }
    }
    #[cfg(unix)]
    #[test]
    fn the_fallback_move_aside_keeps_a_racing_writer_and_leaves_nothing_behind() {
        for racing in [false, true] {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("vault.kdbx");
            fs::write(&p, b"old").unwrap();
            let racer = p.clone();
            let mut race = Race::new(false);
            race.noreplace = unsupported_link();
            if racing {
                race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
            }
            let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
            assert_eq!(fs::read(&p).unwrap(), b"new");
            if racing {
                let ReplaceOutcome::Displaced { kept, .. } = &outcome else {
                    panic!("expected Displaced, got {outcome:?}");
                };
                assert_eq!(fs::read(kept).unwrap(), b"theirs");
                assert_eq!(listing(d.path()).len(), 2);
            } else {
                assert_eq!(outcome, ReplaceOutcome::Replaced);
                only_target(&d);
            }
        }
    }
    #[test]
    fn a_failed_link_after_the_move_aside_restores_and_keeps_the_copy() {
        // The probe link works, the install link fails: the moved-aside file is linked back at
        // the path and ALSO published as a listed `.displaced` copy (a duplicate of the live
        // vault, harmless); the settled copy has a `.displaced` name, and no vault data
        // is deleted on the failure path.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let mut race = Race::new(false);
        race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
        let mut calls = 0;
        race.link = Box::new(move |a: &Path, b: &Path| {
            calls += 1;
            if calls == 2 {
                return Err(Error::other("injected link failure"));
            }
            fs::hard_link(a, b)
        });
        io_kind(race.run(&p, Some(b"old"), b"new"), ErrorKind::Other);
        assert_eq!(fs::read(&p).unwrap(), b"theirs", "restored at the path");
        let names: Vec<_> = listing(d.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|n| n.ends_with(".displaced")), "{names:?}");
        assert!(!names.iter().any(|n| n.ends_with(".aside")), "{names:?}");
        assert!(!holds(d.path(), b"new"), "our temp file is removed");
    }
    #[cfg(unix)]
    #[test]
    fn retargeted_symlink_does_not_redirect_the_replacement() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.kdbx");
        let b = d.path().join("b.kdbx");
        let link = d.path().join("vault.kdbx");
        fs::write(&a, b"old").unwrap();
        fs::write(&b, b"other vault").unwrap();
        std::os::unix::fs::symlink(&a, &link).unwrap();
        let (l, target) = (link.clone(), b.clone());
        let outcome = replace(
            &link,
            Some(b"old"),
            b"new",
            &mut || {
                fs::remove_file(&l).unwrap();
                std::os::unix::fs::symlink(&target, &l).unwrap();
            },
            true,
        );
        assert_eq!(outcome, ReplaceOutcome::Replaced);
        assert_eq!(fs::read(&a).unwrap(), b"new");
        assert_eq!(fs::read(&b).unwrap(), b"other vault");
    }
    #[test]
    fn replace_if_refuses_a_non_regular_target() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::create_dir(&p).unwrap();
        io_kind(
            Race::new(true).run(&p, Some(b"x"), b"new"),
            ErrorKind::InvalidInput,
        );
        assert!(p.is_dir());
    }
    #[test]
    fn displaced_outcome_debug_shows_no_bytes() {
        let outcome = ReplaceOutcome::Displaced {
            bytes: b"secret-bytes".to_vec(),
            kept: PathBuf::from("k"),
            leftover: None,
        };
        assert!(!format!("{outcome:?}").contains("secret"));
    }

    /// Serves one vault's bytes for any path, like a seam that reads from an offline cache.
    struct CachedIo(Vec<u8>);
    impl FileIo for CachedIo {
        fn read(&self, _path: &Path) -> Result<Vec<u8>> {
            Ok(self.0.clone())
        }
        fn metadata(&self, _path: &Path) -> Result<FileMeta> {
            Ok(FileMeta {
                len: self.0.len() as u64,
                modified: None,
            })
        }
        fn write_atomic(&self, _path: &Path, _bytes: &[u8]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn open_with_resolves_a_missing_file_for_the_seam_and_plain_io_still_reports_not_found() {
        let dir = TempDir::new().unwrap();
        let real_dir = fs::canonicalize(dir.path()).unwrap();
        let bytes = Database::new("cached").save(&password()).unwrap();
        let io: Arc<dyn FileIo> = Arc::new(CachedIo(bytes));
        // Placeholder-only: the folder is there, the file is not.
        let missing = dir.path().join("v.kdbx");
        let (file, _) = VaultFile::open_with(io.clone(), &missing, &password()).unwrap();
        assert_eq!(file.path(), real_dir.join("v.kdbx"));
        // The folder is gone too (offline iCloud Drive): kept as written.
        let offline = real_dir.join("gone").join("v.kdbx");
        let (file, _) = VaultFile::open_with(io, &offline, &password()).unwrap();
        assert_eq!(file.path(), offline);
        // Plain I/O: still NotFound.
        let err = VaultFile::open_with(Arc::new(PlainFileIo), &missing, &password()).unwrap_err();
        assert!(matches!(
            err,
            KdbxError::Io {
                kind: ErrorKind::NotFound,
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn open_with_keeps_refusing_a_dangling_symlink() {
        let dir = TempDir::new().unwrap();
        let link = dir.path().join("v.kdbx");
        std::os::unix::fs::symlink(dir.path().join("nowhere"), &link).unwrap();
        let io: Arc<dyn FileIo> = Arc::new(CachedIo(Vec::new()));
        assert!(VaultFile::open_with(io, &link, &password()).is_err());
    }
}
