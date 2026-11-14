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
//!   folder, through any alias). Anything else (a file, a symlink loop, a path that cannot be
//!   inspected) is `Unavailable { invalidFolder }`; only a path that does not exist (an unmounted
//!   drive, a deleted folder) is `Unavailable { folderMissing }`, which is retried. The setting
//!   changes only through the sync commands.
//! - **No session lock during remote work.** A run takes a snapshot of the unlocked vault
//!   (`VaultSession::sync_snapshot`: a clone of the database and a crate-private clone of the
//!   password, a zeroizing `SecretString` that is never logged) together with the session's
//!   incarnation, releases the session lock and runs the engine (folder I/O, one or two KDF
//!   runs) outside it, so the UI, the autosave and `lock` never wait for a sync. Right before
//!   every remote write the engine asks whether the arm and the incarnation are still current;
//!   a lock, unlock, vault switch, relocation or quit in between cancels the run before it
//!   writes.
//! - **Adopting remote changes, then durability, then the checkpoint.** A `SyncResult::Merged` is
//!   merged INTO the live database (never a replacement, so an edit made while the engine ran is
//!   kept) inside one session-lock acquisition that first re-checks the incarnation; a changed
//!   one discards the result (the re-arm that came with the change schedules the next run). The
//!   synced content is then flushed to the primary file, and only once the session reports it
//!   on disk (`VaultSession::durably_holds`) is the engine's checkpoint committed. The checkpoint
//!   means "the copy holds nothing the primary file on disk lacks", so a crash at any point
//!   before the commit only costs an idempotent re-merge on the next run, never a blind push
//!   over remote-only changes. A primary that cannot be written leaves the checkpoint as it was
//!   and retries after [`SECONDARY_RETRY`].
//! - **Checkpoints are per vault and primary file.** Two copies of one vault (same root UUID) at
//!   different paths keep separate checkpoints, because the promise is about one primary file.
//! - **Reconciling without rewriting.** Arming and the retry timer use `SyncMode::PushIfChanged`:
//!   an unlock with nothing new for the copy writes nothing. A completed save and `sync_now`
//!   push, a watcher event only pulls.
//! - **The watcher is part of the health.** A watcher that cannot start shows as
//!   `Error { watchFailed }` (the data sync itself still runs) and is retried on the scheduler
//!   from [`SECONDARY_WATCH_RETRY_MIN`], doubling up to [`SECONDARY_RETRY`]; once it runs, a
//!   pull catches up on what it missed.
//! - **Failures never touch the primary.** They become a [`SecondaryStatus`] only. Transient ones
//!   (folder missing, read-only, timed out, an I/O error, a remote that kept changing) retry after
//!   [`SECONDARY_RETRY`]; the rest (wrong password, not a vault, another vault...) wait for the
//!   next save, watcher event or `sync_now`. A remote that does not open, is damaged, too large,
//!   not a regular file or another vault is never written (the engine's rules).
//! - **Echoes.** The engine notes its own writes in an `EchoFilter`; a watcher event whose remote
//!   content is exactly what this device last wrote runs no engine call.
//!
//! - **Runs never block the scheduler.** Every run happens on the service's own worker thread
//!   (`mypass-secondary-sync`, started on first use). Scheduler jobs, watcher events and
//!   `sync_now` only queue a request, so a hung folder (a stalled network share) delays nothing
//!   else on the scheduler. At most one run is in flight; queued requests are kept in order,
//!   duplicates of a queued request are dropped and requests of an earlier arm are discarded when
//!   a newer one arrives. Lock, unlock, a vault switch, a folder change and quitting never wait
//!   for the worker: they bump the epoch or the session incarnation, and the run in flight stops
//!   at its next check (before every attempt, remote decryption and remote write) without
//!   writing. [`SecondarySync::shutdown`] (the app's exit) disarms and stops the worker without
//!   waiting; [`SecondarySync::wait_idle`] waits for it with a bound. A panic inside a run is
//!   contained, so one faulty run cannot stop syncing.
//!
//! Nothing here logs; statuses and errors carry codes only (R37).
//!
//! Locking discipline: the run gate (one run at a time) is taken before the session lock and the
//! session lock is held only for the snapshot, the adoption and the durability check, never
//! across remote I/O; the service's state lock is never held while calling the session, the
//! scheduler's jobs or a watcher's `stop` (which joins the watcher thread, whose callback takes
//! the state lock). The worker queue's lock is a leaf: nothing else is taken while holding it.

