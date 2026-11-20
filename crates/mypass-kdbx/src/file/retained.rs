//! The identity of retained vault copies and their verified, explicit removal.
//!
//! A guarded replacement ([`super::FileIo::replace_if`]) can keep another writer's file next to
//! the vault (`.displaced`), or leave a working file behind after a failure (`.aside`,
//! `.exchanged`). MyPass never deletes one on its own: it may be the only copy of someone's
//! edit. The user may delete one explicitly, and this module makes that deletion safe:
//!
//! - **Identity at write time.** Every retained copy a replacement in this process creates is
//!   recorded right away with the resolved vault it belongs to, its exact length and the SHA-256
//!   of its bytes ([`RetainedIdentity`]). The record goes into a bounded in-process journal and,
//!   when the app installed one, to a listener that persists it ([`set_retained_copy_listener`]).
//! - **Which vault a copy belongs to.** Retained names carry only the first
//!   [`super::SIBLING_STEM_MAX`] bytes of the vault's file name, so two long vault names can share
//!   one listing. A copy with a record belongs to the recorded vault and to no other. A copy
//!   without one (written before records existed, or by a process that crashed before the record
//!   was persisted) is accepted only where a longer name cannot truncate to the same stem.
//!   These checks establish listing provenance. On deletion, the exact bytes must also open
//!   with the unlocked vault's key and have its root UUID ([`authenticate_retained_copy`]).
//! - **Removal re-verifies everything** ([`remove_retained_copy`]): the name must still be a
//!   retained name of this vault, opened without following a symbolic link, a regular file with
//!   one link, with the exact length and full SHA-256 the caller confirmed; the live vault must
//!   exist, be a readable KDBX file and be a different file. The copy is then moved to a fresh
//!   private name that is itself a listed `.displaced` name (so a failure never hides it), the
//!   moved file is proved to be the verified one, and only then is it unlinked. Any mismatch is a
//!   typed [`RetainedRefusal`] and deletes nothing.
//!
//! Residual (documented): a program that rewrites the verified copy in place, through a handle
//! it already holds, between the hash and the unlink. Retained copies are written once and
//! never reopened for writing by MyPass, KeePassXC or sync clients.

use super::{
    displaced_copies, displaced_kind, io_error, is_retained_file, rename_noreplace, resolve_target,
    resolve_using, split_target, sync_directory, ActiveAside, DisplacedCopyKind, ReplaceOutcome,
    SiblingNames, SIBLING_STEM_MAX,
};
use crate::{Database, KdbxError, Result, MAX_VAULT_FILE_BYTES};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock},
    time::{Duration, SystemTime},
};
use uuid::Uuid;

/// A working file may still be a replacement's rollback source during this grace period.
/// On Unix it starts at the last inode change (including a rename). Stable Windows Rust only
/// exposes the last write time for this check, so there it covers recent writes to the file.
pub const RETAINED_IN_FLIGHT_GRACE: Duration = Duration::from_secs(5 * 60);

/// How many identities the in-process journal keeps (the newest win).
const JOURNAL_MAX: usize = 256;

/// The exact identity of one retained copy, recorded when the replacement kept it.
#[derive(Clone, PartialEq, Eq)]
pub struct RetainedIdentity {
    /// The resolved vault file whose replacement kept the copy.
    pub target: PathBuf,
    /// The copy: the resolved folder of `target` joined with the copy's file name.
    pub path: PathBuf,
    /// Its length in bytes.
    pub len: u64,
    /// The SHA-256 of its bytes.
    pub sha256: [u8; 32],
}

impl fmt::Debug for RetainedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedIdentity")
            .field("target", &self.target)
            .field("path", &self.path)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// Called with every identity recorded in this process (see [`set_retained_copy_listener`]).
pub type RetainedCopyListener = Arc<dyn Fn(&RetainedIdentity) + Send + Sync>;

static LISTENER: RwLock<Option<RetainedCopyListener>> = RwLock::new(None);

fn journal() -> MutexGuard<'static, Vec<RetainedIdentity>> {
    static JOURNAL: Mutex<Vec<RetainedIdentity>> = Mutex::new(Vec::new());
    JOURNAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Installs (or, with `None`, removes) the process-wide listener that persists every retained
