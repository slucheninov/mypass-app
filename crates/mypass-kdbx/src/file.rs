//! Atomic vault replacement and a conservative concurrent-change guard.
use crate::{Database, KdbxError, Result};
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
    /// disk instead of being destroyed, where the platform can swap atomically (see
    /// [`PlainFileIo`] for the remaining window elsewhere).
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
    /// The new bytes are in place, but another writer replaced the file in the instant between
    /// the final check and the replacement. Its content was not destroyed: it is returned in
    /// `bytes` (empty when it could not be read back) and kept on disk at `kept`.
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
    /// file system, so the guarantee depends on the platform:
    ///
    /// - **Unix, where `renameat2(RENAME_EXCHANGE)` (Linux) or `renamex_np(RENAME_SWAP)` (macOS:
    ///   APFS, HFS+) is supported:** the new file is swapped in atomically and whatever was at the
    ///   path at that instant comes back under the temp name. If that is not the expected
    ///   content, a writer slipped in after the final check: its file is kept as
    ///   `.<name>.displaced-<pid>.<nanos>` and returned as [`ReplaceOutcome::Displaced`].
    ///   Nothing is destroyed.
    /// - **Elsewhere (Windows, SMB/NFS/FAT volumes that refuse the swap):** the identity of the
    ///   file checked (device, inode, length, modification and change time; length and
    ///   modification time on Windows) is compared once more immediately before a plain rename.
    ///   A writer that replaces the file in the window between that `stat` and the `rename`
    ///   (two system calls, plus the Windows sharing-violation retries of the rename) is
    ///   overwritten. This window cannot be closed without an atomic swap or a lock that every
    ///   writer honours.
    ///
    /// A missing file (`expected` is `None`) is created without ever replacing one that appeared:
    /// `RENAME_NOREPLACE`/`RENAME_EXCL`, else a hard link (fails on an existing name), else a
    /// check-then-rename with the same `stat`-to-`rename` window as above.
    fn replace_if(
        &self,
        path: &Path,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
        max_len: u64,
    ) -> Result<ReplaceOutcome> {
        replace_if_with(path, expected, bytes, max_len, &mut || {}, true)
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
        // Path identity is resolved once; every later byte/metadata operation uses the seam.
        let path = fs::canonicalize(path).map_err(io_error)?;
        let bytes = io.read(&path)?;
        let db = Database::open(&bytes, password)?;
        let stamp = stamp(io.metadata(&path)?, &bytes);
        Ok((Self { path, stamp, io }, db))
    }
    /// Return the resolved vault path.
    pub fn path(&self) -> &Path {
        &self.path
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
        self.save_bytes(&bytes)?;
        Ok(SaveOutcome::Saved)
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
        let bytes = self.io.read(&self.path)?;
        let db = Database::open(&bytes, password)?;
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
impl Drop for TempCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
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
    let mut cleanup = write_temp_sibling(&target, parent, name, bytes)?;
    let temp = cleanup.path.clone();
    retry_rename(|| rename(&temp, &target), &mut sleep).map_err(io_error)?;
    // Rename transferred ownership; this name may now belong to another writer.
    cleanup.armed = false;
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
/// A hidden sibling name `.<name>.<pid>.<nanos><suffix>` in `parent`.
fn sibling_name(parent: &Path, name: &std::ffi::OsStr, suffix: &str) -> Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| io_error(io::Error::other("system clock before epoch")))?
        .as_nanos();
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{}.{nanos}{suffix}", std::process::id()));
    Ok(parent.join(temp_name))
}
/// Writes `bytes` to a fresh temp file next to `target` and syncs it. The returned guard deletes
/// the temp file on drop until it is disarmed (after a rename handed the name over).
fn write_temp_sibling(
    target: &Path,
    parent: &Path,
    name: &std::ffi::OsStr,
    bytes: &[u8],
) -> Result<TempCleanup> {
    let temp = sibling_name(parent, name, ".tmp")?;
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

/// What identifies one version of a file for the fallback check right before a plain rename.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    node: (u64, u64, i64, i64),
}
fn identity_of(meta: &fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        len: meta.len(),
        modified: meta.modified().ok(),
        #[cfg(unix)]
        node: (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
    }
}
/// The current content of `path` (bounded, regular, read on the opened handle) and the identity
/// of that handle; `None` when the file is missing.
fn read_current(path: &Path, max_len: u64) -> Result<Option<(Vec<u8>, FileIdentity)>> {
    let file = match open_for_read(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(e)),
    };
    let meta = file.metadata().map_err(io_error)?;
    let identity = identity_of(&meta);
    let bytes = read_opened(
        OpenedFile::new(Box::new(file), meta.is_file(), meta.len()),
        max_len,
    )?;
    Ok(Some((bytes, identity)))
}
/// The identity of `path` right now; `None` when it is missing.
fn identity_now(path: &Path) -> Result<Option<FileIdentity>> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(identity_of(&meta))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(e)),
    }
}

