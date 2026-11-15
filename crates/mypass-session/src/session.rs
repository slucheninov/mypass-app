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
//! Saving (debounced autosave, `flush`, `lock`, the external-change merge, the kept encrypted
//! edits after a failed save on lock and their recovery) lives in the `saving` module; its module
//! docs describe the guarantees and the ordering between a background write and a lock-time
//! flush.
//!
//! Locking discipline: observers and the conflict hook are always called after the session lock
//! is released, so they may call back into the session. Closures passed to `with_db`, `mutate`,
//! `touch` and `with_db_and_password` run UNDER the lock and must not call the session. Lock
//! order: the session lock first, then the settings lock (`SettingsStore` never calls the
//! session), then a scheduler's internal lock (schedulers never run a job while holding it).
//! No other component (auto-lock, clipboard service) holds its own lock while calling the
//! session.

use crate::backups::BackupManager;
use crate::scheduler::{lock, Scheduler, TaskHandle};
use crate::settings::{SettingsPatch, SettingsStore};
use crate::storage::{
    ConflictResolver, IoResolver, ResolvedStorage, StorageObserver, StorageStatus,
    REMOTE_CHANGE_DEBOUNCE,
};
use crate::vault_files::{file_status, VaultFileStatus};
use mypass_kdbx::{Database, FileIo, FileMeta, KdbxError, Meta, Timestamp, VaultFile};
use mypass_sync::icloud::merge_vault_bytes;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};
use std::time::UNIX_EPOCH;
use subtle::ConstantTimeEq;

/// Lifecycle state shown by the UI.
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    NoVault,
    Locked,
    Unlocked,
}

/// Why the session was locked.
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SaveState {
    Idle,
    Pending,
    Saving,
    Failed,
}

/// Save state plus the last failure message (no secret content, file-system wording only).
#[derive(Serialize, specta::Type, Clone, PartialEq, Eq, Debug)]
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
    pub(crate) fn pending() -> Self {
        Self {
            state: SaveState::Pending,
            message: None,
        }
    }
    pub(crate) fn saving() -> Self {
        Self {
            state: SaveState::Saving,
            message: None,
        }
    }
    pub(crate) fn failed(message: String) -> Self {
        Self {
            state: SaveState::Failed,
            message: Some(message),
        }
    }
}

/// Snapshot of the session for the UI (the `SessionChanged` event payload and `vault_status`).
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
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

/// Told the vault id after the user's typed master password opened, created or unlocked the
/// vault (also a reopen of the vault already open, which emits no state change). Never told about
/// an unlock with a password that came from somewhere else (device unlock). Runs after the
/// session lock is released, on the calling thread.
pub type PasswordListener = Arc<dyn Fn(&str) + Send + Sync>;

/// What [`VaultSession::unlock_if_incarnation`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConditionalUnlock {
    /// The locked vault of that incarnation is now unlocked.
    Unlocked,
    /// The session no longer holds that locked vault (it was unlocked, locked again, switched or
    /// relocated meanwhile); nothing was installed.
    Superseded,
}

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
    /// The vault is in iCloud Drive and did not download in time (R17); try again later.
    #[error("the vault is still downloading from iCloud")]
    Downloading,
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
            // A read that waited for iCloud (download or file coordination) too long.
            KdbxError::Io {
                kind: ErrorKind::TimedOut,
                ..
            } => Self::Downloading,
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
pub(crate) struct SwitchIo(RwLock<Arc<dyn FileIo>>);

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

/// The name shown for a vault: its database name, or the file stem when that is empty.
pub(crate) fn vault_name_for(db: &Database, path: &Path) -> Option<String> {
    Some(db.meta.database_name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
}

/// A copy of the unlocked vault taken by [`VaultSession::sync_snapshot`]. The password is a
/// `SecretString` (zeroized on drop) and the `Debug` output redacts everything.
pub(crate) struct SyncSnapshot {
    pub(crate) db: Database,
    pub(crate) password: SecretString,
    pub(crate) path: PathBuf,
    pub(crate) vault_id: String,
    pub(crate) incarnation: u64,
}

impl std::fmt::Debug for SyncSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncSnapshot")
            .field("incarnation", &self.incarnation)
            .finish_non_exhaustive()
    }
}

/// What [`VaultSession::durably_holds`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Durability {
    /// On disk.
    Durable,
    /// Unsaved edits: flush and ask again.
    Unsaved,
    /// The live vault lacks part of the content (it was replaced by an external reload).
    Missing,
    /// The vault was locked, switched or relocated.
    Stale,
}