/// copy identity at the moment it is recorded. The app installs one at startup.
pub fn set_retained_copy_listener(listener: Option<RetainedCopyListener>) {
    *LISTENER.write().unwrap_or_else(PoisonError::into_inner) = listener;
}

/// The identities recorded by replacements in this process (bounded, oldest first).
pub fn recorded_retained_copies() -> Vec<RetainedIdentity> {
    journal().clone()
}

/// Why a retained copy may not be deleted. Every refusal deletes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedRefusal {
    /// The name is not (or no longer) a retained copy of this vault: a temp or in-flight name,
    /// the vault itself, a path with separators, or a file that is gone.
    NotRetainedCopy,
    /// A working file that a replacement in progress may still need.
    InFlight,
    /// The bytes, the length or the file at the name differ from what was confirmed or recorded.
    Changed,
    /// A symbolic link, a reparse point, a directory, a device or a file with several links.
    NotRegularFile,
    /// The record names another vault, or the authenticated copy has a different root UUID.
    OtherVault,
    /// The unrecorded stem is ambiguous, or the copy could not be authenticated and opened.
    UnverifiedVault,
    /// The live vault is missing, unreadable or not a KDBX file: the copy may be the only one.
    LiveVaultMissing,
    /// The copy is the live vault file itself.
    SameAsLiveVault,
    /// The copy could not be read to verify it.
    Unreadable,
}

impl RetainedRefusal {
    /// A stable camelCase name, for diagnostics and the app's error message.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRetainedCopy => "notRetainedCopy",
            Self::InFlight => "inFlight",
            Self::Changed => "changed",
            Self::NotRegularFile => "notRegularFile",
            Self::OtherVault => "otherVault",
            Self::UnverifiedVault => "unverifiedVault",
            Self::LiveVaultMissing => "liveVaultMissing",
            Self::SameAsLiveVault => "sameAsLiveVault",
            Self::Unreadable => "unreadable",
        }
    }
}

impl fmt::Display for RetainedRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a deletable copy's identity comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedProvenance {
    /// A write-time record of this vault matches the copy exactly.
    Recorded,
    /// No record; the exact name match and the bytes hashed now are all there is.
    Unrecorded,
}

/// One listed retained copy with its verification.
#[derive(Clone, PartialEq, Eq)]
pub struct RetainedCopyCheck {
    /// Where the copy is (inside the resolved folder of the vault).
    pub path: PathBuf,
    /// Its length in bytes when listed.
    pub len: u64,
    /// How the replacement named it.
    pub kind: DisplacedCopyKind,
    /// The SHA-256 of the bytes read now; `None` when they could not be read.
    pub sha256: Option<[u8; 32]>,
    /// Whether it may be deleted, and on what grounds.
    pub verdict: std::result::Result<RetainedProvenance, RetainedRefusal>,
}

impl fmt::Debug for RetainedCopyCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedCopyCheck")
            .field("path", &self.path)
            .field("len", &self.len)
            .field("kind", &self.kind)
            .field("verdict", &self.verdict)
            .finish_non_exhaustive()
    }
}

/// The retained copies of one vault, verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedInspection {
    /// The vault resolved as the writer resolves it.
    pub target: PathBuf,
    /// The copies, sorted by path.
    pub copies: Vec<RetainedCopyCheck>,
}

/// What the user confirmed: the copy's file name next to the vault, its length and SHA-256.
#[derive(Clone, PartialEq, Eq)]
pub struct RetainedRemoval {
    /// The vault path as configured (resolved again before anything is checked).
    pub target: PathBuf,
    /// The copy's bare file name.
    pub name: OsString,
    /// The confirmed length in bytes.
    pub len: u64,
    /// The confirmed SHA-256.
    pub sha256: [u8; 32],
}

impl fmt::Debug for RetainedRemoval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedRemoval")
            .field("target", &self.target)
            .field("name", &self.name)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// Proof that the confirmed retained bytes opened with the live vault's key and root UUID.
/// The file layer re-verifies the path and bytes before unlinking.
#[derive(Debug)]
pub struct AuthenticatedRetainedRemoval {
    request: RetainedRemoval,
    target: PathBuf,
}

impl AuthenticatedRetainedRemoval {
    /// The resolved vault path whose neighbouring copy was authenticated.
    pub fn target(&self) -> &Path {
        &self.target
    }
}

fn refused(reason: RetainedRefusal) -> KdbxError {
    KdbxError::RetainedCopyRefused(reason)
}