/// How the atomic rename step ended on this platform and volume.
#[cfg(unix)]
enum Swap {
    Done,
    /// The volume or kernel does not support the flag: use the portable fallback.
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

/// The guarded replacement behind [`PlainFileIo::replace_if`]. `before_install` runs right
/// after the final check (a test seam for a writer racing the last step); `exchange: false`
/// forces the portable fallback.
fn replace_if_with(
    path: &Path,
    expected: Option<&[u8; 32]>,
    bytes: &[u8],
    max_len: u64,
    before_install: &mut dyn FnMut(),
    exchange: bool,
) -> Result<ReplaceOutcome> {
    // Resolved once: a symlink retargeted later cannot redirect the replacement to another file.
    let target = resolve_using(|| resolve_target(path), std::thread::sleep).map_err(io_error)?;
    let (parent, name) = split_target(&target)?;
    // The slow part (writing and syncing the new bytes) happens before the final check.
    let cleanup = write_temp_sibling(&target, parent, name, bytes)?;
    let outcome = match expected {
        None => install_new(&target, cleanup, before_install, exchange)?,
        Some(expected) => replace_expected(
            &target,
            parent,
            name,
            cleanup,
            expected,
            max_len,
            before_install,
            exchange,
        )?,
    };
    if outcome != ReplaceOutcome::Conflict {
        finish_directory_sync(sync_directory(parent))?;
    }
    Ok(outcome)
}

/// Creates the missing `target` from the temp file, never replacing a file that appeared.
fn install_new(
    target: &Path,
    mut cleanup: TempCleanup,
    before_install: &mut dyn FnMut(),
    exchange: bool,
) -> Result<ReplaceOutcome> {
    if identity_now(target)?.is_some() {
        return Ok(ReplaceOutcome::Conflict);
    }
    before_install();
    let temp = cleanup.path.clone();
    #[cfg(unix)]
    if exchange {
        match rename_with(&temp, target, rustix::fs::RenameFlags::NOREPLACE)? {
            Swap::Done => {
                cleanup.armed = false;
                return Ok(ReplaceOutcome::Replaced);
            }
            Swap::Refused => return Ok(ReplaceOutcome::Conflict),
            Swap::Unsupported => {}
        }
    }
    #[cfg(not(unix))]
    let _ = exchange;
    // A hard link fails on an existing name; the temp name is then removed by `cleanup`.
    match fs::hard_link(&temp, target) {
        Ok(()) => return Ok(ReplaceOutcome::Replaced),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(ReplaceOutcome::Conflict),
        // FAT, exFAT and some network volumes have no hard links: check, then rename.
        Err(_) => {}
    }
    if identity_now(target)?.is_some() {
        return Ok(ReplaceOutcome::Conflict);
    }
    retry_rename(|| fs::rename(&temp, target), std::thread::sleep).map_err(io_error)?;
    cleanup.armed = false;
    Ok(ReplaceOutcome::Replaced)
}

/// Replaces `target` only while it holds `expected`; see [`PlainFileIo::replace_if`].
#[allow(clippy::too_many_arguments)]
fn replace_expected(
    target: &Path,
    parent: &Path,
    name: &std::ffi::OsStr,
    mut cleanup: TempCleanup,
    expected: &[u8; 32],
    max_len: u64,
    before_install: &mut dyn FnMut(),
    exchange: bool,
) -> Result<ReplaceOutcome> {
    // The final check: exactly the expected content, read on the opened handle.
    let Some((current, checked)) = read_current(target, max_len)? else {
        return Ok(ReplaceOutcome::Conflict);
    };
    if hash(&current) != *expected {
        return Ok(ReplaceOutcome::Conflict);
    }
    before_install();
    let temp = cleanup.path.clone();
    #[cfg(unix)]
    if exchange {
        match rename_with(&temp, target, rustix::fs::RenameFlags::EXCHANGE)? {
            Swap::Done => {
                // `temp` now names whatever was at the target at the instant of the swap.
                let displaced = read_current(&temp, max_len).ok().flatten();
                if displaced
                    .as_ref()
                    .is_some_and(|(bytes, _)| hash(bytes) == *expected)
                {
                    // The expected content: `cleanup` removes it.
                    return Ok(ReplaceOutcome::Replaced);
                }
                let kept = sibling_name(parent, name, ".displaced")?;
                fs::rename(&temp, &kept).map_err(io_error)?;
                cleanup.armed = false;
                return Ok(ReplaceOutcome::Displaced {
                    bytes: displaced.map(|(bytes, _)| bytes).unwrap_or_default(),
                    kept,
                });
            }
            Swap::Refused => return Ok(ReplaceOutcome::Conflict),
            Swap::Unsupported => {}
        }
    }
    #[cfg(not(unix))]
    let _ = (parent, name, exchange);
    // Portable fallback: the file is still the one checked, then a plain rename. The window
    // between this `stat` and the `rename` is the documented residual.
    if identity_now(target)? != Some(checked) {
        return Ok(ReplaceOutcome::Conflict);
    }
    retry_rename(|| fs::rename(&temp, target), std::thread::sleep).map_err(io_error)?;
    cleanup.armed = false;
    Ok(ReplaceOutcome::Replaced)
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

    fn replace(
        p: &Path,
        expected: Option<&[u8]>,
        bytes: &[u8],
        hook: &mut dyn FnMut(),
        exchange: bool,
    ) -> ReplaceOutcome {
        let expected = expected.map(hash);
        replace_if_with(p, expected.as_ref(), bytes, LIMIT, hook, exchange).unwrap()
    }
    fn modes() -> Vec<bool> {
        if cfg!(unix) {
            vec![true, false]
        } else {
            vec![false]
        }
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
    #[cfg(unix)]
    #[test]
    fn writer_racing_the_final_step_is_kept_and_returned_not_destroyed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let outcome = replace(
            &p,
            Some(b"old"),
            b"new",
            &mut || write_atomic(&racer, b"theirs").unwrap(),
            true,
        );
        let ReplaceOutcome::Displaced { bytes, kept } = outcome else {
            panic!("expected Displaced, got {outcome:?}");
        };
        assert_eq!(bytes, b"theirs");
        assert_eq!(fs::read(&p).unwrap(), b"new");
        assert_eq!(fs::read(&kept).unwrap(), b"theirs", "kept on disk");
        assert_eq!(
            kept.parent(),
            Some(d.path().canonicalize().unwrap().as_path())
        );
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
    }
    #[test]
    fn fallback_refuses_when_the_file_changed_before_its_final_stat() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("vault");
        fs::write(&p, b"old").unwrap();
        let racer = p.clone();
        let outcome = replace(
            &p,
            Some(b"old"),
            b"new",
            &mut || write_atomic(&racer, b"theirs").unwrap(),
            false,
        );
        assert_eq!(outcome, ReplaceOutcome::Conflict);
        assert_eq!(fs::read(&p).unwrap(), b"theirs");
        only_target(&d);
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
            replace_if_with(&p, Some(&hash(b"x")), b"new", LIMIT, &mut || {}, true),
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
}
