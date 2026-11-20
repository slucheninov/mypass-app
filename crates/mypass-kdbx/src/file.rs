//! Atomic vault replacement and a conservative concurrent-change guard.
use crate::{Database, KdbxError, KdfLimits, Result};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

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
        /// Where the displaced file was kept (a hidden name next to the target).
        kept: PathBuf,
    },
}
impl fmt::Debug for ReplaceOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replaced => f.write_str("Replaced"),
            Self::Conflict => f.write_str("Conflict"),
            Self::Displaced { bytes, kept } => f
                .debug_struct("Displaced")
                .field("len", &bytes.len())
                .field("kept", kept)
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
    ///   swapped in and whatever was at the path at that instant comes back under our temp
    ///   name, which from then on is never deleted unless it holds the expected content.
    ///   Anything else is renamed to `.<name>.<pid>.<nanos>.<n>.displaced` and returned as
    ///   [`ReplaceOutcome::Displaced`]; if even that rename fails it stays under the temp name
    ///   `.<name>.<pid>.<nanos>.<n>.tmp` (still returned as `Displaced`, `kept` names it).
    /// - **Move aside, then a no-replace link: volumes without the exchange that have hard
    ///   links (Windows NTFS and ReFS; SMB, NFS and other mounts that refuse the swap but
    ///   allow `link`).** After the final check, a probe link proves that hard links work
    ///   here; the current file is then renamed to the fresh `.displaced` name (it captures
    ///   whatever is at the path at that instant, overwriting nothing) and the new file is
    ///   linked in at the path, which fails rather than replaces when another writer created
    ///   the file in between. The moved-aside file is removed only when it proves to be the
    ///   expected content; otherwise it is kept and returned as `Displaced`. While the path
    ///   is briefly absent, a reader sees no file (a racing creator wins, and our bytes are
    ///   then not installed: `Displaced` when a foreign version was moved aside, else
    ///   `Conflict`).
    /// - **Neither (FAT, exFAT and network volumes without hard links):** refused with
    ///   `ErrorKind::Unsupported` before anything is renamed; the target is untouched and
    ///   only our temp file is removed. There is no safe way to replace a file there.
    ///
    /// Remaining window (documented residual): a writer that rewrites the old file **in
    /// place** (truncate and write on the same inode) after we read the swapped-out or
    /// moved-aside file back and judged it to be the expected content. Atomic writers (temp
    /// file plus rename: this app, KeePassXC, sync clients) are not affected.
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
        Self::open_with_limits(io, path, password, KdfLimits::Local)
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
        Self::open_with_limits_checked(io, path, password, KdfLimits::Local, check)
    }

    /// Open with a withdrawal fence and the selected KDF limits.
    pub fn open_with_limits_checked<E: From<KdbxError>>(
        io: Arc<dyn FileIo>,
        path: &Path,
        password: &SecretString,
        limits: KdfLimits,
        check: &dyn Fn() -> std::result::Result<(), E>,
    ) -> std::result::Result<(Self, Database), E> {
        Self::open_checked(io, path, check, |bytes| {
            Database::open_with_limits(bytes, password, limits)
        })
    }

    /// Reopen a local vault to reconcile kept edits. The previous authenticated content keeps
    /// the local KDF limits. A replacement gets the remote limits, checked on the bytes opened.
    pub fn open_for_recovery(
        io: Arc<dyn FileIo>,
        path: &Path,
        password: &SecretString,
        previous_hash: [u8; 32],
    ) -> Result<(Self, Database)> {
        Self::open_checked(io, path, &|| Ok(()), |bytes| {
            let limits = if hash(bytes) == previous_hash {
                KdfLimits::Local
            } else {
                KdfLimits::Remote
            };
            Database::open_with_limits(bytes, password, limits)
        })
    }

    fn open_checked<E: From<KdbxError>>(
        io: Arc<dyn FileIo>,
        path: &Path,
        check: &dyn Fn() -> std::result::Result<(), E>,
        open: impl FnOnce(&[u8]) -> Result<Database>,
    ) -> std::result::Result<(Self, Database), E> {
        check()?;
        // Path identity is resolved once; every later byte/metadata operation uses the seam.
        let path = resolve_vault_path(path).map_err(io_error)?;
        check()?;
        let bytes = io.read(&path)?;
        check()?;
        let db = open(&bytes)?;
        check()?;
        let stamp = stamp(io.metadata(&path)?, &bytes);
        Ok((Self { path, stamp, io }, db))
    }

    /// Hash of the last authenticated or written vault bytes.
    pub fn content_hash(&self) -> [u8; 32] {
        self.stamp.sha256
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
        self.reload_with_limits(password, KdfLimits::Local)
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
/// The longest prefix of the target's file name used in a sibling name, in bytes. With the
/// `.<pid>.<nanos>.<n>.displaced` tail (at most 64 bytes) a sibling name stays far below the
/// 255-byte (or 255-UTF-16-unit) name limit of every common file system, whatever the length of
/// the target's name, so naming a displaced file never fails because the name is too long.
const SIBLING_STEM_MAX: usize = 96;

/// The hidden names one replacement uses next to its target, all sharing one unique token
/// `.<stem>.<pid>.<nanos>.<n>`: a process id, the clock and a per-process counter, so no two
/// replacements (in this process or another) ever pick the same name.
struct SiblingNames {
    /// The new bytes, written and synced before anything is renamed.
    temp: PathBuf,
    /// Where a file that may hold another writer's vault is kept.
    displaced: PathBuf,
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
        let mut cut = stem.len().min(SIBLING_STEM_MAX);
        while !stem.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        let token = format!(
            ".{}.{}.{nanos}.{n}",
            stem.get(..cut).unwrap_or_default(),
            std::process::id()
        );
        Ok(Self {
            temp: parent.join(format!("{token}.tmp")),
            displaced: parent.join(format!("{token}.displaced")),
            probe: parent.join(format!("{token}.link.tmp")),
        })
    }
}
/// Writes `bytes` to the fresh file `temp` next to `target` and syncs it. The returned guard
/// deletes the temp file on drop until it is disarmed (after a rename handed the name over).
fn write_temp_sibling(temp: PathBuf, target: &Path, bytes: &[u8]) -> Result<TempCleanup> {
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
    // Drop the open handle before cleanup on all pre-rename failure paths.
    let mut file = file;
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
    file.sync_all().map_err(io_error)?;
    drop(file);
    Ok(cleanup)
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
    /// A plain rename to a fresh name of our own (move aside, keep a displaced file).
    rename: &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    /// A hard link, failing on an existing name (the no-replace install).
    link: &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    /// Sleeps between the retries of a rename or link that hit a sharing violation.
    sleep: &'a mut dyn FnMut(Duration),
}
/// [`replace_if_with`] over the real system calls.
fn replace_if_production(
    path: &Path,
    expected: Option<&[u8; 32]>,
    bytes: &[u8],
    max_len: u64,
) -> Result<ReplaceOutcome> {
    let (mut before_install, mut after_aside) = (|| {}, || {});
    let mut rename = |from: &Path, to: &Path| fs::rename(from, to);
    let mut link = |from: &Path, to: &Path| fs::hard_link(from, to);
    let mut sleep = std::thread::sleep;
    let mut seams = Seams {
        before_install: &mut before_install,
        after_aside: &mut after_aside,
        exchange: true,
        rename: &mut rename,
        link: &mut link,
        sleep: &mut sleep,
    };
    replace_if_with(path, expected, bytes, max_len, &mut seams)
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
    let (parent, name) = split_target(&target)?;
    let names = SiblingNames::new(parent, name)?;
    // The exchange flag is only read where `renameat_with` exists.
    #[cfg(not(unix))]
    let _ = seams.exchange;
    // The slow part (writing and syncing the new bytes) happens before the final check.
    let cleanup = write_temp_sibling(names.temp.clone(), &target, bytes)?;
    let outcome = match expected {
        None => install_new(&target, &names, cleanup, seams)?,
        Some(expected) => replace_expected(&target, &names, cleanup, expected, max_len, seams)?,
    };
    if outcome != ReplaceOutcome::Conflict {
        finish_directory_sync(sync_directory(parent))?;
    }
    Ok(outcome)
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
    if seams.exchange {
        match rename_with(&names.temp, target, rustix::fs::RenameFlags::EXCHANGE)? {
            Swap::Done => {
                // The temp name now holds whatever was at the target at the instant of the swap,
                // possibly another writer's vault: it is no longer ours to delete.
                cleanup.disarm();
                return Ok(judge_swapped_out(
                    &names.temp,
                    names,
                    expected,
                    max_len,
                    seams,
                    true,
                ));
            }
            Swap::Refused => return Ok(ReplaceOutcome::Conflict),
            Swap::Unsupported => {}
        }
    }
    // Move-aside path. Nothing has been renamed yet; without hard links refuse here.
    probe_links(names, seams)?;
    // Moving the current file to our fresh `.displaced` name overwrites nothing and captures
    // whatever is at the path at this instant, so a writer racing the final check (or a
    // sharing-violation retry) is moved aside, not destroyed.
    let moved = retry_rename(
        || (seams.rename)(target, &names.displaced),
        &mut *seams.sleep,
    );
    match moved {
        Ok(()) => {}
        // The file vanished (a writer is mid-replace, or deleted it): nothing was moved.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ReplaceOutcome::Conflict),
        Err(e) => return Err(io_error(e)),
    }
    (seams.after_aside)();
    let linked = retry_rename(|| (seams.link)(&names.temp, target), &mut *seams.sleep);
    match linked {
        // Our bytes are at the path; the temp name is a second link `cleanup` removes.
        Ok(()) => Ok(judge_swapped_out(
            &names.displaced,
            names,
            expected,
            max_len,
            seams,
            true,
        )),
        // Another writer created the file while the path was empty: theirs stays, ours is not
        // installed. What we moved aside is judged like a swapped-out file.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(judge_swapped_out(
            &names.displaced,
            names,
            expected,
            max_len,
            seams,
            false,
        )),
        Err(e) => {
            // Ours is not installed and the path may be empty. Put the moved-aside file back
            // without replacing anything (best effort) and keep its `.displaced` name as well:
            // nothing is deleted on a failure path.
            let _ = (seams.link)(&names.displaced, target);
            drop(cleanup);
            Err(io_error(e))
        }
    }
}