/// Records the retained copies a replacement of `target` (resolved) produced: the kept copy and
/// any leftover working name of a `Displaced` outcome, and the file a failed replacement kept.
/// Temp names are never recorded (they are never listed). A copy that cannot be read is not
/// recorded; it stays listed, without a record.
pub(super) fn record_outcome(target: &Path, outcome: &Result<ReplaceOutcome>) {
    let paths: Vec<&Path> = match outcome {
        Ok(ReplaceOutcome::Displaced { kept, leftover, .. }) => std::iter::once(kept.as_path())
            .chain(leftover.as_deref())
            .collect(),
        Err(KdbxError::VaultRetained { path, .. }) => vec![path.as_path()],
        _ => return,
    };
    for path in paths {
        if let Some(identity) = identity_of(target, path) {
            record(identity);
        }
    }
}

fn record(identity: RetainedIdentity) {
    {
        let mut journal = journal();
        journal.retain(|known| known.path != identity.path);
        journal.push(identity.clone());
        let excess = journal.len().saturating_sub(JOURNAL_MAX);
        journal.drain(..excess);
    }
    let listener = LISTENER
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(listener) = listener {
        listener(&identity);
    }
}

fn identity_of(target: &Path, copy: &Path) -> Option<RetainedIdentity> {
    let (_, target_name) = split_target(target).ok()?;
    let name = copy.file_name()?;
    displaced_kind(&target_name.to_string_lossy(), &name.to_string_lossy())?;
    let (file, meta) = open_no_follow(copy).ok()?;
    if !is_retained_file(name, &meta) || !is_regular(&meta) {
        return None;
    }
    let (len, sha256) = hash_open(&file, MAX_VAULT_FILE_BYTES).ok()?;
    Some(RetainedIdentity {
        target: target.to_path_buf(),
        path: copy.to_path_buf(),
        len,
        sha256,
    })
}

/// Lists the retained copies of `target` (as [`displaced_copies`]) and verifies each one against
/// the write-time records in `known` and in this process's journal. Read-only.
pub fn inspect_retained_copies(
    target: &Path,
    known: &[RetainedIdentity],
) -> Result<RetainedInspection> {
    let resolved =
        resolve_using(|| resolve_target(target), std::thread::sleep).map_err(io_error)?;
    let journal = recorded_retained_copies();
    let now = SystemTime::now();
    let copies = displaced_copies(target)?
        .into_iter()
        .map(|copy| {
            let (sha256, verdict) = match verify_copy(&copy.path, None) {
                Ok(found) => (
                    Some(found.sha256),
                    provenance(
                        &resolved, &copy.path, copy.kind, &found, known, &journal, now,
                    ),
                ),
                Err(reason) => (None, Err(reason)),
            };
            RetainedCopyCheck {
                path: copy.path,
                len: copy.len,
                kind: copy.kind,
                sha256,
                verdict,
            }
        })
        .collect();
    Ok(RetainedInspection {
        target: resolved,
        copies,
    })
}

/// What reading a copy proved.
struct Verified {
    len: u64,
    sha256: [u8; 32],
    last_change: Option<SystemTime>,
    #[cfg(unix)]
    dev_ino: (u64, u64),
    /// The open handle, kept until the removal is done so the inode cannot be reused.
    _file: fs::File,
}

/// Opens `path` without following a link and checks its type, link count and expected length.
fn open_copy(
    path: &Path,
    expected_len: Option<u64>,
) -> std::result::Result<(fs::File, fs::Metadata), RetainedRefusal> {
    let (file, meta) = open_no_follow(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => RetainedRefusal::NotRetainedCopy,
        _ if is_link_refusal(&e) => RetainedRefusal::NotRegularFile,
        _ => RetainedRefusal::Unreadable,
    })?;
    if !is_regular(&meta) || !single_link(&meta) {
        return Err(RetainedRefusal::NotRegularFile);
    }
    if expected_len.is_some_and(|len| len != meta.len()) {
        return Err(RetainedRefusal::Changed);
    }
    Ok((file, meta))
}