use crate::app_core::ApiError;
use crate::saving::same_content;
use crate::scheduler::{lock, Scheduler, TaskHandle};
use crate::session::{Durability, SaveState, SessionEvent, SyncSnapshot, VaultSession};
use crate::settings::SettingsStore;
use mypass_core::merge::merge;
use mypass_kdbx::Database;
use mypass_platform::AppPaths;
use mypass_sync::{
    EchoFilter, FileCheckpointStore, FolderWatcher, LocalFolderBackend, StorageBackend,
    StorageError, SyncCheckpoint, SyncCheckpointStore, SyncEngine, SyncError, SyncMode, SyncResult,
    WATCH_DEBOUNCE,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs;
use std::io::ErrorKind;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock, Weak};
use std::thread::ThreadId;
use std::time::{Duration, UNIX_EPOCH};

/// Delay between a completed primary save and the push to the secondary folder.
pub const SECONDARY_PUSH_DEBOUNCE: Duration = Duration::from_secs(2);
/// Delay before a transient secondary failure (folder missing, read-only...) is retried.
pub const SECONDARY_RETRY: Duration = Duration::from_secs(300);
/// First delay before a folder watcher that failed to start is tried again; it doubles on every
/// further failure, up to [`SECONDARY_RETRY`].
pub const SECONDARY_WATCH_RETRY_MIN: Duration = Duration::from_secs(5);
/// The name of the service's worker thread, which runs every sync.
pub const SECONDARY_WORKER_THREAD: &str = "mypass-secondary-sync";
/// Folder under `AppPaths::data_dir` that holds one checkpoint folder per vault id.
pub const SYNC_STATE_DIR: &str = "sync";
/// How many flushes a run tries while edits keep arriving before it leaves the checkpoint for
/// the next run.
const DURABLE_TRIES: usize = 3;

/// The checkpoint file of the secondary folder for one primary file, in
/// `<data_dir>/sync/<vault_id>/`: `secondary-<first 16 hex digits of SHA-256(primary path)>.json`.
pub fn secondary_checkpoint_file(primary: &Path) -> String {
    let digest = Sha256::digest(primary.as_os_str().as_encoded_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("secondary-{hex}.json")
}

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
    /// The copy is synced, but changes in the folder are not noticed: the folder watcher could
    /// not start. It is retried with a growing delay.
    WatchFailed,
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
    /// The retry of a folder watcher that failed to start (a pull once it runs).
    Watch,
}

impl RunKind {
    fn mode(self) -> SyncMode {
        match self {
            RunKind::Pull | RunKind::Watch => SyncMode::Pull,
            RunKind::Push | RunKind::Now => SyncMode::Push,
            RunKind::Arm | RunKind::Retry => SyncMode::PushIfChanged,
        }
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
    Watch,
}

#[derive(Default)]
struct Jobs {
    seq: u64,
    push: Option<(u64, TaskHandle)>,
    pull: Option<(u64, TaskHandle)>,
    retry: Option<(u64, TaskHandle)>,
    arm: Option<(u64, TaskHandle)>,
    watch: Option<(u64, TaskHandle)>,
}

impl Jobs {
    fn slot(&mut self, slot: Slot) -> &mut Option<(u64, TaskHandle)> {
        match slot {
            Slot::Push => &mut self.push,
            Slot::Pull => &mut self.pull,
            Slot::Retry => &mut self.retry,
            Slot::Arm => &mut self.arm,
            Slot::Watch => &mut self.watch,
        }
    }