pub(crate) struct Inner {
    pub(crate) path: Option<PathBuf>,
    pub(crate) vault_name: Option<String>,
    pub(crate) vault_id: Option<String>,
    /// Taken out (`None`) while a write or an external-change check runs outside the lock.
    pub(crate) file: Option<VaultFile>,
    pub(crate) db: Option<Database>,
    /// The master password, only while unlocked; `SecretString` zeroizes on drop.
    pub(crate) password: Option<SecretString>,
    pub(crate) dirty_gen: u64,
    pub(crate) saved_gen: u64,
    /// The encrypted database kept when the save on lock failed; written by the retry timer or
    /// merged on the next unlock.
    pub(crate) pending_bytes: Option<Vec<u8>>,
    /// The `VaultFile` that went with `pending_bytes`: its stamp is the disk content as the
    /// session last knew it, so the retry never writes over an external change.
    pub(crate) pending_file: Option<VaultFile>,
    pub(crate) save: SaveStatus,
    pub(crate) lock_reason: Option<LockReason>,
    pub(crate) quitting: bool,
    /// Called by the save and external-check paths.
    pub(crate) conflict_hook: ConflictHook,
    /// The scheduled autosave (debounce) or retry job, cancelled when replaced.
    pub(crate) save_job: Option<TaskHandle>,
    /// A write (or a check) runs outside the lock; `Shared::idle` is signalled when it ends.
    pub(crate) in_flight: bool,
    /// Publications in progress ([`VaultSession::publish_if_current`]: a sync checkpoint being
    /// written for this incarnation). Every incarnation bump (lock, open, unlock, relocation),
    /// `begin_quit` and the secondary sync's arm changes wait for it to drop to zero, so nothing
    /// is published for a vault that was locked, switched or relocated, or for an arm that was
    /// replaced, once that change has happened. `Shared::idle` is signalled when it drops.
    pub(crate) publishing: u32,
    /// The next save is the first since this vault was opened or unlocked (always snapshots).
    pub(crate) first_write_of_session: bool,
    /// The file was created by this session; its first save takes no snapshot (there is
    /// nothing of the user's on disk yet).
    pub(crate) fresh_file: bool,
    /// Bumped by every scheduled save job; a job clears `save_job` when it starts only if it is
    /// still the current one, so `save_job` is `Some` exactly while a save job is armed.
    pub(crate) save_job_id: u64,
    /// A snapshot of the current disk state was already taken (or attempted): retries of a
    /// failed write take none. Cleared by a successful write and by seeing the disk change.
    pub(crate) snapshot_taken: bool,
    /// The metadata last read from or written to disk: the base that tells a local metadata
    /// edit from an untouched one when remote content is merged (see the saving module docs).
    /// Kept while locked, for the merge of kept edits on unlock.
    pub(crate) disk_meta: Option<Meta>,
    /// Bumped whenever the open vault stops being the one a background task took a snapshot of:
    /// every install (open, create, unlock, switch), lock and relocation. A task that snapshots
    /// the session, works outside the lock and comes back (the secondary sync) acts only while
    /// this is unchanged.
    pub(crate) incarnation: u64,
    /// The storage the open vault was resolved to (kept while locked, so an unlock reads through
    /// the same `FileIo`, cache and presenter).
    pub(crate) storage: Option<ResolvedStorage>,
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
            pending_file: None,
            save: SaveStatus::idle(),
            lock_reason: None,
            quitting: false,
            conflict_hook: Arc::new(|_| {}),
            save_job: None,
            in_flight: false,
            publishing: 0,
            first_write_of_session: false,
            fresh_file: false,
            save_job_id: 0,
            snapshot_taken: false,
            disk_meta: None,
            incarnation: 0,
            storage: None,
        }
    }

    pub(crate) fn state(&self) -> SessionState {
        match (&self.db, &self.path) {
            (Some(_), _) => SessionState::Unlocked,
            (None, Some(_)) => SessionState::Locked,
            (None, None) => SessionState::NoVault,
        }
    }

    pub(crate) fn not_unlocked(&self) -> SessionError {
        if self.path.is_some() {
            SessionError::Locked
        } else {
            SessionError::NoVault
        }
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty_gen != self.saved_gen
    }

    /// Sets the save status; returns it when it changed (to notify after unlocking).
    pub(crate) fn set_save(&mut self, s: SaveStatus) -> Option<SaveStatus> {
        (self.save != s).then(|| {
            self.save = s.clone();
            s
        })
    }

    /// Makes `file` + `db` the open vault. The caller has flushed the previous one.
    pub(crate) fn install(&mut self, file: VaultFile, db: Database, password: SecretString) {
        let path = file.path().to_path_buf();
        self.vault_name = vault_name_for(&db, &path);
        self.disk_meta = Some(db.meta.clone());
        self.snapshot_taken = false;
        self.vault_id = Some(db.root_id().simple().to_string());
        self.path = Some(path);
        self.file = Some(file);
        self.db = Some(db);
        self.password = Some(password);
        self.lock_reason = None;
        self.dirty_gen = 0;
        self.saved_gen = 0;
        self.pending_bytes = None;
        self.pending_file = None;
        self.save = SaveStatus::idle();
        self.cancel_save_job();
        self.first_write_of_session = true;
        self.fresh_file = false;
        self.incarnation = self.incarnation.wrapping_add(1);
    }

    pub(crate) fn cancel_save_job(&mut self) {
        if let Some(h) = self.save_job.take() {
            h.cancel();
        }
    }
}