/// Opens a retained copy and hashes all of its bytes.
fn verify_copy(
    path: &Path,
    expected_len: Option<u64>,
) -> std::result::Result<Verified, RetainedRefusal> {
    let (file, meta) = open_copy(path, expected_len)?;
    let max = expected_len.unwrap_or(MAX_VAULT_FILE_BYTES);
    let (len, sha256) = hash_open(&file, max).map_err(|e| match e.kind() {
        io::ErrorKind::FileTooLarge => RetainedRefusal::Changed,
        _ => RetainedRefusal::Unreadable,
    })?;
    if expected_len.is_some_and(|expected| expected != len) {
        return Err(RetainedRefusal::Changed);
    }
    Ok(Verified {
        len,
        sha256,
        last_change: last_working_change(&meta),
        #[cfg(unix)]
        dev_ino: dev_ino(&meta),
        _file: file,
    })
}

#[cfg(unix)]
fn last_working_change(meta: &fs::Metadata) -> Option<SystemTime> {
    use std::os::unix::fs::MetadataExt;

    let seconds = meta.ctime();
    let whole_seconds = Duration::from_secs(seconds.unsigned_abs());
    let whole_time = if seconds < 0 {
        SystemTime::UNIX_EPOCH.checked_sub(whole_seconds)?
    } else {
        SystemTime::UNIX_EPOCH.checked_add(whole_seconds)?
    };
    let nanos = u64::try_from(meta.ctime_nsec()).ok()?;
    whole_time.checked_add(Duration::from_nanos(nanos))
}

#[cfg(not(unix))]
fn last_working_change(meta: &fs::Metadata) -> Option<SystemTime> {
    meta.modified().ok()
}

/// Decides whether a verified copy may be deleted.
fn provenance(
    target: &Path,
    copy: &Path,
    kind: DisplacedCopyKind,
    found: &Verified,
    known: &[RetainedIdentity],
    journal: &[RetainedIdentity],
    now: SystemTime,
) -> std::result::Result<RetainedProvenance, RetainedRefusal> {
    if kind != DisplacedCopyKind::Displaced {
        let settled = found
            .last_change
            .and_then(|last_change| now.duration_since(last_change).ok())
            .is_some_and(|age| age >= RETAINED_IN_FLIGHT_GRACE);
        if !settled {
            return Err(RetainedRefusal::InFlight);
        }
    }
    // The newest record of this exact path wins; the journal is newer than persisted records.
    let record = journal
        .iter()
        .rev()
        .chain(known.iter().rev())
        .find(|r| r.path == copy);
    match record {
        Some(r) if r.target != target => Err(RetainedRefusal::OtherVault),
        Some(r) if r.len != found.len || r.sha256 != found.sha256 => Err(RetainedRefusal::Changed),
        Some(_) => Ok(RetainedProvenance::Recorded),
        None if stem_is_exact(target) => Ok(RetainedProvenance::Unrecorded),
        None => Err(RetainedRefusal::UnverifiedVault),
    }
}

/// Whether retained names of `target` carry its whole file name (no truncation), so a name
/// match cannot also match another vault in the folder.
fn stem_is_exact(target: &Path) -> bool {
    // A longer name may have a four-byte UTF-8 character just beyond this name. Leave
    // enough room for that character so truncation never backs up to this exact stem.
    split_target(target).is_ok_and(|(_, name)| {
        name.to_str()
            .is_some_and(|name| name.len() <= SIBLING_STEM_MAX - 4)
    })
}

/// Authenticates the confirmed bytes under the local KDF limits and checks the root UUID.
/// Read-only, and only used at delete time so listing never runs a KDF.
pub fn authenticate_retained_copy(
    request: RetainedRemoval,
    password: &SecretString,
    root: Uuid,
) -> Result<AuthenticatedRetainedRemoval> {
    let target = resolve_using(|| resolve_target(&request.target), std::thread::sleep)
        .map_err(|_| refused(RetainedRefusal::Unreadable))?;
    let (parent, target_name) = split_target(&target)?;
    let name = bare_name(&request.name).ok_or(refused(RetainedRefusal::NotRetainedCopy))?;
    displaced_kind(&target_name.to_string_lossy(), &name.to_string_lossy())
        .ok_or(refused(RetainedRefusal::NotRetainedCopy))?;
    let (file, meta) = open_copy(&parent.join(name), Some(request.len)).map_err(refused)?;
    if !is_retained_file(name, &meta) {
        return Err(refused(RetainedRefusal::NotRetainedCopy));
    }
    let bytes = super::read_opened(
        super::OpenedFile::new(Box::new(file), true, meta.len()),
        request.len.min(MAX_VAULT_FILE_BYTES),
    )
    .map_err(|err| match err {
        KdbxError::Io {
            kind: io::ErrorKind::FileTooLarge,
            ..
        } => refused(RetainedRefusal::Changed),
        _ => refused(RetainedRefusal::Unreadable),
    })?;
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    if u64::try_from(bytes.len()).ok() != Some(request.len) || sha256 != request.sha256 {
        return Err(refused(RetainedRefusal::Changed));
    }
    let db =
        Database::open(&bytes, password).map_err(|_| refused(RetainedRefusal::UnverifiedVault))?;
    if db.root_id() != root {
        return Err(refused(RetainedRefusal::OtherVault));
    }
    Ok(AuthenticatedRetainedRemoval { request, target })
}