    fn cancel(&mut self, slot: Slot) {
        if let Some((_, h)) = self.slot(slot).take() {
            h.cancel();
        }
    }

    fn cancel_all(&mut self) {
        for slot in [Slot::Push, Slot::Pull, Slot::Retry, Slot::Arm, Slot::Watch] {
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
    /// The watcher of the current arm could not start (shown as `Error { watchFailed }`).
    watch_failed: bool,
    /// The delay before the next watcher retry.
    watch_backoff: Duration,
}

/// What one run came to.
enum Outcome {
    Synced,
    /// The vault was locked, closed or switched meanwhile: nothing to report.
    Stale,
    /// Synced, but the primary file could not be brought up to date, so the checkpoint was
    /// left as it was; retried.
    Deferred,
    Unavailable(SecondaryUnavailable),
    Failed(SyncError),
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct Probe {
    /// Engine calls made (a run that stops at the echo check makes none).
    pub(crate) engine_runs: std::sync::atomic::AtomicUsize,
    /// Runs between the engine call and the adoption of a merge.
    pub(crate) before_adopt: Hook,
    /// Runs inside the engine call, right before each remote write (outside the session lock).
    pub(crate) during_engine: Hook,
    /// Runs after the adoption, before the primary is flushed.
    pub(crate) before_durable: Hook,
    /// Runs after the primary is durable, before the checkpoint is committed.
    pub(crate) before_commit: Hook,
}

#[cfg(test)]
pub(crate) type Hook = Mutex<Option<Box<dyn FnMut() + Send>>>;

#[cfg(test)]
fn run_hook(hook: &Hook) {
    if let Some(f) = lock(hook).as_mut() {
        f();
    }
}

/// One request for the worker.
struct Request {
    kind: RunKind,
    epoch: u64,
    /// `sync_now` waits here for the status its run ends with (dropped unanswered when the
    /// request is discarded).
    reply: Option<mpsc::Sender<SecondaryStatus>>,
}

#[derive(Default)]
struct WorkState {
    queue: VecDeque<Request>,
    /// A run is in flight.
    running: bool,
    /// The worker thread, once started.
    thread: Option<ThreadId>,
    /// The worker stopped (or must stop); nothing is queued any more.
    shutdown: bool,
}

/// The queue between the triggers (scheduler jobs, `sync_now`) and the worker thread.
#[derive(Default)]
struct Worker {
    state: Mutex<WorkState>,
    /// Signalled when a request is queued or the worker must stop.
    wake: Condvar,
    /// Signalled when the worker goes idle (nothing queued, no run in flight) or stops.
    idle: Condvar,
}

impl Worker {
    fn stop(&self) {
        let mut w = lock(&self.state);
        w.shutdown = true;
        let dropped = std::mem::take(&mut w.queue);
        drop(w);
        // Unanswered replies tell their `sync_now` callers to stop waiting.
        drop(dropped);
        self.wake.notify_all();
        self.idle.notify_all();
    }
}

/// The worker thread: runs one request at a time until the service is gone or stopped. Holds
/// the service only while a run is in flight.
fn worker_loop(worker: &Worker, service: &Weak<Shared>) {
    loop {
        let request = {
            let mut w = lock(&worker.state);
            loop {
                if w.shutdown {
                    w.running = false;
                    worker.idle.notify_all();
                    return;
                }
                if let Some(request) = w.queue.pop_front() {
                    w.running = true;
                    break request;
                }
                w.running = false;
                worker.idle.notify_all();
                w = worker.wake.wait(w).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let Some(shared) = service.upgrade() else {
            worker.stop();
            return;
        };
        let me = SecondarySync(shared);
        let status = catch_unwind(AssertUnwindSafe(|| me.run(request.kind, request.epoch)));
        if let (Some(reply), Ok(status)) = (request.reply, status) {
            let _ = reply.send(status);
        }
        // Possibly the last handle: dropping it stops the worker (see `Drop for Shared`), and
        // must happen without the queue lock.
        drop(me);
        lock(&worker.state).running = false;
    }
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
    worker: Arc<Worker>,
    #[cfg(test)]
    probe: Probe,
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.worker.stop();
    }
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
                watch_failed: false,
                watch_backoff: SECONDARY_WATCH_RETRY_MIN,
            }),
            run_gate: Mutex::new(()),
            observers: RwLock::new(Vec::new()),
            attached: AtomicBool::new(false),
            worker: Arc::new(Worker::default()),
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
    ///
    /// The run happens on the worker after the requests queued before it; this call waits for
    /// its result (blocking: a command thread, never the main thread). A lock, vault switch or
    /// quit meanwhile ends that run early; a call from the worker itself (a status observer)
    /// answers the current status without waiting.
    pub fn sync_now(&self) -> Result<SecondaryStatus, ApiError> {
        if self.0.settings.get().secondary_folder.is_none() {
            self.rearm();
            return Ok(self.status());
        }
        self.0.session.with_db(|_| ())?;
        self.rearm();
        if self.on_worker_thread() {
            return Ok(self.status());
        }
        let epoch = lock(&self.0.state).epoch;
        let (reply, answer) = mpsc::channel();
        if !self.enqueue(RunKind::Now, epoch, Some(reply)) {
            return Ok(self.status());
        }
        Ok(answer.recv().unwrap_or_else(|_| self.status()))
    }

    /// Disarms and stops the worker, for the app's exit. Returns at once: a run in flight stops
    /// at its next check without writing (the quitting session refuses it anyway) and the worker
    /// thread then ends. Nothing is synced afterwards; [`SecondarySync::wait_idle`] waits for
    /// the worker with a bound.
    pub fn shutdown(&self) {
        let (old, changed) = {
            let mut st = lock(&self.0.state);
            st.epoch = st.epoch.wrapping_add(1);
            st.jobs.cancel_all();
            let old = st.armed.take();
            let status = SecondaryStatus::Idle {
                last_sync: old
                    .as_ref()
                    .and_then(|a| last_sync_for(&st, Some(&a.key.vault_id), &a.key.folder)),
            };
            let changed = old.is_some().then(|| set_status(&mut st, status)).flatten();
            (old, changed)
        };
        self.0.worker.stop();
        if let Some(watcher) = old.and_then(|a| a.watcher) {
            watcher.stop();
        }
        if let Some(status) = changed {
            self.notify(&status);
        }
    }

    /// Waits until nothing is queued and no run is in flight (or the worker stopped), at most
    /// `timeout`; `true` when it is idle. `false` at once on the worker thread itself.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        if self.on_worker_thread() {
            return false;
        }
        let w = lock(&self.0.worker.state);
        let (w, _) = self
            .0
            .worker
            .idle
            .wait_timeout_while(w, timeout, |w| {
                !(w.shutdown && !w.running || w.queue.is_empty() && !w.running)
            })
            .unwrap_or_else(PoisonError::into_inner);
        !w.running && (w.shutdown || w.queue.is_empty())
    }

