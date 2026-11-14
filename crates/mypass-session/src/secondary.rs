//! Secondary-folder sync: a second copy of the open vault in a folder the user chose (a USB
//! stick, a NAS share, a folder another sync client mirrors), kept in step with the primary file.
//!
//! [`SecondarySync`] observes the session and drives a `mypass_sync::SyncEngine` over a
//! `LocalFolderBackend`:
//!
//! - **Arming.** While a vault is unlocked and `Settings::secondary_folder` is set, the service is
//!   armed for that (vault id, primary path, folder): it syncs once at once (the remote may have
//!   changed while the app was closed), watches the folder and pushes
//!   [`SECONDARY_PUSH_DEBOUNCE`] after every completed save of the primary. Locking, closing or
//!   switching the vault, quitting and a folder change disarm it: the epoch is bumped, every
//!   timer is cancelled and the watcher stopped, so nothing armed for one vault runs for another.
//! - **The folder is untrusted (R7).** It is checked before every run: it must be absolute and an
//!   existing directory, and the copy in it must not be the primary file itself (the primary's own
//!   folder, through any alias). Anything else is `Unavailable { invalidFolder }`; a missing
//!   folder is `Unavailable { folderMissing }`. The setting changes only through the sync commands.
//! - **Ordering with the primary save pipeline.** A run waits until no primary write is in flight
//!   and then holds the session lock for the whole engine call, through
//!   `VaultSession::with_db_and_password` semantics: the password is borrowed in place and never
//!   cloned out of the session (R37), no primary write can start meanwhile, and `lock` waits for
//!   the run to end. The cost is that the UI waits for one sync (one or two KDF runs plus the
//!   folder I/O) when it touches the session during a run.
//! - **Adopting remote changes (the old "merge result discarded" bug).** A
//!   `SyncResult::Merged` is merged INTO the live database inside `VaultSession::mutate_tracked`
//!   (never a replacement, so an edit made while the engine ran is kept), the autosave then writes
//!   it to the primary, and only after the adoption succeeded is the merge checkpoint committed.
//!   An adoption that fails (vault locked or switched meanwhile) commits nothing: the next run
//!   merges again instead of pushing blindly over remote-only changes.
//! - **Failures never touch the primary.** They become a [`SecondaryStatus`] only. Transient ones
//!   (folder missing, read-only, timed out, an I/O error, a remote that kept changing) retry after
//!   [`SECONDARY_RETRY`]; the rest (wrong password, not a vault, another vault...) wait for the
//!   next save, watcher event or `sync_now`. A remote that does not open, is damaged, too large,
//!   not a regular file or another vault is never written (the engine's rules).
//! - **Echoes.** The engine notes its own writes in an `EchoFilter`; a watcher event whose remote
//!   content is exactly what this device last wrote runs no engine call.
//!
//! Nothing here logs; statuses and errors carry codes only (R37).
//!
//! Locking discipline: the run gate (one run at a time) is taken before the session lock; the
//! service's state lock is never held while calling the session, the scheduler's jobs or a
//! watcher's `stop` (which joins the watcher thread, whose callback takes the state lock).

use crate::app_core::ApiError;
use crate::saving::same_content;
use crate::scheduler::{lock, Scheduler, TaskHandle};
use crate::session::{SaveState, SessionError, SessionEvent, VaultSession};
use crate::settings::SettingsStore;
use mypass_core::merge::merge;
use mypass_kdbx::Database;
use mypass_platform::AppPaths;
use mypass_sync::{
    EchoFilter, FileCheckpointStore, FolderWatcher, LocalFolderBackend, MergedRemote,
    StorageBackend, StorageError, SyncCheckpoint, SyncCheckpointStore, SyncEngine, SyncError,
    SyncResult, WATCH_DEBOUNCE,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, UNIX_EPOCH};

