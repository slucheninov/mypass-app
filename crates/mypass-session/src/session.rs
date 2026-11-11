//! The vault session: one open vault, its lock state, guarded access and the quit gate.
//!
//! [`VaultSession`] is a cheap `Clone` handle over one `Mutex`. States: `NoVault` (no path),
//! `Locked` (a path, no database in memory) and `Unlocked` (database and password in memory).
//!
//! Edits go through [`VaultSession::mutate`], which works on a CLONE of the database and swaps it
//! in only when the closure returns `Ok`: an `Err` leaves the live database exactly as it was.
//! The cost is one clone per edit (typical vaults are a few MiB); it was chosen over undo logs
//! because the rollback is obviously correct.
//!
//! Saving in this module is the simple synchronous form: [`VaultSession::flush`] writes through
//! the guarded `VaultFile::save` while holding the session lock. The debounced autosave, the
//! retry and the external-change merge replace it in the saving module (Task 7). A failed save on
//! lock keeps the encrypted bytes (`pending_bytes`) and restores them on the next unlock, so
//! locking never drops an edit.
//!
//! Locking discipline: observers and the conflict hook are always called after the session lock
//! is released, so they may call back into the session. Closures passed to `with_db`, `mutate`,
//! `touch` and `with_db_and_password` run UNDER the lock and must not call the session.

use crate::scheduler::{lock, Scheduler};
use crate::settings::{SettingsPatch, SettingsStore};
use crate::vault_files::{file_status, VaultFileStatus};
use mypass_kdbx::{Database, FileIo, FileMeta, KdbxError, Timestamp, VaultFile};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::UNIX_EPOCH;

/// Lifecycle state shown by the UI.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    NoVault,
    Locked,
    Unlocked,
}

/// Why the session was locked.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum LockReason {
    Manual,
    Idle,
    Sleep,
    ScreenLock,
    Hotkey,
    Quit,
}

/// Where the unsaved-changes pipeline stands.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SaveState {
    Idle,
    Pending,
    Saving,
    Failed,
}

/// Save state plus the last failure message (no secret content, file-system wording only).
#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SaveStatus {
    pub state: SaveState,
    pub message: Option<String>,
}

impl SaveStatus {
    /// Nothing to save.
    pub fn idle() -> Self {
        Self {
            state: SaveState::Idle,
            message: None,
        }
    }
    fn pending() -> Self {
        Self {
            state: SaveState::Pending,
            message: None,
        }
    }
    fn failed(message: String) -> Self {
        Self {
            state: SaveState::Failed,
            message: Some(message),
        }
    }
}

/// Snapshot of the session for the UI (the `SessionChanged` event payload and `vault_status`).
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub state: SessionState,
    pub vault_path: Option<String>,
    pub vault_name: Option<String>,
    pub lock_reason: Option<LockReason>,
    pub file_status: VaultFileStatus,
    pub save: SaveStatus,
}

/// What observers are told.
#[derive(Clone, PartialEq, Debug)]
pub enum SessionEvent {
    /// Lock state, vault or path changed.
    StateChanged(SessionStatus),
    /// The vault content changed in memory.
    VaultChanged,
    /// The save state changed.
    SaveChanged(SaveStatus),
}

/// A session observer; runs after the session lock is released, on the calling thread.
pub type Observer = Arc<dyn Fn(&SessionEvent) + Send + Sync>;

/// Why the password generator could not produce a value (raised from Task 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GeneratorRulesError {
    #[error("no character class selected")]
    NoClass,
    #[error("the site's password rules cannot be satisfied")]
    Unsatisfiable,
}

/// Every session failure. Messages never contain secrets.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SessionError {
    #[error("the vault is locked")]
    Locked,
    #[error("no vault is open")]
    NoVault,
    #[error("wrong password")]
    WrongPassword,
    #[error("the file already exists")]
    AlreadyExists,
    #[error("not found")]
    NotFound,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("corrupt vault: {0}")]
    Corrupt(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("the vault file changed on disk")]
    ChangedOnDisk,
    #[error("i/o error: {0}")]
    Io(String),
    #[error("the app is quitting")]
    Quitting,
    #[error("generator: {0}")]
    GeneratorRules(GeneratorRulesError),
}

impl From<KdbxError> for SessionError {
    fn from(e: KdbxError) -> Self {
        match e {
            KdbxError::WrongPassword => Self::WrongPassword,
            KdbxError::Corrupt(m) | KdbxError::InvalidKdfParameters(m) => Self::Corrupt(m),
            e @ KdbxError::NotKdbx => Self::Corrupt(e.to_string()),
            e @ KdbxError::UnsupportedVersion { .. } => Self::Unsupported(e.to_string()),
            KdbxError::Unsupported(m) => Self::Unsupported(m),
            KdbxError::Io { message, .. } => Self::Io(message),
            KdbxError::InvalidOperation(m) => Self::InvalidInput(m),
            e @ KdbxError::Random => Self::Io(e.to_string()),
        }
    }
}

/// Like `From`, but a missing file is `NotFound` (opening a path the user picked or remembered).
fn open_error(e: KdbxError) -> SessionError {
    match e {
        KdbxError::Io {
            kind: ErrorKind::NotFound,
            ..
        } => SessionError::NotFound,
        other => other.into(),
    }
}

/// Which remote-content reconciliation happened (reported by the saving module, Task 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictEvent {
    MergedOnSave,
    MergedOnCheck,
    ReloadedOnCheck,
}

/// Called after a merge or reload of remote content; default no-op.
pub type ConflictHook = Arc<dyn Fn(ConflictEvent) + Send + Sync>;