    fn on_worker_thread(&self) -> bool {
        lock(&self.0.worker.state).thread == Some(std::thread::current().id())
    }

    /// Hands a run to the worker (starting it on first use) and returns at once; `false` when
    /// the request was not queued (stopped worker, superseded epoch, no thread). Requests of an
    /// earlier arm are dropped, and so is a duplicate of a request already queued.
    fn enqueue(
        &self,
        kind: RunKind,
        epoch: u64,
        reply: Option<mpsc::Sender<SecondaryStatus>>,
    ) -> bool {
        let current = lock(&self.0.state).epoch;
        if epoch != current {
            return false;
        }
        let started = {
            let mut w = lock(&self.0.worker.state);
            if w.shutdown {
                return false;
            }
            w.queue.retain(|r| r.epoch == current);
            let duplicate = reply.is_none()
                && w.queue
                    .iter()
                    .any(|r| r.kind == kind && r.epoch == epoch && r.reply.is_none());
            if !duplicate {
                w.queue.push_back(Request { kind, epoch, reply });
            }
            // Started under the queue lock, so two triggers never start two workers; the new
            // thread waits for this lock before it looks at the queue.
            let started = w.thread.is_some() || self.spawn_worker(&mut w);
            if started {
                self.0.worker.wake.notify_one();
            } else {
                w.queue.clear();
            }
            started
        };
        if !started {
            self.finish(
                epoch,
                Outcome::Failed(SyncError::Storage(StorageError::Io(
                    "the sync worker could not start".into(),
                ))),
            );
            return false;
        }
        true
    }