/// Delay between a completed primary save and the push to the secondary folder.
pub const SECONDARY_PUSH_DEBOUNCE: Duration = Duration::from_secs(2);
/// Delay before a transient secondary failure (folder missing, read-only...) is retried.
pub const SECONDARY_RETRY: Duration = Duration::from_secs(300);
/// Folder under `AppPaths::data_dir` that holds one checkpoint folder per vault id.
pub const SYNC_STATE_DIR: &str = "sync";
/// The checkpoint file of the secondary folder, in `<data_dir>/sync/<vault_id>/`.
pub const SECONDARY_CHECKPOINT_FILE: &str = "secondary.json";

/// Why the secondary folder cannot be used right now (a localisable code).
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SecondaryUnavailable {
    /// Not absolute, not a directory, or the primary vault's own folder.
    InvalidFolder,
    /// The folder does not exist (an unmounted drive, a deleted folder).
    FolderMissing,
    /// The folder or the copy in it cannot be written.
    ReadOnly,
    /// The storage did not answer in time.
    TimedOut,
}

/// Why the last secondary sync failed (a localisable code). The remote was left as it was.
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SecondaryError {
    /// The copy does not open with the session password.
    WrongPassword,
    /// The file is not a KDBX vault.
    NotAVault,
    /// A vault format this app does not read.
    Unsupported,
    /// The copy is damaged.
    Corrupt,
    /// The file is a different vault (another root group).
    DifferentVault,
    /// The file is larger than any vault this app opens.
    TooLarge,
    /// The path is not a regular file (a folder, a device, a pipe).
    NotRegularFile,
    /// The copy kept changing while it was synced.
    RemoteBusy,
    /// A read, write or checkpoint failure.
    Io,
}

/// What the Settings sync pane shows (the `SyncStatusChanged` event payload).
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum SecondaryStatus {
    /// No secondary folder is configured.
    Off,
    /// Configured and not running; `last_sync` is the time of the last success for this vault
    /// and folder in this run of the app, in unix milliseconds.
    Idle {
        #[serde(rename = "lastSync")]
        last_sync: Option<f64>,
    },
    /// A sync is running.
    Syncing,
    /// The folder cannot be used; retried every [`SECONDARY_RETRY`] except for `invalidFolder`.
    Unavailable { reason: SecondaryUnavailable },
    /// The last sync failed; the remote and the primary are untouched.
    Error { code: SecondaryError },
}

/// A status observer; runs after the service's lock is released, on the calling thread.
pub type SecondaryObserver = Arc<dyn Fn(&SecondaryStatus) + Send + Sync>;

/// What a folder watcher calls on a (debounced) change of the watched file.
pub type WatchCallback = Arc<dyn Fn() + Send + Sync>;

/// Starts folder watchers (production: `mypass_sync::FolderWatcher`; tests: a fake they fire).
pub trait FolderWatch: Send + Sync {
    /// Watches `file_name` in `dir`; `on_change` must only hand work off (it may run on the
    /// watcher's thread).
    fn watch(
        &self,
        dir: &Path,
        file_name: &str,
        on_change: WatchCallback,
    ) -> Result<Box<dyn WatchGuard>, StorageError>;
}

/// A running watcher. After `stop` returns no callback runs any more.
pub trait WatchGuard: Send {
    fn stop(self: Box<Self>);
}

/// The production [`FolderWatch`]: `notify` through `mypass_sync::FolderWatcher`, debounced by
/// `WATCH_DEBOUNCE`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NotifyFolderWatch;

impl FolderWatch for NotifyFolderWatch {
    fn watch(
        &self,
        dir: &Path,
        file_name: &str,
        on_change: WatchCallback,
    ) -> Result<Box<dyn WatchGuard>, StorageError> {
        Ok(Box::new(FolderWatcher::start(
            dir,
            file_name,
            WATCH_DEBOUNCE,
            on_change,
        )?))
    }
}

impl WatchGuard for FolderWatcher {
    fn stop(self: Box<Self>) {
        FolderWatcher::stop(*self);
    }
}

/// Why a run started; only a watcher event pulls without pushing the local vault.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunKind {
    /// First run after arming (unlock, vault switch, folder change).
    Arm,
    /// After a completed primary save.
    Push,
    /// The watcher saw the remote change.
    Pull,
    /// The retry timer after a transient failure.
    Retry,
    /// `sync_now`.
    Now,
}