pub(crate) struct Shared {
    pub(crate) inner: Mutex<Inner>,
    /// Signalled whenever `Inner::in_flight` drops to `false`.
    pub(crate) idle: Condvar,
    pub(crate) io: Arc<SwitchIo>,
    observers: RwLock<Vec<Observer>>,
    password_listeners: RwLock<Vec<PasswordListener>>,
    pub(crate) deps: SessionDeps,
    pub(crate) backups: BackupManager,
    /// Resolves a vault path to its storage (iCloud Drive); `None`: the session's IO seam.
    resolver: RwLock<Option<IoResolver>>,
    /// The conflict resolver for vaults whose storage has none of its own.
    conflict_resolver: RwLock<Option<ConflictResolver>>,
    storage_observers: RwLock<Vec<StorageObserver>>,
    /// The storage status last told to the observers.
    last_storage: Mutex<Option<StorageStatus>>,
    /// The debounced check after a remote-change notification. Its own small lock, never the
    /// session's: the notification arrives on a platform thread that must not block.
    remote_job: Mutex<Option<TaskHandle>>,
}

/// The vault session handle (cheap to clone; every clone is the same session).
#[derive(Clone)]
pub struct VaultSession(pub(crate) Arc<Shared>);

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
pub(crate) struct Outbox {
    pub(crate) state_changed: bool,
    pub(crate) vault_changed: bool,
    pub(crate) save: Option<SaveStatus>,
}