/// What a session is built from.
pub struct SessionDeps {
    pub settings: SettingsStore,
    pub scheduler: Arc<dyn Scheduler>,
    /// `AppPaths.backups_dir`.
    pub backups_root: PathBuf,
    /// Production: `Arc::new(PlainFileIo)`.
    pub file_io: Arc<dyn FileIo>,
}

/// The IO seam every `VaultFile` of the session is opened with: it forwards to a replaceable
/// target, so [`VaultSession::set_file_io`] takes effect for the next save without reopening
/// the file (reopening would refresh the change-detection stamp and could hide an external edit).
struct SwitchIo(RwLock<Arc<dyn FileIo>>);

impl SwitchIo {
    fn get(&self) -> Arc<dyn FileIo> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn set(&self, io: Arc<dyn FileIo>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = io;
    }
}

impl FileIo for SwitchIo {
    fn read(&self, path: &Path) -> mypass_kdbx::Result<Vec<u8>> {
        self.get().read(path)
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> mypass_kdbx::Result<()> {
        self.get().write_atomic(path, bytes)
    }
    fn metadata(&self, path: &Path) -> mypass_kdbx::Result<FileMeta> {
        self.get().metadata(path)
    }
}

pub(crate) struct Inner {
    path: Option<PathBuf>,
    vault_name: Option<String>,
    vault_id: Option<String>,
    file: Option<VaultFile>,
    db: Option<Database>,
    /// The master password, only while unlocked; `SecretString` zeroizes on drop.
    pub(crate) password: Option<SecretString>,
    dirty_gen: u64,
    saved_gen: u64,
    /// The encrypted database kept when the save on lock failed; restored on the next unlock.
    pending_bytes: Option<Vec<u8>>,
    save: SaveStatus,
    lock_reason: Option<LockReason>,
    quitting: bool,
    /// Called by the save and external-check paths (Task 7).
    conflict_hook: ConflictHook,
}

impl Inner {
    pub(crate) fn new() -> Self {
        Self {
            path: None,
            vault_name: None,
            vault_id: None,
            file: None,
            db: None,
            password: None,
            dirty_gen: 0,
            saved_gen: 0,
            pending_bytes: None,
            save: SaveStatus::idle(),
            lock_reason: None,
            quitting: false,
            conflict_hook: Arc::new(|_| {}),
        }
    }

    fn state(&self) -> SessionState {
        match (&self.db, &self.path) {
            (Some(_), _) => SessionState::Unlocked,
            (None, Some(_)) => SessionState::Locked,
            (None, None) => SessionState::NoVault,
        }
    }

    fn not_unlocked(&self) -> SessionError {
        if self.path.is_some() {
            SessionError::Locked
        } else {
            SessionError::NoVault
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty_gen != self.saved_gen
    }

    /// Sets the save status; returns it when it changed (to notify after unlocking).
    fn set_save(&mut self, s: SaveStatus) -> Option<SaveStatus> {
        (self.save != s).then(|| {
            self.save = s.clone();
            s
        })
    }

    /// Synchronous guarded save of the live database when it is dirty.
    fn flush(&mut self) -> (Result<(), SessionError>, Option<SaveStatus>) {
        if self.db.is_none() {
            if self.pending_bytes.is_some() {
                let msg = "unsaved changes are kept until the next unlock".to_owned();
                return (Err(SessionError::Io(msg)), None);
            }
            return (Ok(()), None);
        }
        if !self.is_dirty() {
            return (Ok(()), None);
        }
        let (Some(file), Some(db), Some(password)) = (&mut self.file, &self.db, &self.password)
        else {
            return (Err(SessionError::Locked), None);
        };
        let target = self.dirty_gen;
        match file.save(db, password) {
            Ok(mypass_kdbx::SaveOutcome::Saved) => {
                self.saved_gen = target;
                (Ok(()), self.set_save(SaveStatus::idle()))
            }
            Ok(mypass_kdbx::SaveOutcome::ChangedOnDisk) => {
                let e = SessionError::ChangedOnDisk;
                let changed = self.set_save(SaveStatus::failed(e.to_string()));
                (Err(e), changed)
            }
            Err(e) => {
                let e = SessionError::from(e);
                let changed = self.set_save(SaveStatus::failed(e.to_string()));
                (Err(e), changed)
            }
        }
    }

    /// Makes `file` + `db` the open vault. The caller has flushed the previous one.
    fn install(&mut self, file: VaultFile, db: Database, password: SecretString) {
        let path = file.path().to_path_buf();
        let name = Some(db.meta.database_name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()));
        self.vault_name = name;
        self.vault_id = Some(db.root_id().simple().to_string());
        self.path = Some(path);
        self.file = Some(file);
        self.db = Some(db);
        self.password = Some(password);
        self.lock_reason = None;
        self.dirty_gen = 0;
        self.saved_gen = 0;
        self.pending_bytes = None;
        self.save = SaveStatus::idle();
    }
}

struct Shared {
    inner: Mutex<Inner>,
    io: Arc<SwitchIo>,
    observers: RwLock<Vec<Observer>>,
    deps: SessionDeps,
}

/// The vault session handle (cheap to clone; every clone is the same session).
#[derive(Clone)]
pub struct VaultSession(Arc<Shared>);

impl std::fmt::Debug for VaultSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = lock(&self.0.inner);
        f.debug_struct("VaultSession")
            .field("state", &inner.state())
            .field("path", &inner.path)
            .field("save", &inner.save.state)
            .finish_non_exhaustive()
    }
}

/// Events gathered under the lock and delivered after it is released.
#[derive(Default)]
struct Outbox {
    state_changed: bool,
    vault_changed: bool,
    save: Option<SaveStatus>,
}