impl RunKind {
    fn push_local(self) -> bool {
        self != RunKind::Pull
    }
}

/// What the service is armed for; a change of any part re-arms.
#[derive(Clone, PartialEq, Eq, Debug)]
struct ArmKey {
    vault_id: String,
    /// The canonical primary path (`VaultSession::vault_path`).
    primary: PathBuf,
    /// The folder exactly as stored in the settings.
    folder: String,
}

struct Armed {
    key: ArmKey,
    echo: Arc<EchoFilter>,
    watcher: Option<Box<dyn WatchGuard>>,
}

struct LastSync {
    vault_id: String,
    folder: String,
    /// Unix milliseconds.
    at: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Push,
    Pull,
    Retry,
    Arm,
}

#[derive(Default)]
struct Jobs {
    seq: u64,
    push: Option<(u64, TaskHandle)>,
    pull: Option<(u64, TaskHandle)>,
    retry: Option<(u64, TaskHandle)>,
    arm: Option<(u64, TaskHandle)>,
}

impl Jobs {
    fn slot(&mut self, slot: Slot) -> &mut Option<(u64, TaskHandle)> {
        match slot {
            Slot::Push => &mut self.push,
            Slot::Pull => &mut self.pull,
            Slot::Retry => &mut self.retry,
            Slot::Arm => &mut self.arm,
        }
    }

    fn cancel(&mut self, slot: Slot) {
        if let Some((_, h)) = self.slot(slot).take() {
            h.cancel();
        }
    }

    fn cancel_all(&mut self) {
        for slot in [Slot::Push, Slot::Pull, Slot::Retry, Slot::Arm] {
            self.cancel(slot);
        }
    }
}

struct State {
    /// Bumped by every arm and disarm; a job, a watcher event or a run of an older epoch does
    /// nothing.
    epoch: u64,
    armed: Option<Armed>,
    status: SecondaryStatus,
    last_sync: Option<LastSync>,
    /// The last save state seen, to detect a completed save (Pending or Saving, then Idle).
    last_save: SaveState,
    jobs: Jobs,
}

/// What one run came to.
enum Outcome {
    Synced,
    /// The vault was locked, closed or switched meanwhile: nothing to report.
    Stale,
    Unavailable(SecondaryUnavailable),
    Failed(SyncError),
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct Probe {
    /// Engine calls made (a run that stops at the echo check makes none).
    pub(crate) engine_runs: std::sync::atomic::AtomicUsize,
    /// Runs between the engine call and the adoption of a merge (test-only).
    #[allow(clippy::type_complexity)]
    pub(crate) before_adopt: Mutex<Option<Box<dyn FnMut() + Send>>>,
}

struct Shared {
    session: VaultSession,
    settings: SettingsStore,
    scheduler: Arc<dyn Scheduler>,
    /// `<data_dir>/sync`.
    sync_root: PathBuf,
    watch: Arc<dyn FolderWatch>,
    state: Mutex<State>,
    /// One run at a time (taken before the session lock).
    run_gate: Mutex<()>,
    observers: RwLock<Vec<SecondaryObserver>>,
    attached: AtomicBool,
    #[cfg(test)]
    probe: Probe,
}

/// The secondary-folder sync service (cheap to clone; every clone is the same service).
#[derive(Clone)]
pub struct SecondarySync(Arc<Shared>);

impl std::fmt::Debug for SecondarySync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = lock(&self.0.state);
        f.debug_struct("SecondarySync")
            .field("status", &st.status)
            .field("armed", &st.armed.is_some())
            .finish_non_exhaustive()
    }
}

fn vault_id_of(db: &Database) -> String {
    db.root_id().simple().to_string()
}