    /// Starts the worker thread (the caller holds the queue lock); `false` when the OS refused
    /// a thread.
    fn spawn_worker(&self, w: &mut WorkState) -> bool {
        let worker = self.0.worker.clone();
        let service = Arc::downgrade(&self.0);
        let spawned = std::thread::Builder::new()
            .name(SECONDARY_WORKER_THREAD.to_owned())
            .spawn(move || worker_loop(&worker, &service));
        match spawned {
            Ok(handle) => {
                w.thread = Some(handle.thread().id());
                true
            }
            Err(_) => false,
        }
    }

    /// Clears the secondary folder setting and stops syncing; the copy in the folder is left as
    /// it is. The current vault's checkpoint is cleared (best effort), so a later connect merges
    /// with whatever the folder then holds instead of writing over it. The setting goes first
    /// and disarms, so a run in flight stops at its next check instead of being waited out.
    pub fn disconnect(&self) -> Result<(), ApiError> {
        self.0.settings.set_secondary_folder(None)?;
        self.rearm();
        let _gate = lock(&self.0.run_gate);
        if let (Some(vault_id), Some(primary)) =
            (self.0.session.vault_id(), self.0.session.vault_path())
        {
            let path = self.checkpoint_path(&vault_id, &primary);
            if path.exists() {
                // Best effort: a stale checkpoint only allows writing over a copy that is exactly
                // what this device wrote last.
                let _ = FileCheckpointStore::new(path).save(None);
            }
        }
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
            st.watch_failed = false;
            st.watch_backoff = SECONDARY_WATCH_RETRY_MIN;
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

    fn checkpoint_path(&self, vault_id: &str, primary: &Path) -> PathBuf {
        self.0
            .sync_root
            .join(vault_id)
            .join(secondary_checkpoint_file(primary))
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
                    me.enqueue(kind, epoch, None);
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

    /// Starts the watcher of the current arm when none runs, and records whether one runs: a
    /// failure shows in the status and arms the watcher retry (with a growing delay).
    fn watch_folder(&self, epoch: u64, dir: &Path, file_name: &str) {
        let watching = self.ensure_watcher(epoch, dir, file_name);
        let mut st = lock(&self.0.state);
        if st.epoch != epoch {
            return;
        }
        if watching {
            st.watch_failed = false;
            st.watch_backoff = SECONDARY_WATCH_RETRY_MIN;
            st.jobs.cancel(Slot::Watch);
        } else {
            st.watch_failed = true;
            if st.jobs.watch.is_none() {
                let delay = st.watch_backoff;
                st.watch_backoff = delay.saturating_mul(2).min(SECONDARY_RETRY);
                self.schedule(&mut st, Slot::Watch, delay, RunKind::Watch);
            }
        }
    }

    /// Starts the watcher of the current arm when none runs. `false` only when it failed to
    /// start (a stale epoch or a running watcher is `true`: nothing to report).
    fn ensure_watcher(&self, epoch: u64, dir: &Path, file_name: &str) -> bool {
        {
            let st = lock(&self.0.state);
            match &st.armed {
                Some(armed) if st.epoch == epoch && armed.watcher.is_none() => {}
                _ => return true,
            }
        }
        let weak: Weak<Shared> = Arc::downgrade(&self.0);
        let callback: WatchCallback = Arc::new(move || {
            if let Some(shared) = weak.upgrade() {
                SecondarySync(shared).on_remote_change(epoch);
            }
        });
        let Ok(guard) = self.0.watch.watch(dir, file_name, callback) else {
            return false;
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
        true
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
        self.watch_folder(epoch, &dir, file_name);
        if matches!(kind, RunKind::Pull | RunKind::Watch) {
            if let Ok(Some(bytes)) = backend.read() {
                if echo.is_echo(&SyncCheckpoint::of_bytes(&bytes, None)) {
                    return self.settle_watch_status(epoch);
                }
            }
        }
        if let Some(status) = self.set_status_if(epoch, SecondaryStatus::Syncing) {
            self.notify(&status);
        }
        // The vault switched between the arm check and here: not this arm's vault.
        let snapshot = match self.0.session.sync_snapshot() {
            Ok(s) if s.vault_id == key.vault_id && s.path == key.primary => s,
            _ => return self.finish(epoch, Outcome::Stale),
        };
        #[cfg(test)]
        self.0.probe.engine_runs.fetch_add(1, Ordering::SeqCst);
        let checkpoints = Arc::new(FileCheckpointStore::new(
            self.checkpoint_path(&key.vault_id, &key.primary),
        ));
        let engine = SyncEngine::new(backend, checkpoints).with_echo_filter(echo);
        let still_current = || {
            #[cfg(test)]
            run_hook(&self.0.probe.during_engine);
            self.is_current(epoch, snapshot.incarnation)
        };
        // Outside the session lock: remote I/O and KDF runs never block the session.
        let synced = engine.sync_checked(
            &snapshot.db,
            &snapshot.password,
            kind.mode(),
            &still_current,
        );
        let outcome = match synced {
            Ok(result) => self.settle(&engine, &snapshot, result),
            Err(SyncError::Cancelled) => Outcome::Stale,
            Err(e) => Outcome::Failed(e),
        };
        drop(snapshot);
        self.finish(epoch, outcome)
    }

    /// Whether the arm of `epoch` and the session incarnation of the snapshot are both still
    /// current (and the app is not quitting). Never holds the state lock while asking the
    /// session.
    fn is_current(&self, epoch: u64, incarnation: u64) -> bool {
        let armed = {
            let st = lock(&self.0.state);
            st.epoch == epoch && st.armed.is_some()
        };
        armed && self.0.session.is_incarnation(incarnation)
    }

    /// Adopts a merge into the LIVE database (edits made while the engine ran are kept), makes
    /// the synced content durable in the primary file, and only then commits the checkpoint.
    /// A change of the open vault in between (lock, unlock, switch, relocation, quit) discards
    /// the result and commits nothing, so the next run merges again.
    fn settle(&self, engine: &SyncEngine, snapshot: &SyncSnapshot, result: SyncResult) -> Outcome {
        let (merged, pending) = result.into_parts();
        if let Some(remote) = &merged {
            #[cfg(test)]
            run_hook(&self.0.probe.before_adopt);
            let adopted =
                self.0
                    .session
                    .mutate_if_incarnation(snapshot.incarnation, |live, _now| {
                        let next = merge(live, remote);
                        if same_content(&next, live) {
                            return Ok(((), false));
                        }
                        *live = next;
                        Ok(((), true))
                    });
            if !matches!(adopted, Ok(Some(()))) {
                return Outcome::Stale;
            }
        }
        let Some(pending) = pending else {
            return Outcome::Synced;
        };
        #[cfg(test)]
        run_hook(&self.0.probe.before_durable);
        let content = merged.as_ref().unwrap_or(&snapshot.db);
        match self.make_durable(snapshot.incarnation, content) {
            Durability::Durable => {}
            Durability::Stale => return Outcome::Stale,
            Durability::Unsaved | Durability::Missing => return Outcome::Deferred,
        }
        #[cfg(test)]
        run_hook(&self.0.probe.before_commit);
        match engine.commit(pending) {
            Ok(()) => Outcome::Synced,
            Err(e) => Outcome::Failed(e),
        }
    }

    /// Flushes the primary until `content` is on disk, a few times while edits keep arriving.
    fn make_durable(&self, incarnation: u64, content: &Database) -> Durability {
        let mut found = Durability::Unsaved;
        for _ in 0..DURABLE_TRIES {
            if !self.0.session.is_incarnation(incarnation) {
                return Durability::Stale;
            }
            if self.0.session.flush().is_err() {
                // The save status shows the primary's failure; the checkpoint waits.
                return match self.0.session.durably_holds(incarnation, content) {
                    Durability::Stale => Durability::Stale,
                    _ => Durability::Unsaved,
                };
            }
            found = self.0.session.durably_holds(incarnation, content);
            if found != Durability::Unsaved {
                return found;
            }
        }
        found
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
            let deferred = matches!(outcome, Outcome::Deferred);
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
                Outcome::Stale | Outcome::Deferred => SecondaryStatus::Idle {
                    last_sync: last_sync_for(&st, Some(&key.vault_id), &key.folder),
                },
                Outcome::Unavailable(reason) => SecondaryStatus::Unavailable { reason },
                Outcome::Failed(e) => failure_status(&e),
            };
            // A healthy-looking run never hides a folder that is not watched.
            let status = match status {
                SecondaryStatus::Idle { .. } if st.watch_failed => SecondaryStatus::Error {
                    code: SecondaryError::WatchFailed,
                },
                other => other,
            };
            if deferred || is_transient(&status) {
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

    /// A run that made no engine call (an echo) leaves the status alone, except that a watcher
    /// that runs again clears `watchFailed`.
    fn settle_watch_status(&self, epoch: u64) -> SecondaryStatus {
        let changed = {
            let mut st = lock(&self.0.state);
            let recovered = st.epoch == epoch
                && !st.watch_failed
                && st.status
                    == (SecondaryStatus::Error {
                        code: SecondaryError::WatchFailed,
                    });
            match st.armed.as_ref().map(|a| a.key.clone()) {
                Some(key) if recovered => {
                    let status = SecondaryStatus::Idle {
                        last_sync: last_sync_for(&st, Some(&key.vault_id), &key.folder),
                    };
                    set_status(&mut st, status)
                }
                _ => None,
            }
        };
        if let Some(status) = &changed {
            self.notify(status);
        }
        self.status()
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
        SyncError::RemoteChangedDuringSync | SyncError::ConcurrentCopyKept => {
            SecondaryError::RemoteBusy
        }
        // `Cancelled` never reaches here (a run maps it to a stale outcome).
        SyncError::Checkpoint(_) | SyncError::Encode(_) | SyncError::Cancelled => {
            SecondaryError::Io
        }
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
    let meta = fs::metadata(path).map_err(folder_error)?;
    if !meta.is_dir() {
        return Err(SecondaryUnavailable::InvalidFolder);
    }
    let dir = fs::canonicalize(path).map_err(folder_error)?;
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

/// Only a path that does not exist is a missing folder (retried: an unmounted drive comes back);
/// anything else that keeps it from being inspected (a symlink loop, no permission) is invalid.
fn folder_error(e: std::io::Error) -> SecondaryUnavailable {
    if e.kind() == ErrorKind::NotFound {
        SecondaryUnavailable::FolderMissing
    } else {
        SecondaryUnavailable::InvalidFolder
    }
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