/// Judges the file that was at the target the instant ours went in (`at`: the temp name after
/// an exchange, the `.displaced` name after a move aside). Only a file read back and proved to
/// be the expected content is removed; anything else (another writer's version, or a file that
/// cannot be read) is kept and returned as `Displaced`. Never fails: a file that cannot be
/// renamed to the `.displaced` name stays where it is. `installed` tells whether our bytes are
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
        let _ = fs::remove_file(at);
        return if installed {
            ReplaceOutcome::Replaced
        } else {
            ReplaceOutcome::Conflict
        };
    }
    let kept = if at == names.displaced.as_path() {
        names.displaced.clone()
    } else {
        match (seams.rename)(at, &names.displaced) {
            Ok(()) => names.displaced.clone(),
            // Keep it under its current, recognisable name rather than lose it.
            Err(_) => at.to_path_buf(),
        }
    };
    ReplaceOutcome::Displaced {
        bytes: content.unwrap_or_default(),
        kept,
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
    #[test]
    fn local_recovery_keeps_local_limits_only_for_the_same_ciphertext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.kdbx");
        let mut db = Database::new("synthetic");
        db.settings.kdf = crate::KdfParams::Argon2 {
            variant: crate::Argon2Variant::Id,
            memory_bytes: 128 * 1024,
            parallelism: 9,
            iterations: 1,
            version: 0x13,
            salt: vec![0; 32],
        };
        let pw = password();
        fs::write(&path, db.save(&pw).unwrap()).unwrap();
        let (file, expected) = VaultFile::open(&path, &pw).unwrap();
        let reopen =
            || VaultFile::open_for_recovery(file.io().clone(), &path, &pw, file.content_hash());
        assert_eq!(reopen().unwrap().1, expected);

        db.meta.database_name = "external edit".into();
        fs::write(&path, db.save(&pw).unwrap()).unwrap();
        assert!(matches!(
            reopen(),
            Err(KdbxError::RemoteKdfLimitExceeded("Argon2 parallelism"))
        ));
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
    struct Race<'a> {
        exchange: bool,
        before: Hook<'a>,
        after_aside: Hook<'a>,
        rename: Call<'a>,
        link: Call<'a>,
        sleep: Box<dyn FnMut(Duration) + 'a>,
    }
    impl<'a> Race<'a> {
        fn new(exchange: bool) -> Self {
            Self {
                exchange,
                before: Box::new(|| {}),
                after_aside: Box::new(|| {}),
                rename: Box::new(|a: &Path, b: &Path| fs::rename(a, b)),
                link: Box::new(|a: &Path, b: &Path| fs::hard_link(a, b)),
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
                rename: &mut *self.rename,
                link: &mut *self.link,
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
            let ReplaceOutcome::Displaced { bytes, kept } = outcome else {
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
    #[cfg(unix)]
    #[test]
    fn failed_displaced_rename_after_an_exchange_keeps_the_competing_copy() {
        // N1: the swap succeeded, so the temp name holds the competing vault; naming it
        // `.displaced` fails. It must stay on disk (under the temp name) and be returned.
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let mut race = Race::new(true);
        race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
        race.rename = Box::new(|_: &Path, _: &Path| Err(Error::other("injected rename failure")));
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        let ReplaceOutcome::Displaced { bytes, kept } = outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        assert_eq!(bytes, b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"new");
        assert_eq!(
            fs::read(&kept).unwrap(),
            b"theirs",
            "competing copy was deleted after the displaced rename failed"
        );
        assert!(kept.to_string_lossy().ends_with(".tmp"), "{kept:?}");
        assert!(holds(d.path(), b"theirs"));
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
        let ReplaceOutcome::Displaced { bytes, kept } = outcome else {
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
        race.rename = Box::new(move |a: &Path, b: &Path| {
            if failures > 0 {
                failures -= 1;
                return Err(Error::from(ErrorKind::PermissionDenied));
            }
            fs::rename(a, b)
        });
        race.sleep = Box::new(move |_| write_atomic(&racer, b"theirs").unwrap());
        let outcome = race.run(&p, Some(b"old"), b"new").unwrap();
        let ReplaceOutcome::Displaced { bytes, kept } = outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        assert_eq!(bytes, b"theirs");
        assert_eq!(fs::read(&kept).unwrap(), b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"new");
    }
    #[test]
    fn a_failed_move_aside_touches_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let mut race = Race::new(false);
        race.before = Box::new(move || write_atomic(&racer, b"theirs").unwrap());
        race.rename = Box::new(|_: &Path, _: &Path| Err(Error::other("injected")));
        io_kind(race.run(&p, Some(b"old"), b"new"), ErrorKind::Other);
        assert_eq!(fs::read(&p).unwrap(), b"theirs");
        only_target(&d);
    }
    #[test]
    fn a_failed_link_after_the_move_aside_restores_and_keeps_the_copy() {
        // The probe link works, the install link fails: the moved-aside file is linked back at
        // the path and its `.displaced` name kept; nothing is deleted on the failure path.
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