impl SecondarySync {
    /// The production service: `notify` folder watchers. Call [`SecondarySync::attach`] to start.
    pub fn new(
        session: VaultSession,
        settings: SettingsStore,
        scheduler: Arc<dyn Scheduler>,
        paths: &AppPaths,
    ) -> Self {
        Self::with_watch(
            session,
            settings,
            scheduler,
            paths,
            Arc::new(NotifyFolderWatch),
        )
    }

    /// Like [`SecondarySync::new`] with another folder watcher (tests fire a fake).
    pub fn with_watch(
        session: VaultSession,
        settings: SettingsStore,
        scheduler: Arc<dyn Scheduler>,
        paths: &AppPaths,
        watch: Arc<dyn FolderWatch>,
    ) -> Self {
        let status = if settings.get().secondary_folder.is_some() {
            SecondaryStatus::Idle { last_sync: None }
        } else {
            SecondaryStatus::Off
        };
        Self(Arc::new(Shared {
            session,
            settings,
            scheduler,
            sync_root: paths.data_dir.join(SYNC_STATE_DIR),
            watch,
            state: Mutex::new(State {
                epoch: 0,
                armed: None,
                status,
                last_sync: None,
                last_save: SaveState::Idle,
                jobs: Jobs::default(),
            }),
            run_gate: Mutex::new(()),
            observers: RwLock::new(Vec::new()),
            attached: AtomicBool::new(false),
            #[cfg(test)]
            probe: Probe::default(),
        }))
    }

    #[cfg(test)]
    pub(crate) fn probe(&self) -> &Probe {
        &self.0.probe
    }

    /// The current status.
    pub fn status(&self) -> SecondaryStatus {
        lock(&self.0.state).status.clone()
    }

    /// Registers an observer for every later status change.
    pub fn add_observer(&self, o: SecondaryObserver) {
        self.0
            .observers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(o);
    }

    /// Subscribes to the session (unlock, lock, vault switch, completed saves) and arms for the
    /// current state. Idempotent.
    pub fn attach(&self) {
        if self.0.attached.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(&self.0);
        self.0
            .session
            .add_observer(Arc::new(move |event: &SessionEvent| {
                if let Some(shared) = weak.upgrade() {
                    SecondarySync(shared).on_session_event(event);
                }
            }));
        self.rearm();
    }

    /// Syncs now (push and pull) and returns the resulting status. `Off` when no folder is set;
    /// `Locked` / `NoVault` unless a vault is open.
    pub fn sync_now(&self) -> Result<SecondaryStatus, ApiError> {
        if self.0.settings.get().secondary_folder.is_none() {
            self.rearm();
            return Ok(self.status());
        }
        self.0.session.with_db(|_| ())?;
        self.rearm();
        let epoch = lock(&self.0.state).epoch;
        Ok(self.run(RunKind::Now, epoch))
    }

    /// Clears the secondary folder setting and stops syncing; the copy in the folder is left as
    /// it is. The current vault's checkpoint is cleared (best effort), so a later connect merges
    /// with whatever the folder then holds instead of writing over it.
    pub fn disconnect(&self) -> Result<(), ApiError> {
        let _gate = lock(&self.0.run_gate);
        self.0.settings.set_secondary_folder(None)?;
        if let Some(vault_id) = self.0.session.vault_id() {
            let path = self.checkpoint_path(&vault_id);
            if path.exists() {
                // Best effort: a stale checkpoint only allows writing over a copy that is exactly
                // what this device wrote last.
                let _ = FileCheckpointStore::new(path).save(None);
            }
        }
        self.rearm();
        Ok(())
    }