impl Outbox {
    /// Keeps the latest save status of two (the later change wins).
    pub(crate) fn save(&mut self, s: Option<SaveStatus>) {
        if s.is_some() {
            self.save = s;
        }
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl VaultSession {
    pub fn new(deps: SessionDeps) -> Self {
        let io = Arc::new(SwitchIo(RwLock::new(deps.file_io.clone())));
        let backups = BackupManager::new(deps.backups_root.clone(), deps.settings.clone());
        Self(Arc::new(Shared {
            inner: Mutex::new(Inner::new()),
            idle: Condvar::new(),
            io,
            observers: RwLock::new(Vec::new()),
            password_listeners: RwLock::new(Vec::new()),
            deps,
            backups,
            resolver: RwLock::new(None),
            conflict_resolver: RwLock::new(None),
            storage_observers: RwLock::new(Vec::new()),
            last_storage: Mutex::new(None),
            remote_job: Mutex::new(None),
        }))
    }

    /// The backup manager this session snapshots through (a clone shares its failure record, so
    /// the backups pane reports the failures of the session's own snapshots).
    pub fn backups(&self) -> BackupManager {
        self.0.backups.clone()
    }

    /// Registers an observer for every later event.
    pub fn add_observer(&self, o: Observer) {
        self.0
            .observers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(o);
    }

    /// Registers a [`PasswordListener`] for every later typed-password open, create or unlock.
    pub fn add_password_listener(&self, l: PasswordListener) {
        self.0
            .password_listeners
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(l);
    }

    /// Tells the password listeners that the typed password opened `vault_id`. Called after the
    /// session lock is released.
    fn password_entered(&self, vault_id: Option<String>) {
        let Some(id) = vault_id else {
            return;
        };
        let listeners = self
            .0
            .password_listeners
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for l in &listeners {
            l(&id);
        }
    }

    /// The current status (stats the vault file once, outside the session lock).
    pub fn status(&self) -> SessionStatus {
        let (state, path, vault_name, lock_reason, save, storage) = {
            let inner = lock(&self.0.inner);
            (
                inner.state(),
                inner.path.clone(),
                inner.vault_name.clone(),
                inner.lock_reason,
                inner.save.clone(),
                inner.storage.clone(),
            )
        };
        // Outside the session lock: the health of an iCloud vault looks at its cache files.
        let health = storage.as_ref().map(ResolvedStorage::health);
        let file_status = match (&storage, health) {
            (_, Some(h)) if h.downloading => VaultFileStatus::Downloading,
            // Served from the offline copy: that is the file the session works with.
            (
                Some(ResolvedStorage {
                    icloud: Some(icloud),
                    ..
                }),
                Some(h),
            ) if h.fallback_active => file_status(Some(icloud.cache_path())),
            _ => file_status(path.as_deref()),
        };
        SessionStatus {
            state,
            vault_path: path.as_deref().map(path_string),
            vault_name,
            lock_reason,
            file_status,
            save,
        }
    }

    /// Where the open vault is stored and how that storage is doing.
    pub fn storage_status(&self) -> StorageStatus {
        let storage = lock(&self.0.inner).storage.clone();
        storage
            .as_ref()
            .map(ResolvedStorage::status)
            .unwrap_or_default()
    }

    /// Adds an observer of [`VaultSession::storage_status`] changes.
    pub fn add_storage_observer(&self, o: StorageObserver) {
        self.0
            .storage_observers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(o);
    }

    /// Tells the storage observers when the storage status changed since they were last told.
    /// Called with the session lock released.
    pub(crate) fn notify_storage(&self) {
        let status = self.storage_status();
        {
            let mut last = lock(&self.0.last_storage);
            if *last == Some(status) {
                return;
            }
            *last = Some(status);
        }
        let observers = self
            .0
            .storage_observers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for o in &observers {
            o(&status);
        }
    }

    /// Installs the resolver of vault paths to their storage (production: iCloud Drive paths to
    /// `ICloudFileIo`); applies from the next open. `None`: every vault uses the session's IO
    /// seam.
    pub fn set_io_resolver(&self, resolver: Option<IoResolver>) {
        *self
            .0
            .resolver
            .write()
            .unwrap_or_else(PoisonError::into_inner) = resolver;
    }

    /// Installs the conflict resolver used for vaults whose storage has none of its own (an
    /// iCloud vault always uses its `ICloudFileIo`). Default: none.
    pub fn set_conflict_resolver(&self, resolver: Option<ConflictResolver>) {
        *self
            .0
            .conflict_resolver
            .write()
            .unwrap_or_else(PoisonError::into_inner) = resolver;
    }

    /// The storage of the vault at `path`: the resolver's answer, else the session's IO seam.
    fn resolve_storage(&self, path: &Path) -> ResolvedStorage {
        let resolver = self
            .0
            .resolver
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        resolver
            .and_then(|r| r(path))
            .unwrap_or_else(|| ResolvedStorage::plain(self.0.io.clone()))
    }

    /// The conflict resolver for `storage`: its own, else the session-wide one.
    pub(crate) fn conflict_resolver_for(
        &self,
        storage: Option<&ResolvedStorage>,
    ) -> Option<ConflictResolver> {
        storage.and_then(ResolvedStorage::conflicts).or_else(|| {
            self.0
                .conflict_resolver
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }

    /// Merges the conflict versions of the vault at `path` into the file before it is read
    /// (best effort: a failure leaves the versions for the next attempt, the read goes on). The
    /// merge step uses `password`, which the caller already holds; it is never copied out.
    pub(crate) fn merge_conflicts_before_read(
        resolver: Option<&ConflictResolver>,
        path: &Path,
        password: &SecretString,
    ) -> bool {
        let Some(resolver) = resolver else {
            return false;
        };
        let mut merge =
            |current: &[u8], version: &[u8]| merge_vault_bytes(current, version, password);
        resolver(path, &mut merge).unwrap_or(false)
    }

    /// Replaces the open vault's storage; stops the presenter of the one it replaced.
    fn replace_storage(
        &self,
        inner: &mut Inner,
        storage: ResolvedStorage,
    ) -> Option<ResolvedStorage> {
        let old = inner.storage.replace(storage);
        match (&old, &inner.storage) {
            (Some(old), Some(new)) if old.same_as(new) => None,
            _ => old,
        }
    }

    /// Called with the session lock released: stops `old`'s presenter and registers the open
    /// vault's, which debounces a remote change onto the scheduler (the platform thread that
    /// calls it never blocks on the session).
    fn attach_presenter(&self, old: Option<ResolvedStorage>) {
        if let Some(icloud) = old.and_then(|s| s.icloud) {
            icloud.clear_on_remote_change();
        }
        let icloud = lock(&self.0.inner)
            .storage
            .as_ref()
            .and_then(|s| s.icloud.clone());
        if let Some(icloud) = icloud {
            let weak = Arc::downgrade(&self.0);
            // A platform without presenters (or a refusal) leaves the 30 s poll as the check.
            let _ = icloud.set_on_remote_change(Arc::new(move || {
                if let Some(shared) = weak.upgrade() {
                    VaultSession(shared).remote_changed();
                }
            }));
        }
    }

    /// A remote change was reported: (re)arms one check after [`REMOTE_CHANGE_DEBOUNCE`], so a
    /// burst of notifications costs one check. Never takes the session lock.
    pub(crate) fn remote_changed(&self) {
        let weak = Arc::downgrade(&self.0);
        let handle = self.0.deps.scheduler.schedule(
            REMOTE_CHANGE_DEBOUNCE,
            Box::new(move || {
                if let Some(shared) = weak.upgrade() {
                    // Locked or no vault: nothing to check; the next unlock reads the file.
                    let _ = VaultSession(shared).check_external_change();
                }
            }),
        );
        if let Some(previous) = lock(&self.0.remote_job).replace(handle) {
            previous.cancel();
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

    pub(crate) fn emit(&self, out: Outbox) {
        self.notify_storage();
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

    pub(crate) fn check_not_quitting(&self) -> Result<(), SessionError> {
        if lock(&self.0.inner).quitting {
            Err(SessionError::Quitting)
        } else {
            Ok(())
        }
    }

    pub(crate) fn now(&self) -> Timestamp {
        match self.0.deps.scheduler.wall_now().duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp(i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
            Err(e) => Timestamp(-i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX)),
        }
    }

    /// Installs a freshly opened vault (after saving the previous one), then records it in the
    /// settings and notifies. `fresh` marks a file this session has just created.
    fn finish_open(
        &self,
        file: VaultFile,
        db: Database,
        password: SecretString,
        fresh: bool,
        storage: ResolvedStorage,
    ) -> Result<(), SessionError> {
        self.prepare_switch()?;
        let mut out = Outbox::default();
        let old_storage;
        let path = {
            let mut inner = self.wait_quiescent(lock(&self.0.inner));
            if inner.quitting {
                return Err(SessionError::Quitting);
            }
            if (inner.db.is_some() && inner.is_dirty()) || inner.pending_bytes.is_some() {
                // Edits landed between the save above and this point; never drop them.
                return Err(SessionError::InvalidInput(
                    "the open vault has unsaved changes; try again".into(),
                ));
            }
            inner.install(file, db, password);
            inner.fresh_file = fresh;
            old_storage = self.replace_storage(&mut inner, storage);
            out.state_changed = true;
            out.vault_changed = true;
            (inner.path.clone(), inner.vault_id.clone())
        };
        self.attach_presenter(old_storage);
        let (path, vault_id) = path;
        if let Some(p) = path {
            self.remember(&p);
        }
        self.emit(out);
        self.password_entered(vault_id);
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
        let storage = self.resolve_storage(path);
        storage.io.write_atomic(path, &bytes)?;
        let (file, db) = VaultFile::open_with(storage.io.clone(), path, &password)?;
        self.finish_open(file, db, password, true, storage)
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
                // Reopening the locked vault that holds unsaved changes is an unlock. When the
                // password opens the disk but not the kept edits (changed elsewhere), the normal
                // open below runs instead: the switch writes the kept edits as an `unsaved-*`
                // copy first (see the saving module docs).
                match self.unlock(password.clone()) {
                    Ok(()) => {
                        if let Some(p) = self.vault_path() {
                            self.remember(&p);
                        }
                        return Ok(());
                    }
                    Err(SessionError::WrongPassword) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        // The KDF runs without the session lock, so status() stays responsive meanwhile.
        let storage = self.resolve_storage(path);
        // iCloud conflict versions (and offline edits) are merged into the file before it is
        // read, with the password just typed.
        let conflicts = self.conflict_resolver_for(Some(&storage));
        Self::merge_conflicts_before_read(conflicts.as_ref(), path, &password);
        let (file, db) =
            VaultFile::open_with(storage.io.clone(), path, &password).map_err(open_error)?;
        self.finish_open(file, db, password, false, storage)
    }

    /// Unlocks the locked vault with `password`. Unlocking an unlocked session only verifies the
    /// password.
    ///
    /// When the last lock kept unsaved edits (`pending_bytes`), they are decrypted with the same
    /// password, merged with the vault on disk (which may have changed meanwhile) and saved. A
    /// password that opens the disk but not the kept edits (or the reverse) is `WrongPassword`
    /// and keeps them; see the module docs for the recovery path.
    pub fn unlock(&self, password: SecretString) -> Result<(), SessionError> {
        let r = self.unlock_inner(password, None).map(|_| ());
        if r.is_err() {
            // A failed read may have changed the storage health (still downloading).
            self.notify_storage();
        }
        r
    }

    /// The locked vault right now, as `(incarnation, vault id)`: what a device unlock binds its
    /// prompt to. `None` unless a vault is open, locked and the session is not quitting.
    pub(crate) fn locked_binding(&self) -> Option<(u64, String)> {
        let inner = lock(&self.0.inner);
        if inner.quitting || inner.db.is_some() || inner.path.is_none() {
            return None;
        }
        inner.vault_id.clone().map(|id| (inner.incarnation, id))
    }

    /// Unlocks with a password that did not come from the user's keyboard (device unlock), and
    /// only the locked vault of `incarnation` (see [`VaultSession::locked_binding`]): the check
    /// happens before the KDF and again, atomically, at the install. Any change in between (an
    /// unlock by other means, a lock, a switch to another vault or another copy of the same one,
    /// a relocation) gives [`ConditionalUnlock::Superseded`] and installs nothing. The
    /// [`PasswordListener`]s are not told.
    pub(crate) fn unlock_if_incarnation(
        &self,
        password: SecretString,
        incarnation: u64,
    ) -> Result<ConditionalUnlock, SessionError> {
        self.unlock_inner(password, Some(incarnation))
    }

    /// The unlocked vault's id and a copy of its password, read in one acquisition of the session
    /// lock (so the two always belong to the same vault). For device-unlock enrolment; the copy is
    /// zeroized on drop and never logged.
    pub(crate) fn identity_with_password(&self) -> Result<(String, SecretString), SessionError> {
        let inner = lock(&self.0.inner);
        match (&inner.db, &inner.password, &inner.vault_id) {
            (Some(_), Some(p), Some(id)) => Ok((id.clone(), p.clone())),
            _ => Err(inner.not_unlocked()),
        }
    }

    /// [`VaultSession::unlock`] (`expected` is `None`: a typed password, listeners told) or
    /// [`VaultSession::unlock_if_incarnation`] (`expected` is the incarnation, listeners not told).
    fn unlock_inner(
        &self,
        password: SecretString,
        expected: Option<u64>,
    ) -> Result<ConditionalUnlock, SessionError> {
        let superseded =
            |inner: &Inner| expected.is_some_and(|i| i != inner.incarnation || inner.db.is_some());
        // The KDF runs without the session lock; a retry write that consumed the kept edits
        // meanwhile makes the loop start over with the new state.
        for _ in 0..3 {
            let (path, pending, base, storage) = {
                let inner = self.wait_idle(lock(&self.0.inner));
                if inner.quitting {
                    return Err(SessionError::Quitting);
                }
                if superseded(&inner) {
                    return Ok(ConditionalUnlock::Superseded);
                }
                if inner.db.is_some() {
                    let vault_id = inner.vault_id.clone();
                    drop(inner);
                    self.verify_password(&password)?;
                    self.password_entered(vault_id);
                    return Ok(ConditionalUnlock::Unlocked);
                }
                let path = inner.path.clone().ok_or(SessionError::NoVault)?;
                (
                    path,
                    inner.pending_bytes.clone(),
                    inner.disk_meta.clone(),
                    inner.storage.clone(),
                )
            };
            let storage = storage.unwrap_or_else(|| self.resolve_storage(&path));
            let conflicts = self.conflict_resolver_for(Some(&storage));
            Self::merge_conflicts_before_read(conflicts.as_ref(), &path, &password);
            let (file, disk_db) =
                VaultFile::open_with(storage.io.clone(), &path, &password).map_err(open_error)?;
            let restored = match &pending {
                Some(bytes) => Some(crate::saving::reconcile(
                    &Database::open(bytes, &password)?,
                    &disk_db,
                    base.as_ref(),
                )),
                None => None,
            };
            let had_pending = restored.is_some();
            let mut out = Outbox::default();
            let old_storage;
            let vault_id = {
                let mut inner = self.wait_quiescent(lock(&self.0.inner));
                if inner.quitting {
                    return Err(SessionError::Quitting);
                }
                if superseded(&inner) {
                    return Ok(ConditionalUnlock::Superseded);
                }
                if inner.db.is_some() || inner.path.as_deref() != Some(path.as_path()) {
                    // Another call unlocked or switched vaults meanwhile; that state wins.
                    let vault_id = inner.vault_id.clone();
                    drop(inner);
                    self.verify_password(&password)?;
                    self.password_entered(vault_id);
                    return Ok(ConditionalUnlock::Unlocked);
                }
                if inner.pending_bytes != pending {
                    continue;
                }
                match restored {
                    Some(merged) => {
                        let unsaved = !crate::saving::same_on_disk(&merged, &disk_db);
                        let disk_meta = disk_db.meta.clone();
                        inner.install(file, merged, password);
                        inner.disk_meta = Some(disk_meta);
                        if unsaved {
                            inner.dirty_gen = 1;
                            out.save(inner.set_save(SaveStatus::pending()));
                        }
                    }
                    None => inner.install(file, disk_db, password),
                }
                old_storage = self.replace_storage(&mut inner, storage);
                out.state_changed = true;
                out.vault_changed = true;
                inner.vault_id.clone()
            };
            self.attach_presenter(old_storage);
            self.emit(out);
            if expected.is_none() {
                self.password_entered(vault_id);
            }
            if had_pending {
                // A failure shows as `Failed` and schedules a retry; the unlock itself succeeded.
                let _ = self.flush();
            }
            return Ok(ConditionalUnlock::Unlocked);
        }
        Err(SessionError::Io(
            "the unsaved changes were being written; try again".into(),
        ))
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
        self.mutate_tracked(|db, now| f(db, now).map(|r| (r, true)))
    }

    /// Like [`VaultSession::mutate`], but the closure also reports whether it changed anything.
    /// With `false` the clone is dropped: no swap, no dirty bump, no save scheduling and no
    /// change event, so a no-op edit never rewrites the vault file or burns a backup slot.
    pub fn mutate_tracked<R>(
        &self,
        f: impl FnOnce(&mut Database, Timestamp) -> Result<(R, bool), SessionError>,
    ) -> Result<R, SessionError> {
        match self.mutate_inner(None, f)? {
            Some(r) => Ok(r),
            // Unreachable: without an expected incarnation the edit always runs.
            None => Err(SessionError::Locked),
        }
    }

    fn mutate_inner<R>(
        &self,
        incarnation: Option<u64>,
        f: impl FnOnce(&mut Database, Timestamp) -> Result<(R, bool), SessionError>,
    ) -> Result<Option<R>, SessionError> {
        let now = self.now();
        let mut out = Outbox::default();
        let result = {
            let mut inner = lock(&self.0.inner);
            if inner.quitting {
                return Err(SessionError::Quitting);
            }
            if incarnation.is_some_and(|i| i != inner.incarnation) {
                return Ok(None);
            }
            let Some(db) = &inner.db else {
                return Err(inner.not_unlocked());
            };
            let mut draft = db.clone();
            let (r, changed) = f(&mut draft, now)?;
            if !changed {
                return Ok(Some(r));
            }
            inner.db = Some(draft);
            inner.dirty_gen = inner.dirty_gen.wrapping_add(1);
            if !inner.in_flight {
                out.save = inner.set_save(SaveStatus::pending());
            }
            self.schedule_autosave(&mut inner);
            out.vault_changed = true;
            r
        };
        self.emit(out);
        Ok(Some(result))
    }

    /// An in-memory change that does NOT mark the session dirty (access times).
    ///
    /// Unlike [`VaultSession::mutate`], which edits a clone and swaps it in only on `Ok`, the
    /// closure runs on the LIVE database: there is no rollback, and a panic midway leaves a
    /// half-applied change. It must be infallible and trivial (set a timestamp, bump a counter).
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
                // `subtle` compares without an early exit on the first differing byte (a length
                // mismatch still returns early, as with any password comparison).
                let same: bool = p
                    .expose_secret()
                    .as_bytes()
                    .ct_eq(candidate.expose_secret().as_bytes())
                    .into();
                if same {
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

    /// A copy of the unlocked vault for work that runs outside the session lock (the secondary
    /// sync: remote I/O and KDF runs). The password is a crate-private clone, like the one the
    /// save pipeline takes; it is never logged and zeroized when the snapshot drops. Refused
    /// while quitting.
    pub(crate) fn sync_snapshot(&self) -> Result<SyncSnapshot, SessionError> {
        let inner = lock(&self.0.inner);
        if inner.quitting {
            return Err(SessionError::Quitting);
        }
        match (&inner.db, &inner.password, &inner.path, &inner.vault_id) {
            (Some(db), Some(password), Some(path), Some(vault_id)) => Ok(SyncSnapshot {
                db: db.clone(),
                password: password.clone(),
                path: path.clone(),
                vault_id: vault_id.clone(),
                incarnation: inner.incarnation,
            }),
            _ => Err(inner.not_unlocked()),
        }
    }

    /// Whether the session still holds the vault of a snapshot taken at `incarnation` (not
    /// locked, switched or relocated since) and is not quitting.
    pub(crate) fn is_incarnation(&self, incarnation: u64) -> bool {
        let inner = lock(&self.0.inner);
        !inner.quitting && inner.db.is_some() && inner.incarnation == incarnation
    }

    /// [`VaultSession::mutate_tracked`] that runs only while the session still holds the vault
    /// of `incarnation`; `Ok(None)` (nothing changed) otherwise. The check and the edit happen
    /// under one acquisition of the session lock.
    pub(crate) fn mutate_if_incarnation<R>(
        &self,
        incarnation: u64,
        f: impl FnOnce(&mut Database, Timestamp) -> Result<(R, bool), SessionError>,
    ) -> Result<Option<R>, SessionError> {
        self.mutate_inner(Some(incarnation), f)
    }

    /// Runs `publish` (a local side effect describing the vault of `incarnation`, such as the
    /// secondary sync's checkpoint) only while the session still holds that vault unlocked and
    /// is not quitting, and while `still` (the caller's own identity check, asked under the
    /// session lock) holds; `None` when anything changed. The check is atomic with every
    /// invalidation: `publish` runs outside the session lock (it does file I/O), but while it
    /// runs every incarnation bump, `begin_quit` and [`VaultSession::while_unpublished`] wait
    /// for it, so a lock, switch, relocation, quit or arm change either happens before the
    /// check (nothing is published) or after the publication completed.
    pub(crate) fn publish_if_current<R>(
        &self,
        incarnation: u64,
        still: impl FnOnce() -> bool,
        publish: impl FnOnce() -> R,
    ) -> Option<R> {
        {
            let mut inner = lock(&self.0.inner);
            if inner.quitting || inner.db.is_none() || inner.incarnation != incarnation || !still()
            {
                return None;
            }
            inner.publishing = inner.publishing.saturating_add(1);
        }
        /// Ends the publication on every exit, a panic included, so no waiter hangs.
        struct Published<'a>(&'a VaultSession);
        impl Drop for Published<'_> {
            fn drop(&mut self) {
                let mut inner = lock(&self.0 .0.inner);
                inner.publishing = inner.publishing.saturating_sub(1);
                drop(inner);
                self.0 .0.idle.notify_all();
            }
        }
        let _published = Published(self);
        Some(publish())
    }

    /// Runs `f` under the session lock once no publication is in progress: an invalidation
    /// outside the session (the secondary sync's arm changes) serialised against
    /// [`VaultSession::publish_if_current`]. `f` must not call back into the session.
    pub(crate) fn while_unpublished<R>(&self, f: impl FnOnce() -> R) -> R {
        let inner = self.wait_unpublished(lock(&self.0.inner));
        let r = f();
        drop(inner);
        r
    }

    /// Whether `content` is on the local disk: the session still holds the vault of
    /// `incarnation`, has no unsaved edit (so the live vault is what the last write put on disk)
    /// and the live vault already holds everything in `content`. Waits for an in-flight write.
    pub(crate) fn durably_holds(&self, incarnation: u64, content: &Database) -> Durability {
        let inner = self.wait_idle(lock(&self.0.inner));
        if inner.quitting || inner.incarnation != incarnation {
            return Durability::Stale;
        }
        let Some(live) = &inner.db else {
            return Durability::Stale;
        };
        if inner.is_dirty() {
            return Durability::Unsaved;
        }
        if crate::saving::same_content(&mypass_core::merge::merge(live, content), live) {
            Durability::Durable
        } else {
            Durability::Missing
        }
    }

    /// Flushes, writes the current database to `new_path` through `io` (`AlreadyExists` when the
    /// target exists), reopens the session there and records the new path in the settings. The
    /// old file is left untouched. Refused while quitting.
    pub fn relocate(&self, new_path: &Path, io: Arc<dyn FileIo>) -> Result<(), SessionError> {
        self.check_not_quitting()?;
        if self.is_unlocked() {
            self.flush()?;
        }
        let mut out = Outbox::default();
        let result = self.relocate_locked(new_path, io);
        if result.is_ok() {
            if let Some(p) = self.vault_path() {
                self.remember(&p);
            }
            out.state_changed = true;
        }
        self.emit(out);
        result
    }

    fn relocate_locked(&self, new_path: &Path, io: Arc<dyn FileIo>) -> Result<(), SessionError> {
        let inner = self.wait_quiescent(lock(&self.0.inner));
        if inner.quitting {
            return Err(SessionError::Quitting);
        }
        if inner.db.is_none() {
            return Err(inner.not_unlocked());
        }
        let mut inner = inner;
        if inner.is_dirty() {
            return Err(SessionError::Io(
                "edits arrived while relocating; try again".into(),
            ));
        }
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
                inner.incarnation = inner.incarnation.wrapping_add(1);
                // The new file is written through `io`: no iCloud cache or presenter follows it.
                if let Some(icloud) = inner
                    .storage
                    .replace(ResolvedStorage::plain(self.0.io.clone()))
                    .and_then(|s| s.icloud)
                {
                    icloud.clear_on_remote_change();
                }
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
        // Waits for a publication in progress (a local checkpoint write), never for a vault
        // write: the quit flow bounds that wait itself (`flush_blocking`).
        self.wait_unpublished(lock(&self.0.inner)).quitting = true;
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
                KdbxError::Io {
                    kind: ErrorKind::TimedOut,
                    message: "the vault did not download from iCloud in time".into(),
                },
                SessionError::Downloading,
            ),
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
    fn publication_runs_only_for_the_current_unlocked_incarnation() {
        let f = fx();
        f.created();
        let current = lock(&f.session.0.inner).incarnation;
        let publish = |inc: u64, still: bool| f.session.publish_if_current(inc, || still, || 7);
        assert_eq!(publish(current, true), Some(7));
        assert_eq!(publish(current, false), None, "the caller's own check");
        assert_eq!(publish(current.wrapping_add(1), true), None);
        f.session.lock(LockReason::Manual);
        assert_eq!(publish(current, true), None, "locked");
        f.session.unlock(pw("pw")).unwrap();
        let unlocked = lock(&f.session.0.inner).incarnation;
        assert_ne!(unlocked, current);
        assert_eq!(publish(current, true), None, "an earlier unlock");
        assert_eq!(publish(unlocked, true), Some(7));
        f.session.begin_quit();
        assert_eq!(publish(unlocked, true), None, "quitting");
        // The publication count drops again on every exit, a panic included.
        f.session.cancel_quit();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.session
                .publish_if_current(unlocked, || true, || panic!("publication failed"))
        }));
        assert!(panicked.is_err());
        assert_eq!(lock(&f.session.0.inner).publishing, 0);
        f.session.lock(LockReason::Manual);
        assert!(
            !f.session.is_unlocked(),
            "a lock never hangs after a failed publication"
        );
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