/// Compares two secrets without an early exit on the first differing byte (the length still
/// shows, as with any password comparison).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl VaultSession {
    pub fn new(deps: SessionDeps) -> Self {
        let io = Arc::new(SwitchIo(RwLock::new(deps.file_io.clone())));
        Self(Arc::new(Shared {
            inner: Mutex::new(Inner::new()),
            io,
            observers: RwLock::new(Vec::new()),
            deps,
        }))
    }

    /// Registers an observer for every later event.
    pub fn add_observer(&self, o: Observer) {
        self.0
            .observers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(o);
    }

    /// The current status (stats the vault file once, outside the session lock).
    pub fn status(&self) -> SessionStatus {
        let (state, path, vault_name, lock_reason, save) = {
            let inner = lock(&self.0.inner);
            (
                inner.state(),
                inner.path.clone(),
                inner.vault_name.clone(),
                inner.lock_reason,
                inner.save.clone(),
            )
        };
        SessionStatus {
            state,
            vault_path: path.as_deref().map(path_string),
            vault_name,
            lock_reason,
            file_status: file_status(path.as_deref()),
            save,
        }
    }

    pub fn is_unlocked(&self) -> bool {
        lock(&self.0.inner).db.is_some()
    }

    /// `Database::root_id()` in simple (32 lowercase hex) form; kept while locked.
    pub fn vault_id(&self) -> Option<String> {
        lock(&self.0.inner).vault_id.clone()
    }

    /// The canonical path of the vault file; kept while locked.
    pub fn vault_path(&self) -> Option<PathBuf> {
        lock(&self.0.inner).path.clone()
    }

    fn emit(&self, out: Outbox) {
        let mut events = Vec::new();
        if out.state_changed {
            events.push(SessionEvent::StateChanged(self.status()));
        }
        if out.vault_changed {
            events.push(SessionEvent::VaultChanged);
        }
        if let Some(s) = out.save {
            events.push(SessionEvent::SaveChanged(s));
        }
        if events.is_empty() {
            return;
        }
        let observers = self
            .0
            .observers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for e in &events {
            for o in &observers {
                o(e);
            }
        }
    }

    fn check_not_quitting(&self) -> Result<(), SessionError> {
        if lock(&self.0.inner).quitting {
            Err(SessionError::Quitting)
        } else {
            Ok(())
        }
    }

    fn now(&self) -> Timestamp {
        match self.0.deps.scheduler.wall_now().duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp(i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
            Err(e) => Timestamp(-i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX)),
        }
    }

    /// Flushes the current vault before another one replaces it; an unsaved edit blocks the swap.
    fn flush_before_switch(&self, inner: &mut Inner, out: &mut Outbox) -> Result<(), SessionError> {
        if inner.db.is_none() && inner.pending_bytes.is_some() {
            return Err(SessionError::InvalidInput(
                "the open vault has unsaved changes; unlock it to save them first".into(),
            ));
        }
        let (r, save) = inner.flush();
        out.save = save.or(out.save.take());
        r
    }

    /// Installs a freshly opened vault (after flushing the previous one), then records it in the
    /// settings and notifies.
    fn finish_open(
        &self,
        file: VaultFile,
        db: Database,
        password: SecretString,
    ) -> Result<(), SessionError> {
        let mut out = Outbox::default();
        let path = {
            let mut inner = lock(&self.0.inner);
            if inner.quitting {
                return Err(SessionError::Quitting);
            }
            let flushed = self.flush_before_switch(&mut inner, &mut out);
            if let Err(e) = flushed {
                drop(inner);
                self.emit(out);
                return Err(e);
            }
            inner.install(file, db, password);
            out.state_changed = true;
            out.vault_changed = true;
            inner.path.clone()
        };
        if let Some(p) = path {
            self.remember(&p);
        }
        self.emit(out);
        Ok(())
    }

    /// Records `path` as the current and most recent vault (best effort: the vault is open
    /// whether or not the settings file can be written).
    fn remember(&self, path: &Path) {
        let p = path_string(path);
        let settings = &self.0.deps.settings;
        let _ = settings.update(SettingsPatch {
            vault_path: Some(Some(p.clone())),
            ..SettingsPatch::default()
        });
        settings.remember_vault(&p);
    }

    /// Creates a vault named `name` with the default (slow, Argon2id 64 MiB) KDF.
    pub fn create_vault(
        &self,
        path: &Path,
        password: SecretString,
        name: &str,
    ) -> Result<(), SessionError> {
        self.check_not_quitting()?;
        self.create_vault_with(path, password, Database::new(name))
    }

    /// Writes `db` to `path` (refusing an existing file with `AlreadyExists`), reopens it from
    /// disk and makes it the unlocked session.
    pub fn create_vault_with(
        &self,
        path: &Path,
        password: SecretString,
        db: Database,
    ) -> Result<(), SessionError> {
        self.check_not_quitting()?;
        // Best-effort check; the window to a concurrent creator is the time of one KDF run.
        if std::fs::symlink_metadata(path).is_ok() {
            return Err(SessionError::AlreadyExists);
        }
        let bytes = db.save(&password)?;
        self.0.io.write_atomic(path, &bytes)?;
        let (file, db) = VaultFile::open_with(self.0.io.clone(), path, &password)?;
        self.finish_open(file, db, password)
    }

    /// Opens an existing vault file and makes it the unlocked session; also records it as the
    /// current and most recent vault in the settings.
    pub fn open_vault(&self, path: &Path, password: SecretString) -> Result<(), SessionError> {
        self.check_not_quitting()?;
        {
            let inner = lock(&self.0.inner);
            let same = std::fs::canonicalize(path).ok() == inner.path;
            if same && inner.pending_bytes.is_some() {
                drop(inner);
                // Reopening the locked vault that holds unsaved changes is an unlock.
                self.unlock(password)?;
                if let Some(p) = self.vault_path() {
                    self.remember(&p);
                }
                return Ok(());
            }
        }
        // The KDF runs without the session lock, so status() stays responsive meanwhile.
        let (file, db) =
            VaultFile::open_with(self.0.io.clone(), path, &password).map_err(open_error)?;
        self.finish_open(file, db, password)
    }

    /// Unlocks the locked vault with `password`. Unlocking an unlocked session only verifies the
    /// password.
    pub fn unlock(&self, password: SecretString) -> Result<(), SessionError> {
        let path = {
            let inner = lock(&self.0.inner);
            if inner.quitting {
                return Err(SessionError::Quitting);
            }
            if inner.db.is_some() {
                drop(inner);
                return self.verify_password(&password);
            }
            inner.path.clone().ok_or(SessionError::NoVault)?
        };
        let (file, disk_db) =
            VaultFile::open_with(self.0.io.clone(), &path, &password).map_err(open_error)?;
        let mut out = Outbox::default();
        {
            let mut inner = lock(&self.0.inner);
            if inner.quitting {
                return Err(SessionError::Quitting);
            }
            if inner.db.is_some() || inner.path.as_deref() != Some(path.as_path()) {
                // Another call unlocked or switched vaults meanwhile; that state wins.
                drop(inner);
                return self.verify_password(&password);
            }
            let restored = match &inner.pending_bytes {
                Some(bytes) => Some(Database::open(bytes, &password)?),
                None => None,
            };
            match restored {
                Some(db) => {
                    inner.install(file, db, password);
                    inner.dirty_gen = 1;
                    out.save = inner.set_save(SaveStatus::pending());
                }
                None => inner.install(file, disk_db, password),
            }
            out.state_changed = true;
            out.vault_changed = true;
        }
        self.emit(out);
        Ok(())
    }

    /// Saves pending edits, then drops the database and the password (zeroized) and keeps the
    /// path. If the save fails, the encrypted bytes are kept and restored on the next unlock.
    /// Works while quitting.
    pub fn lock(&self, reason: LockReason) {
        let mut out = Outbox::default();
        {
            let mut inner = lock(&self.0.inner);
            if inner.db.is_none() {
                return;
            }
            let (flushed, save) = inner.flush();
            out.save = save;
            if flushed.is_err() {
                if let (Some(db), Some(password)) = (&inner.db, &inner.password) {
                    // Only fails when the OS has no randomness left; nothing more can be kept.
                    inner.pending_bytes = db.save(password).ok();
                }
            }
            inner.db = None;
            inner.password = None;
            inner.file = None;
            inner.lock_reason = Some(reason);
            out.state_changed = true;
        }
        self.emit(out);
    }

    /// Writes unsaved edits now (synchronous). `Ok` only when everything is on disk.
    /// Works while quitting.
    pub fn flush(&self) -> Result<(), SessionError> {
        let (result, save) = lock(&self.0.inner).flush();
        self.emit(Outbox {
            save,
            ..Outbox::default()
        });
        result
    }

    /// Read access to the unlocked database. Works while quitting.
    pub fn with_db<R>(&self, f: impl FnOnce(&Database) -> R) -> Result<R, SessionError> {
        let inner = lock(&self.0.inner);
        match &inner.db {
            Some(db) => Ok(f(db)),
            None => Err(inner.not_unlocked()),
        }
    }

    /// Edits a clone of the database; `Ok` swaps it in and marks the session dirty, `Err` leaves
    /// the live database untouched. `now` comes from the injected clock.
    pub fn mutate<R>(
        &self,
        f: impl FnOnce(&mut Database, Timestamp) -> Result<R, SessionError>,
    ) -> Result<R, SessionError> {
        let now = self.now();
        let mut out = Outbox::default();
        let result = {
            let mut inner = lock(&self.0.inner);
            if inner.quitting {
                return Err(SessionError::Quitting);
            }
            let Some(db) = &inner.db else {
                return Err(inner.not_unlocked());
            };
            let mut draft = db.clone();
            let r = f(&mut draft, now)?;
            inner.db = Some(draft);
            inner.dirty_gen = inner.dirty_gen.wrapping_add(1);
            out.save = inner.set_save(SaveStatus::pending());
            out.vault_changed = true;
            r
        };
        self.emit(out);
        Ok(result)
    }

    /// An in-memory change that does NOT mark the session dirty (access times).
    pub fn touch<R>(
        &self,
        f: impl FnOnce(&mut Database, Timestamp) -> R,
    ) -> Result<R, SessionError> {
        let now = self.now();
        let mut inner = lock(&self.0.inner);
        if inner.quitting {
            return Err(SessionError::Quitting);
        }
        match &mut inner.db {
            Some(db) => Ok(f(db, now)),
            None => Err(inner.not_unlocked()),
        }
    }

    /// Compares `candidate` with the session password in constant time; never touches the file.
    pub fn verify_password(&self, candidate: &SecretString) -> Result<(), SessionError> {
        let inner = lock(&self.0.inner);
        match (&inner.db, &inner.password) {
            (Some(_), Some(p)) => {
                if constant_time_eq(
                    p.expose_secret().as_bytes(),
                    candidate.expose_secret().as_bytes(),
                ) {
                    Ok(())
                } else {
                    Err(SessionError::WrongPassword)
                }
            }
            _ => Err(inner.not_unlocked()),
        }
    }

    /// Read access to the database and the password, for callers that must re-encrypt or compare
    /// (backup restore, device-unlock enrolment).
    pub fn with_db_and_password<R>(
        &self,
        f: impl FnOnce(&Database, &SecretString) -> R,
    ) -> Result<R, SessionError> {
        let inner = lock(&self.0.inner);
        match (&inner.db, &inner.password) {
            (Some(db), Some(p)) => Ok(f(db, p)),
            _ => Err(inner.not_unlocked()),
        }
    }

    /// Flushes, writes the current database to `new_path` through `io` (`AlreadyExists` when the
    /// target exists), reopens the session there and records the new path in the settings. The
    /// old file is left untouched. Refused while quitting.
    pub fn relocate(&self, new_path: &Path, io: Arc<dyn FileIo>) -> Result<(), SessionError> {
        let mut out = Outbox::default();
        let result = self.relocate_locked(new_path, io, &mut out);
        if result.is_ok() {
            if let Some(p) = self.vault_path() {
                self.remember(&p);
            }
            out.state_changed = true;
        }
        self.emit(out);
        result
    }

    fn relocate_locked(
        &self,
        new_path: &Path,
        io: Arc<dyn FileIo>,
        out: &mut Outbox,
    ) -> Result<(), SessionError> {
        let mut inner = lock(&self.0.inner);
        if inner.quitting {
            return Err(SessionError::Quitting);
        }
        if inner.db.is_none() {
            return Err(inner.not_unlocked());
        }
        let (flushed, save) = inner.flush();
        out.save = save;
        flushed?;
        if std::fs::symlink_metadata(new_path).is_ok() {
            return Err(SessionError::AlreadyExists);
        }
        let (Some(db), Some(password)) = (&inner.db, &inner.password) else {
            return Err(SessionError::Locked);
        };
        let bytes = db.save(password)?;
        io.write_atomic(new_path, &bytes)?;
        let previous = self.0.io.get();
        self.0.io.set(io);
        match VaultFile::open_with(self.0.io.clone(), new_path, password) {
            Ok((file, _)) => {
                inner.path = Some(file.path().to_path_buf());
                inner.file = Some(file);
                Ok(())
            }
            Err(e) => {
                self.0.io.set(previous);
                Err(e.into())
            }
        }
    }

    /// Swaps the IO seam for the next open or save.
    pub fn set_file_io(&self, io: Arc<dyn FileIo>) {
        self.0.io.set(io);
    }

    /// Replaces the hook called when a save or an external-change check merged or reloaded
    /// remote content.
    pub fn set_conflict_hook(&self, hook: ConflictHook) {
        lock(&self.0.inner).conflict_hook = hook;
    }

    /// Starts the quit gate: from now on `create_vault*`, `open_vault`, `unlock`, `mutate`,
    /// `touch` and `relocate` fail with `Quitting`; `lock`, `flush`, `status` and `with_db` keep
    /// working. Idempotent.
    pub fn begin_quit(&self) {
        lock(&self.0.inner).quitting = true;
    }

    /// Re-opens the session after a cancelled quit.
    pub fn cancel_quit(&self) {
        lock(&self.0.inner).quitting = false;
    }

    pub fn is_quitting(&self) -> bool {
        lock(&self.0.inner).quitting
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::ManualScheduler;
    use crate::settings::SettingsStore;
    use crate::testing::fast_database;
    use mypass_kdbx::test_util::FailingFileIo;
    use mypass_kdbx::{Entry, KdbxError, PlainFileIo};
    use std::fs;
    use std::io::ErrorKind;
    use std::sync::Mutex;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    struct Fx {
        dir: TempDir,
        session: VaultSession,
        settings: SettingsStore,
        scheduler: Arc<ManualScheduler>,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let settings = SettingsStore::in_memory();
        let scheduler = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let session = VaultSession::new(SessionDeps {
            settings: settings.clone(),
            scheduler: scheduler.clone(),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        Fx {
            dir,
            session,
            settings,
            scheduler,
        }
    }

    impl Fx {
        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }
        /// Creates `v.kdbx` with password "pw" and one entry; returns the canonical path.
        fn created(&self) -> PathBuf {
            let path = self.path("v.kdbx");
            let mut db = fast_database("Test Vault");
            let now = Timestamp::now();
            let mut e = Entry::new(now);
            e.set_field("Title", "first", false);
            let root = db.root_id();
            db.add_entry(root, e, now).unwrap();
            self.session.create_vault_with(&path, pw("pw"), db).unwrap();
            fs::canonicalize(path).unwrap()
        }
        fn entries(&self) -> usize {
            self.session.with_db(|db| db.root.entries.len()).unwrap()
        }
        fn add_entry(&self) -> Result<(), SessionError> {
            self.session.mutate(|db, now| {
                let root = db.root_id();
                db.add_entry(root, Entry::new(now), now)?;
                Ok(())
            })
        }
    }

    #[test]
    fn new_session_is_no_vault() {
        let f = fx();
        let s = f.session.status();
        assert_eq!(s.state, SessionState::NoVault);
        assert_eq!(s.vault_path, None);
        assert_eq!(s.vault_name, None);
        assert_eq!(s.lock_reason, None);
        assert_eq!(s.file_status, VaultFileStatus::NoVault);
        assert_eq!(s.save.state, SaveState::Idle);
        assert!(!f.session.is_unlocked());
        assert_eq!(f.session.vault_id(), None);
    }

    #[test]
    fn create_then_status_unlocked_with_name_and_path() {
        let f = fx();
        let path = f.created();
        let s = f.session.status();
        assert_eq!(s.state, SessionState::Unlocked);
        assert_eq!(s.vault_name.as_deref(), Some("Test Vault"));
        assert_eq!(s.vault_path.as_deref(), path.to_str());
        assert!(matches!(s.file_status, VaultFileStatus::Connected { .. }));
        assert_eq!(f.session.vault_path(), Some(path));
        assert_eq!(f.entries(), 1);
    }

    #[test]
    fn create_vault_uses_the_given_name() {
        let f = fx();
        let path = f.path("named.kdbx");
        f.session.create_vault(&path, pw("pw"), "Named").unwrap();
        assert_eq!(f.session.status().vault_name.as_deref(), Some("Named"));
    }

    #[test]
    fn create_refuses_existing_file_without_touching_it() {
        let f = fx();
        let path = f.path("v.kdbx");
        fs::write(&path, b"existing bytes").unwrap();
        let r = f
            .session
            .create_vault_with(&path, pw("pw"), fast_database("x"));
        assert_eq!(r, Err(SessionError::AlreadyExists));
        assert_eq!(fs::read(&path).unwrap(), b"existing bytes");
        assert_eq!(f.session.status().state, SessionState::NoVault);
    }

    #[test]
    fn open_wrong_password_stays_locked_with_wrong_password_error() {
        let f = fx();
        f.created();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.session.unlock(pw("bad")),
            Err(SessionError::WrongPassword)
        );
        assert_eq!(f.session.status().state, SessionState::Locked);
        let other = fx();
        let r = other.session.open_vault(&f.path("v.kdbx"), pw("bad"));
        assert_eq!(r, Err(SessionError::WrongPassword));
        assert_eq!(other.session.status().state, SessionState::NoVault);
    }

    #[test]
    fn open_nonexistent_is_not_found() {
        let f = fx();
        let r = f.session.open_vault(&f.path("missing.kdbx"), pw("pw"));
        assert_eq!(r, Err(SessionError::NotFound));
    }

    #[test]
    fn open_vault_opens_an_existing_file() {
        let f = fx();
        let path = f.created();
        let g = fx();
        g.session.open_vault(&path, pw("pw")).unwrap();
        assert_eq!(g.session.status().state, SessionState::Unlocked);
        assert_eq!(g.entries(), 1);
    }

    #[test]
    fn lock_drops_database_and_with_db_fails_locked() {
        let f = fx();
        let path = f.created();
        f.session.lock(LockReason::Manual);
        assert_eq!(f.session.with_db(|_| ()), Err(SessionError::Locked));
        assert_eq!(f.session.mutate(|_, _| Ok(())), Err(SessionError::Locked));
        assert_eq!(f.session.touch(|_, _| ()), Err(SessionError::Locked));
        assert!(!f.session.is_unlocked());
        assert_eq!(f.session.vault_path(), Some(path));
        assert_eq!(f.session.status().state, SessionState::Locked);
    }

    #[test]
    fn unlock_after_lock_restores_same_content() {
        let f = fx();
        f.created();
        let before = f.session.with_db(|db| db.clone()).unwrap();
        f.session.lock(LockReason::Manual);
        f.session.unlock(pw("pw")).unwrap();
        let after = f.session.with_db(|db| db.clone()).unwrap();
        assert_eq!(before, after);
        assert_eq!(f.session.status().lock_reason, None);
    }

    #[test]
    fn lock_records_reason() {
        let f = fx();
        f.created();
        f.session.lock(LockReason::Idle);
        assert_eq!(f.session.status().lock_reason, Some(LockReason::Idle));
    }

    #[test]
    fn lock_saves_dirty_changes_before_dropping_the_database() {
        let f = fx();
        f.created();
        f.add_entry().unwrap();
        f.session.lock(LockReason::Manual);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.entries(), 2);
    }

    #[test]
    fn failed_save_on_lock_keeps_the_edit_for_the_next_unlock() {
        let f = fx();
        f.created();
        let failing = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.session.set_file_io(failing.clone());
        f.add_entry().unwrap();
        failing.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        assert_eq!(f.session.status().save.state, SaveState::Failed);
        failing.heal();
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.entries(), 2);
        assert_eq!(f.session.status().save.state, SaveState::Pending);
        f.session.flush().unwrap();
        f.session.lock(LockReason::Manual);
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.entries(), 2);
    }

    #[test]
    fn with_db_on_no_vault_is_no_vault() {
        let f = fx();
        assert_eq!(f.session.with_db(|_| ()), Err(SessionError::NoVault));
        assert_eq!(f.session.unlock(pw("pw")), Err(SessionError::NoVault));
    }

    #[test]
    fn failed_mutation_leaves_database_untouched() {
        let f = fx();
        f.created();
        let r: Result<(), _> = f.session.mutate(|db, now| {
            let root = db.root_id();
            db.add_entry(root, Entry::new(now), now)?;
            db.root.entries.clear();
            Err(SessionError::InvalidInput("nope".into()))
        });
        assert_eq!(r, Err(SessionError::InvalidInput("nope".into())));
        assert_eq!(f.entries(), 1);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
    }

    #[test]
    fn successful_mutation_marks_dirty_and_uses_the_injected_clock() {
        let f = fx();
        f.created();
        let stamp = f.session.mutate(|_, now| Ok(now)).unwrap();
        assert_eq!(stamp, Timestamp(1_700_000_000));
        assert_eq!(f.session.status().save.state, SaveState::Pending);
    }

    #[test]
    fn touch_does_not_mark_dirty() {
        let f = fx();
        f.created();
        let n = f
            .session
            .touch(|db, _| {
                db.meta.database_name = "touched".into();
                1
            })
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        assert_eq!(
            f.session
                .with_db(|db| db.meta.database_name.clone())
                .unwrap(),
            "touched"
        );
    }

    fn record(session: &VaultSession) -> Arc<Mutex<Vec<SessionEvent>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        session.add_observer(Arc::new(move |e: &SessionEvent| {
            l.lock().unwrap().push(e.clone())
        }));
        log
    }

    #[test]
    fn observer_receives_state_changes_in_order() {
        let f = fx();
        let log = record(&f.session);
        f.created();
        f.session.lock(LockReason::Manual);
        let states: Vec<_> = log
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::StateChanged(s) => Some(s.state),
                _ => None,
            })
            .collect();
        assert_eq!(states, vec![SessionState::Unlocked, SessionState::Locked]);
    }

    #[test]
    fn mutation_emits_vault_and_save_events() {
        let f = fx();
        f.created();
        let log = record(&f.session);
        f.add_entry().unwrap();
        let events = log.lock().unwrap().clone();
        assert!(events.contains(&SessionEvent::VaultChanged));
        assert!(events.iter().any(|e| matches!(
            e,
            SessionEvent::SaveChanged(SaveStatus {
                state: SaveState::Pending,
                ..
            })
        )));
    }

    #[test]
    fn observer_may_call_status_reentrantly() {
        let f = fx();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (s, seen2) = (f.session.clone(), seen.clone());
        f.session.add_observer(Arc::new(move |_: &SessionEvent| {
            seen2.lock().unwrap().push(s.status().state);
            let _ = s.with_db(|_| ());
        }));
        f.created();
        f.session.lock(LockReason::Manual);
        assert!(!seen.lock().unwrap().is_empty());
    }

    #[test]
    fn open_vault_remembers_recent_and_vault_path() {
        let f = fx();
        let path = f.created();
        let g = fx();
        g.session.open_vault(&path, pw("pw")).unwrap();
        let s = g.settings.get();
        let p = path.to_str().unwrap();
        assert_eq!(s.vault_path.as_deref(), Some(p));
        assert_eq!(s.recent_vaults, vec![p.to_owned()]);
        // Creating also records the vault.
        assert_eq!(f.settings.get().vault_path.as_deref(), Some(p));
    }

    #[test]
    fn vault_id_is_root_uuid_simple_hex_32_chars() {
        let f = fx();
        f.created();
        let root = f.session.with_db(|db| db.root_id()).unwrap();
        let id = f.session.vault_id().unwrap();
        assert_eq!(id, root.simple().to_string());
        assert_eq!(id.len(), 32);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        f.session.lock(LockReason::Manual);
        assert_eq!(f.session.vault_id(), Some(id));
    }

    #[test]
    fn kdbx_error_mapping_table() {
        let io = KdbxError::Io {
            kind: ErrorKind::PermissionDenied,
            message: "denied".into(),
        };
        let table = [
            (KdbxError::WrongPassword, SessionError::WrongPassword),
            (
                KdbxError::Corrupt("c".into()),
                SessionError::Corrupt("c".into()),
            ),
            (
                KdbxError::NotKdbx,
                SessionError::Corrupt("not a KDBX file".into()),
            ),
            (
                KdbxError::UnsupportedVersion { major: 3, minor: 1 },
                SessionError::Unsupported("unsupported KDBX version 3.1".into()),
            ),
            (
                KdbxError::Unsupported("u".into()),
                SessionError::Unsupported("u".into()),
            ),
            (io, SessionError::Io("denied".into())),
            (
                KdbxError::InvalidOperation("op".into()),
                SessionError::InvalidInput("op".into()),
            ),
            (
                KdbxError::InvalidKdfParameters("k".into()),
                SessionError::Corrupt("k".into()),
            ),
            (
                KdbxError::Random,
                SessionError::Io("OS randomness unavailable".into()),
            ),
        ];
        for (from, want) in table {
            assert_eq!(SessionError::from(from.clone()), want, "{from:?}");
        }
    }

    #[test]
    fn verify_password_accepts_current_and_rejects_other_without_io() {
        let f = fx();
        f.created();
        let counting = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.session.set_file_io(counting.clone());
        assert_eq!(f.session.verify_password(&pw("pw")), Ok(()));
        assert_eq!(
            f.session.verify_password(&pw("other")),
            Err(SessionError::WrongPassword)
        );
        assert_eq!(
            f.session.verify_password(&pw("pw2")),
            Err(SessionError::WrongPassword)
        );
        assert_eq!(
            f.session.verify_password(&pw("")),
            Err(SessionError::WrongPassword)
        );
        assert_eq!(
            (
                counting.read_calls(),
                counting.write_calls(),
                counting.metadata_calls()
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn verify_password_while_locked_is_locked() {
        let f = fx();
        assert_eq!(
            f.session.verify_password(&pw("pw")),
            Err(SessionError::NoVault)
        );
        f.created();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.session.verify_password(&pw("pw")),
            Err(SessionError::Locked)
        );
    }

    #[test]
    fn with_db_and_password_exposes_both() {
        use secrecy::ExposeSecret;
        let f = fx();
        f.created();
        let (name, secret) = f
            .session
            .with_db_and_password(|db, p| {
                (db.meta.database_name.clone(), p.expose_secret().to_owned())
            })
            .unwrap();
        assert_eq!((name.as_str(), secret.as_str()), ("Test Vault", "pw"));
    }

    #[test]
    fn relocate_moves_session_to_new_file_and_leaves_old_untouched() {
        let f = fx();
        let old = f.created();
        let old_bytes = fs::read(&old).unwrap();
        let target = f.path("moved.kdbx");
        f.session.relocate(&target, Arc::new(PlainFileIo)).unwrap();
        let target = fs::canonicalize(&target).unwrap();
        assert_eq!(f.session.vault_path(), Some(target.clone()));
        f.add_entry().unwrap();
        f.session.lock(LockReason::Manual);
        assert_eq!(fs::read(&old).unwrap(), old_bytes);
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.entries(), 2);
        let check = fx();
        check.session.open_vault(&old, pw("pw")).unwrap();
        assert_eq!(check.entries(), 1);
    }

    #[test]
    fn relocate_refuses_existing_target_with_already_exists() {
        let f = fx();
        let old = f.created();
        let target = f.path("taken.kdbx");
        fs::write(&target, b"taken").unwrap();
        assert_eq!(
            f.session.relocate(&target, Arc::new(PlainFileIo)),
            Err(SessionError::AlreadyExists)
        );
        assert_eq!(fs::read(&target).unwrap(), b"taken");
        assert_eq!(f.session.vault_path(), Some(old));
    }

    #[test]
    fn relocate_updates_vault_path_and_recents() {
        let f = fx();
        let old = f.created();
        let target = f.path("moved.kdbx");
        f.session.relocate(&target, Arc::new(PlainFileIo)).unwrap();
        let target = fs::canonicalize(&target).unwrap();
        let s = f.settings.get();
        assert_eq!(s.vault_path.as_deref(), target.to_str());
        assert_eq!(
            s.recent_vaults,
            vec![
                target.to_str().unwrap().to_owned(),
                old.to_str().unwrap().to_owned()
            ]
        );
    }

    #[test]
    fn relocate_writes_through_the_given_io() {
        let f = fx();
        f.created();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.session
            .relocate(&f.path("moved.kdbx"), io.clone())
            .unwrap();
        assert_eq!(io.write_calls(), 1);
        f.add_entry().unwrap();
        f.session.flush().unwrap();
        assert_eq!(io.write_calls(), 2);
    }

    #[test]
    fn set_file_io_is_used_by_the_next_save() {
        let f = fx();
        f.created();
        let counting = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.session.set_file_io(counting.clone());
        f.add_entry().unwrap();
        assert_eq!(counting.write_calls(), 0);
        f.session.flush().unwrap();
        assert_eq!(counting.write_calls(), 1);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
    }

    #[test]
    fn failed_flush_reports_and_keeps_the_session_dirty() {
        let f = fx();
        f.created();
        let failing = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.session.set_file_io(failing.clone());
        f.add_entry().unwrap();
        failing.fail_write_atomic(ErrorKind::PermissionDenied);
        assert!(matches!(f.session.flush(), Err(SessionError::Io(_))));
        let s = f.session.status().save;
        assert_eq!(s.state, SaveState::Failed);
        assert!(s.message.is_some());
        failing.heal();
        f.session.flush().unwrap();
        assert_eq!(f.session.status().save, SaveStatus::idle());
    }

    #[test]
    fn flush_refuses_to_overwrite_an_external_change() {
        let f = fx();
        let path = f.created();
        f.add_entry().unwrap();
        let other = fast_database("other").save(&pw("pw")).unwrap();
        fs::write(&path, &other).unwrap();
        assert_eq!(f.session.flush(), Err(SessionError::ChangedOnDisk));
        assert_eq!(fs::read(&path).unwrap(), other);
    }

    #[test]
    fn mutate_after_begin_quit_is_rejected_with_quitting() {
        let f = fx();
        f.created();
        f.session.begin_quit();
        assert_eq!(f.add_entry(), Err(SessionError::Quitting));
        assert_eq!(f.session.touch(|_, _| ()), Err(SessionError::Quitting));
        assert_eq!(f.entries(), 1);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        assert_eq!(f.scheduler.pending(), 0);
    }

    #[test]
    fn open_unlock_create_after_begin_quit_are_rejected_with_quitting() {
        let f = fx();
        let path = f.created();
        f.session.lock(LockReason::Manual);
        f.session.begin_quit();
        assert_eq!(f.session.unlock(pw("pw")), Err(SessionError::Quitting));
        assert_eq!(
            f.session.open_vault(&path, pw("pw")),
            Err(SessionError::Quitting)
        );
        assert_eq!(
            f.session
                .create_vault_with(&f.path("n.kdbx"), pw("pw"), fast_database("n")),
            Err(SessionError::Quitting)
        );
        assert_eq!(
            f.session.create_vault(&f.path("n.kdbx"), pw("pw"), "n"),
            Err(SessionError::Quitting)
        );
        assert!(!f.path("n.kdbx").exists());
    }

    #[test]
    fn lock_and_flush_still_work_while_quitting() {
        let f = fx();
        f.created();
        f.add_entry().unwrap();
        f.session.begin_quit();
        assert_eq!(f.entries(), 2);
        f.session.flush().unwrap();
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        f.session.lock(LockReason::Quit);
        let s = f.session.status();
        assert_eq!(s.state, SessionState::Locked);
        assert_eq!(s.lock_reason, Some(LockReason::Quit));
        f.session.cancel_quit();
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.entries(), 2);
    }

    #[test]
    fn cancel_quit_reopens_the_session() {
        let f = fx();
        f.created();
        f.session.begin_quit();
        f.session.cancel_quit();
        assert!(!f.session.is_quitting());
        f.add_entry().unwrap();
        assert_eq!(f.entries(), 2);
    }

    #[test]
    fn begin_quit_is_idempotent() {
        let f = fx();
        f.session.begin_quit();
        f.session.begin_quit();
        assert!(f.session.is_quitting());
        f.session.cancel_quit();
        assert!(!f.session.is_quitting());
    }

    #[test]
    fn debug_output_contains_no_secret() {
        let f = fx();
        f.created();
        f.add_entry().unwrap();
        let text = format!("{:?} {:?}", f.session, f.session.status());
        assert!(!text.contains("pw\""), "{text}");
        assert!(!text.contains("first"), "{text}");
    }

    #[test]
    fn the_password_is_held_only_as_a_secret_string() {
        // Type-level substitute for a freed-memory test (no `unsafe` in this crate): the session
        // stores the master password as `SecretString`, which zeroizes on drop.
        fn is_secret(_: &Option<SecretString>) {}
        let inner = Inner::new();
        is_secret(&inner.password);
    }
}