    /// Re-reads the session and the settings and arms, re-arms or disarms. Call it after
    /// `SettingsStore::set_secondary_folder` (the session events call it themselves).
    pub fn rearm(&self) {
        let desired = self.desired_key();
        let vault_id = self.0.session.vault_id();
        let folder = self.0.settings.get().secondary_folder;
        let (old, changed) = {
            let mut st = lock(&self.0.state);
            if st.armed.as_ref().map(|a| &a.key) == desired.as_ref() {
                // Still disarmed: only the configured / not configured status can have changed.
                let changed = if st.armed.is_none() {
                    let status = idle_status(&st, vault_id.as_deref(), folder.as_deref());
                    set_status(&mut st, status)
                } else {
                    None
                };
                drop(st);
                if let Some(status) = changed {
                    self.notify(&status);
                }
                return;
            }
            st.epoch = st.epoch.wrapping_add(1);
            st.jobs.cancel_all();
            let old = st.armed.take();
            if let Some(key) = desired {
                st.armed = Some(Armed {
                    key,
                    echo: Arc::new(EchoFilter::new()),
                    watcher: None,
                });
                self.schedule(&mut st, Slot::Arm, Duration::ZERO, RunKind::Arm);
            }
            let status = idle_status(&st, vault_id.as_deref(), folder.as_deref());
            let changed = set_status(&mut st, status);
            (old, changed)
        };
        if let Some(watcher) = old.and_then(|a| a.watcher) {
            watcher.stop();
        }
        if let Some(status) = changed {
            self.notify(&status);
        }
    }

    fn desired_key(&self) -> Option<ArmKey> {
        let folder = self.0.settings.get().secondary_folder?;
        if !self.0.session.is_unlocked() {
            return None;
        }
        Some(ArmKey {
            vault_id: self.0.session.vault_id()?,
            primary: self.0.session.vault_path()?,
            folder,
        })
    }

    fn checkpoint_path(&self, vault_id: &str) -> PathBuf {
        self.0
            .sync_root
            .join(vault_id)
            .join(SECONDARY_CHECKPOINT_FILE)
    }

    fn on_session_event(&self, event: &SessionEvent) {
        match event {
            SessionEvent::StateChanged(_) => self.rearm(),
            SessionEvent::SaveChanged(s) => {
                let mut st = lock(&self.0.state);
                let previous = std::mem::replace(&mut st.last_save, s.state);
                let completed = s.state == SaveState::Idle
                    && matches!(previous, SaveState::Pending | SaveState::Saving);
                if completed && st.armed.is_some() {
                    self.schedule(&mut st, Slot::Push, SECONDARY_PUSH_DEBOUNCE, RunKind::Push);
                }
            }
            SessionEvent::VaultChanged => {}
        }
    }

    /// (Re)schedules the job of `slot` for the current epoch, cancelling the one it replaces.
    fn schedule(&self, st: &mut State, slot: Slot, after: Duration, kind: RunKind) {
        st.jobs.cancel(slot);
        st.jobs.seq = st.jobs.seq.wrapping_add(1);
        let id = st.jobs.seq;
        let epoch = st.epoch;
        let weak: Weak<Shared> = Arc::downgrade(&self.0);
        let job = Box::new(move || {
            if let Some(shared) = weak.upgrade() {
                let me = SecondarySync(shared);
                if me.take_job(slot, id, epoch) {
                    me.run(kind, epoch);
                }
            }
        });
        let handle = self.0.scheduler.schedule(after, job);
        *st.jobs.slot(slot) = Some((id, handle));
    }

    /// Clears the slot when job `id` is still the current one of `epoch`; `false` means it was
    /// superseded or the service re-armed (a cancelled job may still run once).
    fn take_job(&self, slot: Slot, id: u64, epoch: u64) -> bool {
        let mut st = lock(&self.0.state);
        let current = st.jobs.slot(slot).as_ref().map(|(i, _)| *i) == Some(id);
        if current {
            *st.jobs.slot(slot) = None;
        }
        current && st.epoch == epoch
    }

    /// The watcher's callback: hand a pull to the scheduler (coalesced), never sync here.
    fn on_remote_change(&self, epoch: u64) {
        let mut st = lock(&self.0.state);
        if st.epoch != epoch || st.armed.is_none() || st.jobs.pull.is_some() {
            return;
        }
        self.schedule(&mut st, Slot::Pull, Duration::ZERO, RunKind::Pull);
    }