/// Deletes one retained copy of `removal.target` after re-verifying it (see the module docs).
/// `known` holds the persisted write-time records.
pub fn remove_retained_copy(
    removal: &AuthenticatedRetainedRemoval,
    known: &[RetainedIdentity],
) -> Result<()> {
    remove_with(
        removal,
        known,
        SystemTime::now(),
        &mut RemoveSeams {
            before_move: &mut || {},
            unlink: &mut remove_private_name,
        },
    )
}

/// Test seams of the removal.
pub(super) struct RemoveSeams<'a> {
    /// Runs after every check passed, right before the copy is moved to its private name.
    pub(super) before_move: &'a mut dyn FnMut(),
    /// Unlinks the private name.
    pub(super) unlink: &'a mut dyn FnMut(&Path) -> io::Result<()>,
}

/// The one unlink of a verified retained copy, under its private removal name.
fn remove_private_name(private: &Path) -> io::Result<()> {
    fs::remove_file(private)
}

pub(super) fn remove_with(
    authenticated: &AuthenticatedRetainedRemoval,
    known: &[RetainedIdentity],
    now: SystemTime,
    seams: &mut RemoveSeams<'_>,
) -> Result<()> {
    let removal = &authenticated.request;
    let target =
        resolve_using(|| resolve_target(&removal.target), std::thread::sleep).map_err(io_error)?;
    if target != authenticated.target {
        return Err(refused(RetainedRefusal::Changed));
    }
    let (parent, target_name) = split_target(&target)?;
    let name = bare_name(&removal.name).ok_or(refused(RetainedRefusal::NotRetainedCopy))?;
    let kind = displaced_kind(&target_name.to_string_lossy(), &name.to_string_lossy())
        .ok_or(refused(RetainedRefusal::NotRetainedCopy))?;
    if kind != DisplacedCopyKind::Displaced && ActiveAside::contains(name) {
        return Err(refused(RetainedRefusal::InFlight));
    }
    let copy = parent.join(name);
    if copy == target {
        return Err(refused(RetainedRefusal::SameAsLiveVault));
    }
    let found = verify_copy(&copy, Some(removal.len)).map_err(refused)?;
    if !is_retained_file(name, &found._file.metadata().map_err(io_error)?) {
        return Err(refused(RetainedRefusal::NotRetainedCopy));
    }
    if found.sha256 != removal.sha256 {
        return Err(refused(RetainedRefusal::Changed));
    }
    provenance(
        &target,
        &copy,
        kind,
        &found,
        known,
        &recorded_retained_copies(),
        now,
    )
    .map_err(refused)?;
    check_live_vault(&target, &found)?;
    (seams.before_move)();
    let private = move_to_private_name(&copy, parent, target_name)?;
    if let Err(reason) = confirm_moved(&private, &found) {
        // Not the file we verified: put it back where it was (never replacing anything). If that
        // fails it stays under the private name, which is itself a listed `.displaced` name.
        let _ = rename_noreplace(&private, &copy);
        return Err(refused(reason));
    }
    // A failed unlink leaves the copy under the private, listed name: never hidden.
    (seams.unlink)(&private).map_err(io_error)?;
    let _ = sync_directory(parent);
    Ok(())
}

/// `name` when it is one plain file name (no separators, no `.` or `..`).
fn bare_name(name: &OsStr) -> Option<&OsStr> {
    let path = Path::new(name);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(only)), None) if only == name => Some(only),
        _ => None,
    }
}