    /// Starts the watcher of the current arm when none runs.
    fn ensure_watcher(&self, epoch: u64, dir: &Path, file_name: &str) {
        {
            let st = lock(&self.0.state);
            match &st.armed {
                Some(armed) if st.epoch == epoch && armed.watcher.is_none() => {}
                _ => return,
            }
        }
        let weak: Weak<Shared> = Arc::downgrade(&self.0);
        let callback: WatchCallback = Arc::new(move || {
            if let Some(shared) = weak.upgrade() {
                SecondarySync(shared).on_remote_change(epoch);
            }
        });
        // A failure leaves the folder unwatched; the run reports the folder's state and a retry
        // tries the watcher again.
        let Ok(guard) = self.0.watch.watch(dir, file_name, callback) else {
            return;
        };
        let unused = {
            let mut st = lock(&self.0.state);
            let current = st.epoch == epoch;
            match st.armed.as_mut() {
                Some(armed) if current && armed.watcher.is_none() => {
                    armed.watcher = Some(guard);
                    None
                }
                _ => Some(guard),
            }
        };
        if let Some(guard) = unused {
            guard.stop();
        }
    }

    /// One sync of the current arm. Returns the status afterwards.
    fn run(&self, kind: RunKind, epoch: u64) -> SecondaryStatus {
        let _gate = lock(&self.0.run_gate);
        let (key, echo) = {
            let st = lock(&self.0.state);
            match &st.armed {
                Some(armed) if st.epoch == epoch => (armed.key.clone(), armed.echo.clone()),
                _ => return st.status.clone(),
            }
        };
        if self.0.session.is_quitting() {
            return self.status();
        }
        if self.0.settings.get().secondary_folder.as_deref() != Some(key.folder.as_str()) {
            // The setting changed without a re-arm: follow it.
            self.rearm();
            return self.status();
        }
        let Some(file_name) = key.primary.file_name().and_then(|n| n.to_str()) else {
            return self.finish(
                epoch,
                Outcome::Unavailable(SecondaryUnavailable::InvalidFolder),
            );
        };
        let dir = match validate_folder(&key.folder, &key.primary, file_name) {
            Ok(dir) => dir,
            Err(reason) => return self.finish(epoch, Outcome::Unavailable(reason)),
        };
        let backend = match LocalFolderBackend::new(&dir, file_name) {
            Ok(backend) => Arc::new(backend),
            Err(e) => return self.finish(epoch, storage_outcome(e)),
        };
        self.ensure_watcher(epoch, &dir, file_name);
        if kind == RunKind::Pull {
            if let Ok(Some(bytes)) = backend.read() {
                if echo.is_echo(&SyncCheckpoint::of_bytes(&bytes, None)) {
                    return self.status();
                }
            }
        }
        if let Some(status) = self.set_status_if(epoch, SecondaryStatus::Syncing) {
            self.notify(&status);
        }
        #[cfg(test)]
        self.0.probe.engine_runs.fetch_add(1, Ordering::SeqCst);
        let checkpoints = Arc::new(FileCheckpointStore::new(
            self.checkpoint_path(&key.vault_id),
        ));
        let engine = SyncEngine::new(backend, checkpoints).with_echo_filter(echo);
        let synced = self
            .0
            .session
            .with_idle_db_and_password(|db, password, path| {
                // The vault switched between the arm check and here: not this arm's vault.
                if vault_id_of(db) != key.vault_id || path != Some(key.primary.as_path()) {
                    return None;
                }
                Some(engine.sync(db, password, kind.push_local()))
            });
        let outcome = match synced {
            Ok(Some(Ok(SyncResult::Merged(merged)))) => self.adopt(&engine, &key, *merged),
            Ok(Some(Ok(_))) => Outcome::Synced,
            Ok(Some(Err(e))) => Outcome::Failed(e),
            Ok(None) | Err(_) => Outcome::Stale,
        };
        self.finish(epoch, outcome)
    }

    /// Merges the remote's changes into the LIVE database (edits made while the engine ran are
    /// kept), then commits the merge checkpoint. A failed adoption commits nothing.
    fn adopt(&self, engine: &SyncEngine, key: &ArmKey, merged: MergedRemote) -> Outcome {
        let (remote, pending) = merged.into_parts();
        #[cfg(test)]
        if let Some(hook) = lock(&self.0.probe.before_adopt).as_mut() {
            hook();
        }
        let adopted = self.0.session.mutate_tracked(|live, _now| {
            if vault_id_of(live) != key.vault_id {
                return Err(SessionError::InvalidInput(
                    "the vault changed during the sync".into(),
                ));
            }
            let next = merge(live, &remote);
            if same_content(&next, live) {
                return Ok(((), false));
            }
            *live = next;
            Ok(((), true))
        });
        match adopted {
            Ok(()) => match engine.commit_merged(pending) {
                Ok(()) => Outcome::Synced,
                Err(e) => Outcome::Failed(e),
            },
            Err(_) => Outcome::Stale,
        }
    }

    /// Records the outcome of a run of `epoch` (ignored when the service re-armed meanwhile),
    /// arms the retry for transient failures and stops the watcher while the folder is
    /// unavailable.
    fn finish(&self, epoch: u64, outcome: Outcome) -> SecondaryStatus {
        let now = self.now_millis();
        let (status, stop, changed) = {
            let mut st = lock(&self.0.state);
            let Some(key) = st.armed.as_ref().map(|a| a.key.clone()) else {
                return st.status.clone();
            };
            if st.epoch != epoch {
                return st.status.clone();
            }
            let status = match outcome {
                Outcome::Synced => {
                    st.last_sync = Some(LastSync {
                        vault_id: key.vault_id.clone(),
                        folder: key.folder.clone(),
                        at: now,
                    });
                    SecondaryStatus::Idle {
                        last_sync: Some(now),
                    }
                }
                Outcome::Stale => SecondaryStatus::Idle {
                    last_sync: last_sync_for(&st, Some(&key.vault_id), &key.folder),
                },
                Outcome::Unavailable(reason) => SecondaryStatus::Unavailable { reason },
                Outcome::Failed(e) => failure_status(&e),
            };
            if is_transient(&status) {
                self.schedule(&mut st, Slot::Retry, SECONDARY_RETRY, RunKind::Retry);
            } else {
                st.jobs.cancel(Slot::Retry);
            }
            let stop = if matches!(status, SecondaryStatus::Unavailable { .. }) {
                st.armed.as_mut().and_then(|a| a.watcher.take())
            } else {
                None
            };
            let changed = set_status(&mut st, status.clone());
            (status, stop, changed)
        };
        if let Some(watcher) = stop {
            watcher.stop();
        }
        if let Some(status) = changed {
            self.notify(&status);
        }
        status
    }

    /// Sets the status when `epoch` is still current; returns it when it changed.
    fn set_status_if(&self, epoch: u64, status: SecondaryStatus) -> Option<SecondaryStatus> {
        let mut st = lock(&self.0.state);
        if st.epoch != epoch {
            return None;
        }
        set_status(&mut st, status)
    }

    fn notify(&self, status: &SecondaryStatus) {
        let observers = self
            .0
            .observers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for o in &observers {
            o(status);
        }
    }

    fn now_millis(&self) -> f64 {
        match self.0.scheduler.wall_now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64() * 1000.0,
            Err(_) => 0.0,
        }
    }
}

fn set_status(st: &mut State, status: SecondaryStatus) -> Option<SecondaryStatus> {
    (st.status != status).then(|| {
        st.status = status.clone();
        status
    })
}

/// `Off` without a folder, otherwise `Idle` with the last success for this vault and folder.
fn idle_status(st: &State, vault_id: Option<&str>, folder: Option<&str>) -> SecondaryStatus {
    match folder {
        None => SecondaryStatus::Off,
        Some(folder) => SecondaryStatus::Idle {
            last_sync: last_sync_for(st, vault_id, folder),
        },
    }
}

fn last_sync_for(st: &State, vault_id: Option<&str>, folder: &str) -> Option<f64> {
    st.last_sync
        .as_ref()
        .filter(|l| Some(l.vault_id.as_str()) == vault_id && l.folder == folder)
        .map(|l| l.at)
}