/// The live vault must be there, readable, a KDBX file and not the copy: otherwise the copy may
/// be the only one left.
fn check_live_vault(target: &Path, copy: &Verified) -> Result<()> {
    let missing = || refused(RetainedRefusal::LiveVaultMissing);
    let file = super::open_for_read(target).map_err(|_| missing())?;
    let meta = file.metadata().map_err(|_| missing())?;
    if !meta.is_file() || meta.len() < 8 {
        return Err(missing());
    }
    #[cfg(unix)]
    if dev_ino(&meta) == copy.dev_ino {
        return Err(refused(RetainedRefusal::SameAsLiveVault));
    }
    #[cfg(not(unix))]
    let _ = copy;
    let mut signature = [0u8; 8];
    (&file).read_exact(&mut signature).map_err(|_| missing())?;
    let kdbx = [0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5];
    if signature != kdbx {
        return Err(missing());
    }
    Ok(())
}

/// Moves `copy` to a fresh `.displaced` name of its own, never replacing a file: a no-replace
/// rename. A volume without that primitive cannot safely do the private move.
fn move_to_private_name(copy: &Path, parent: &Path, target_name: &OsStr) -> Result<PathBuf> {
    for _ in 0..super::PUBLISH_ATTEMPTS {
        let private = SiblingNames::new(parent, target_name)?.displaced;
        match rename_noreplace(copy, &private) {
            Ok(()) => return Ok(private),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(refused(RetainedRefusal::Changed))
            }
            Err(e) => return Err(io_error(e)),
        }
    }
    Err(io_error(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free private name to remove the copy",
    )))
}

/// Proves the file now at `private` is the one that was verified: the same inode (Unix, where
/// the verified handle is still open) or, where std exposes no file id, the same bytes.
fn confirm_moved(private: &Path, found: &Verified) -> std::result::Result<(), RetainedRefusal> {
    #[cfg(unix)]
    {
        let meta = fs::symlink_metadata(private).map_err(|_| RetainedRefusal::Changed)?;
        if dev_ino(&meta) != found.dev_ino || !meta.is_file() {
            return Err(RetainedRefusal::Changed);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let again = verify_copy(private, Some(found.len))?;
        if again.sha256 != found.sha256 {
            return Err(RetainedRefusal::Changed);
        }
        Ok(())
    }
}

/// Streams at most `max_len` bytes of `file` into SHA-256; longer is `FileTooLarge`.
fn hash_open(file: &fs::File, max_len: u64) -> io::Result<(u64, [u8; 32])> {
    let mut hasher = Sha256::new();
    let mut reader = file.take(max_len.saturating_add(1));
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(buf.get(..n).unwrap_or_default());
        total = total.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    }
    if total > max_len {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "retained copy is too large",
        ));
    }
    Ok((total, hasher.finalize().into()))
}

/// Opens without following a final symbolic link (`O_NOFOLLOW`), without blocking on a FIFO,
/// and returns the handle's own metadata.
#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<(fs::File, fs::Metadata)> {
    use std::os::unix::fs::OpenOptionsExt;
    let flags =
        (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOCTTY)
            .bits();
    let flags = i32::try_from(flags).map_err(|_| io::Error::other("open flags out of range"))?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)?;
    let meta = file.metadata()?;
    Ok((file, meta))
}

/// Opens the reparse point itself, never its target (`FILE_FLAG_OPEN_REPARSE_POINT`); the
/// handle's attributes then tell a link or junction from a plain file.
#[cfg(windows)]
fn open_no_follow(path: &Path) -> io::Result<(fs::File, fs::Metadata)> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let meta = file.metadata()?;
    Ok((file, meta))
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(path: &Path) -> io::Result<(fs::File, fs::Metadata)> {
    let _ = path;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no-follow open unsupported",
    ))
}

fn is_link_refusal(e: &io::Error) -> bool {
    #[cfg(unix)]
    {
        e.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
    }
    #[cfg(not(unix))]
    {
        let _ = e;
        false
    }
}

fn is_regular(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.is_file() && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(not(windows))]
    {
        meta.is_file()
    }
}

/// One link only (Unix). Windows exposes no link count in stable std; there a removal drops one
/// name only, and the same-file check relies on the reparse refusal and the distinct name.
fn single_link(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.nlink() == 1
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        true
    }
}

#[cfg(unix)]
fn dev_ino(meta: &fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(test)]
#[path = "retained_tests.rs"]
mod tests;