/// Whether the retry timer should run for this status.
fn is_transient(status: &SecondaryStatus) -> bool {
    match status {
        SecondaryStatus::Unavailable { reason } => *reason != SecondaryUnavailable::InvalidFolder,
        SecondaryStatus::Error { code } => {
            matches!(code, SecondaryError::Io | SecondaryError::RemoteBusy)
        }
        SecondaryStatus::Off | SecondaryStatus::Idle { .. } | SecondaryStatus::Syncing => false,
    }
}

fn storage_outcome(e: StorageError) -> Outcome {
    match e {
        StorageError::Unavailable(_) => Outcome::Unavailable(SecondaryUnavailable::FolderMissing),
        StorageError::ReadOnly => Outcome::Unavailable(SecondaryUnavailable::ReadOnly),
        StorageError::TimedOut => Outcome::Unavailable(SecondaryUnavailable::TimedOut),
        StorageError::InvalidPath(_) => Outcome::Unavailable(SecondaryUnavailable::InvalidFolder),
        other => Outcome::Failed(SyncError::Storage(other)),
    }
}

/// The status for an engine failure (typed codes only, R17).
fn failure_status(e: &SyncError) -> SecondaryStatus {
    let code = match e {
        SyncError::Storage(storage) => {
            return match storage_outcome(storage.clone()) {
                Outcome::Unavailable(reason) => SecondaryStatus::Unavailable { reason },
                _ => SecondaryStatus::Error {
                    code: match storage {
                        StorageError::TooLarge { .. } => SecondaryError::TooLarge,
                        StorageError::NotRegularFile => SecondaryError::NotRegularFile,
                        _ => SecondaryError::Io,
                    },
                },
            }
        }
        SyncError::RemotePasswordMismatch => SecondaryError::WrongPassword,
        SyncError::RemoteNotAVault => SecondaryError::NotAVault,
        SyncError::RemoteUnsupported(_) => SecondaryError::Unsupported,
        SyncError::RemoteCorrupt(_) => SecondaryError::Corrupt,
        SyncError::RemoteIsDifferentVault => SecondaryError::DifferentVault,
        SyncError::RemoteChangedDuringSync => SecondaryError::RemoteBusy,
        SyncError::Checkpoint(_) | SyncError::Encode(_) => SecondaryError::Io,
    };
    SecondaryStatus::Error { code }
}

/// Checks the folder from the settings (untrusted, R7) and returns it canonicalised: absolute,
/// an existing directory, and not the primary vault's own folder (the copy there would be the
/// primary file itself), through any alias.
pub(crate) fn validate_folder(
    folder: &str,
    primary: &Path,
    file_name: &str,
) -> Result<PathBuf, SecondaryUnavailable> {
    let path = Path::new(folder);
    if folder.is_empty() || !path.is_absolute() {
        return Err(SecondaryUnavailable::InvalidFolder);
    }
    let meta = fs::metadata(path).map_err(|_| SecondaryUnavailable::FolderMissing)?;
    if !meta.is_dir() {
        return Err(SecondaryUnavailable::InvalidFolder);
    }
    let dir = fs::canonicalize(path).map_err(|_| SecondaryUnavailable::FolderMissing)?;
    let primary = fs::canonicalize(primary).unwrap_or_else(|_| primary.to_path_buf());
    if let Some(parent) = primary.parent() {
        if dir == parent || same_node(&dir, parent) {
            return Err(SecondaryUnavailable::InvalidFolder);
        }
    }
    let target = dir.join(file_name);
    if fs::canonicalize(&target).is_ok_and(|t| t == primary) || same_node(&target, &primary) {
        return Err(SecondaryUnavailable::InvalidFolder);
    }
    Ok(dir)
}

/// Whether two existing paths are the same file-system node (catches a case-insensitive alias
/// or a hard link that `canonicalize` leaves different).
#[cfg(unix)]
fn same_node(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Windows (stage 6): `canonicalize` already resolves aliases; compare case-insensitively.
#[cfg(not(unix))]
fn same_node(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase(),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
