//! The save pipeline: debounced autosave, `flush`, `lock`, the concurrent-write merge and the
//! encrypted edits kept after a failed save on lock.
//!
//! # Guarantees
//!
//! - **An edit is never dropped from memory while the app runs.** [`VaultSession::flush`] is
//!   `Ok` only when `saved_gen` has reached `dirty_gen` (every edit is on disk). A failed write
//!   leaves the session dirty, shows `SaveState::Failed` with the message and schedules a retry
//!   after [`PENDING_RETRY`].
//! - **No external change is overwritten or dropped.** Every write goes through the guarded
//!   `VaultFile::save`. When the file changed on disk (another device synced, KeePassXC saved),
//!   the save reloads it and merges it into the LIVE session database with
//!   `mypass_core::merge::merge` (edits made while the write ran are included), emits
//!   `VaultChanged`, and writes the merge (up to [`MAX_SAVE_ROUNDS`] rounds). The merge result is
//!   kept in the session and reaches disk: this is the fix for the old app's bug that discarded
//!   the merged result (`SyncResult.merged`) until the next unlock.
//! - **Locking keeps unsaved edits.** `lock` flushes first. If that fails, the database is
//!   serialised (encrypted with the master password, so safe to hold) into `pending_bytes`
//!   before it is dropped. A timer retries writing them every [`PENDING_RETRY`], but only while
//!   the file on disk is unchanged since the session last read or wrote it. Otherwise they wait
//!   for the next `unlock`, which decrypts them with the supplied password, merges them with the
//!   vault on disk and saves.
//!
//! # Ordering between writers
//!
//! A write runs OUTSIDE the session lock (so `status()` and reads stay responsive during the
//! KDF), with `Inner::in_flight` set and the `VaultFile` taken out of the session. Every path
//! that writes or replaces the file (`flush`, the autosave job, the retry jobs, `lock`,
//! `check_external_change`, `unlock`, opening another vault, `relocate`) first waits on the
//! `Shared::idle` condvar until no write is in flight, so two writes never overlap and never
//! land out of order. A scheduled job that finds a write in flight returns at once: the running
//! write reschedules the autosave when edits arrived meanwhile, and `flush` loops until nothing
//! is dirty. Scheduled jobs hold only a weak reference to the session and re-check its state
//! when they run (a cancelled job may still run once).
//!
//! `lock` waits for an in-flight write (one KDF plus a write) before it drops the database, so
//! callers on a UI or event thread must not call it directly from a platform event sink:
//! `AutoLock` hands system events to the scheduler for that reason. If the save on lock fails,
//! the re-encryption into `pending_bytes` runs under the session lock (one KDF); `status()`
//! blocks for that time, which only happens on that failure path.
//!
//! # Recovery when the kept edits cannot be merged
//!
//! The kept edits are encrypted with the master password the session had. If the password was
//! changed elsewhere meanwhile, the disk opens with the new password and the edits only with the
//! old one, so the unlock fails with `WrongPassword` with either password and the edits stay
//! kept. The user has two ways out:
//!
//! 1. Make the file open with the old password again (revert the change on the other device or
//!    restore that file version): the next unlock merges the edits and saves them.
//! 2. Open (or create) another vault, or re-open this one, from the vault picker. Before it
//!    switches, the session writes the kept edits as `unsaved-<unix_secs>.kdbx` into this
//!    vault's backup folder (the folder "Reveal backups" opens), still encrypted with the OLD
//!    password, and only then lets them go. That file opens in MyPass or KeePassXC with the old
//!    password, so the edits can be copied into the vault. Backup rotation never deletes
//!    `unsaved-*` files. If that copy cannot be written, the switch is refused and the edits stay
//!    kept.
//!
//! Kept edits otherwise live only in memory: `flush` (and `flush_blocking` on the quit path)
//! returns an error while they cannot be written, so the quit flow can tell the user.

use crate::scheduler::lock;
use crate::session::{
    ConflictEvent, Inner, LockReason, Outbox, SaveStatus, SessionError, Shared, VaultSession,
};
use mypass_core::merge::merge;
use mypass_kdbx::{Database, FileIo, KdbxError, SaveOutcome, VaultFile};
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, MutexGuard, Weak};
use std::time::Duration;

/// Delay between the last edit and the autosave.
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(1);
/// Delay before a failed write (or the kept edits after a failed save on lock) is retried.
pub const PENDING_RETRY: Duration = Duration::from_secs(30);
/// How often one save may find the file changed on disk, merge and try again.
pub const MAX_SAVE_ROUNDS: usize = 3;
/// Prefix of the copy of kept edits written before switching vaults (never rotated).
pub const UNSAVED_COPY_PREFIX: &str = "unsaved-";

/// What [`VaultSession::check_external_change`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalChange {
    /// The file is as the session last read or wrote it.
    None,
    /// Remote content was merged into the session (the vault changed in memory).
    Merged,
    /// The file changed on disk but holds nothing new (stamp refreshed only).
    Reloaded,
}

/// Who runs a save.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Caller {
    /// A scheduled job: returns when another write is in flight, reschedules leftovers.
    Job,
    /// `flush`: waits for an in-flight write, loops until nothing is dirty.
    Flush,
}

/// One write taken out of the session for the duration of the I/O.
struct Work {
    file: VaultFile,
    db: Database,
    password: SecretString,
    gen: u64,
    vault_id: Option<String>,
    first_write: bool,
    fresh_file: bool,
}

enum WriteResult {
    Saved(u64),
    Remote(Box<Database>),
    Failed(SessionError),
}

enum Step {
    Done,
    Again,
    Fail(SessionError),
}

/// Whether `merged` (a merge output) holds the same content as `other`. The merge sorts children
/// and tombstones by UUID and normalises history, so `other` is compared in that canonical form
/// (`merge(other, other)`), never with a plain `==` that would see a different child order as a
/// change.
pub(crate) fn same_content(merged: &Database, other: &Database) -> bool {
    *merged == merge(other, other)
}

fn wrong_password_message() -> String {
    "the vault on disk no longer opens with the current password".to_owned()
}

impl VaultSession {
    fn weak(&self) -> Weak<Shared> {
        Arc::downgrade(&self.0)
    }

    /// Waits (releasing the lock) until no write or check is in flight.
    pub(crate) fn wait_idle<'a>(&self, mut g: MutexGuard<'a, Inner>) -> MutexGuard<'a, Inner> {
        while g.in_flight {
            g = self
                .0
                .idle
                .wait(g)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        g
    }

    fn schedule_job(&self, inner: &mut Inner, after: Duration, pending: bool) {
        inner.cancel_save_job();
        let weak = self.weak();
        let job = Box::new(move || {
            if let Some(shared) = weak.upgrade() {
                let session = VaultSession(shared);
                if pending {
                    let _ = session.retry_pending();
                } else {
                    let _ = session.run_save(Caller::Job);
                }
            }
        });
        inner.save_job = Some(self.0.deps.scheduler.schedule(after, job));
    }

    /// (Re)schedules the autosave [`AUTOSAVE_DEBOUNCE`] from now, cancelling the previous one.
    pub(crate) fn schedule_autosave(&self, inner: &mut Inner) {
        self.schedule_job(inner, AUTOSAVE_DEBOUNCE, false);
    }

    /// Takes the next write out of the session, or `None` when there is nothing to write.
    fn take_work(inner: &mut Inner, out: &mut Outbox) -> Option<Work> {
        if !inner.is_dirty() {
            return None;
        }
        let (Some(db), Some(password)) = (&inner.db, &inner.password) else {
            return None;
        };
        let (db, password) = (db.clone(), password.clone());
        let file = inner.file.take()?;
        inner.cancel_save_job();
        inner.in_flight = true;
        out.save(inner.set_save(SaveStatus::saving()));
        Some(Work {
            file,
            db,
            password,
            gen: inner.dirty_gen,
            vault_id: inner.vault_id.clone(),
            first_write: std::mem::take(&mut inner.first_write_of_session),
            fresh_file: std::mem::take(&mut inner.fresh_file),
        })
    }

    /// Snapshot, then one guarded write; on `ChangedOnDisk` the remote content is reloaded.
    fn write_work(&self, w: &mut Work) -> WriteResult {
        if let (false, Some(id)) = (w.fresh_file, &w.vault_id) {
            self.0
                .backups
                .snapshot_before_save(id, w.file.path(), self.now().0, w.first_write);
        }
        match w.file.save(&w.db, &w.password) {
            Ok(SaveOutcome::Saved) => WriteResult::Saved(w.gen),
            Ok(SaveOutcome::ChangedOnDisk) => match w.file.reload(&w.password) {
                Ok(remote) => WriteResult::Remote(Box::new(remote)),
                Err(e) => WriteResult::Failed(e.into()),
            },
            Err(e) => WriteResult::Failed(e.into()),
        }
    }

    /// The core save loop shared by the autosave job, the retry job and `flush`.
    fn run_save(&self, caller: Caller) -> Result<(), SessionError> {
        let mut rounds = 0usize;
        loop {
            let mut out = Outbox::default();
            let work = {
                let mut inner = lock(&self.0.inner);
                if caller == Caller::Flush {
                    inner = self.wait_idle(inner);
                } else if inner.in_flight {
                    return Ok(());
                }
                Self::take_work(&mut inner, &mut out)
            };
            self.emit(out);
            let Some(mut work) = work else {
                return Ok(());
            };
            let result = self.write_work(&mut work);
            let mut out = Outbox::default();
            let mut hook = None;
            let step = {
                let mut inner = lock(&self.0.inner);
                inner.in_flight = false;
                inner.file = Some(work.file);
                self.0.idle.notify_all();
                let step = match result {
                    WriteResult::Saved(gen) => {
                        inner.saved_gen = gen;
                        if inner.is_dirty() {
                            out.save(inner.set_save(SaveStatus::pending()));
                            if caller == Caller::Job {
                                self.schedule_autosave(&mut inner);
                                Step::Done
                            } else {
                                Step::Again
                            }
                        } else {
                            out.save(inner.set_save(SaveStatus::idle()));
                            Step::Done
                        }
                    }
                    WriteResult::Remote(remote) => {
                        rounds += 1;
                        hook = Some((inner.conflict_hook.clone(), ConflictEvent::MergedOnSave));
                        self.apply_remote(&mut inner, &remote, &mut out);
                        if !inner.is_dirty() {
                            out.save(inner.set_save(SaveStatus::idle()));
                            Step::Done
                        } else if rounds >= MAX_SAVE_ROUNDS {
                            Step::Fail(SessionError::ChangedOnDisk)
                        } else {
                            Step::Again
                        }
                    }
                    WriteResult::Failed(e) => Step::Fail(e),
                };
                if let Step::Fail(e) = &step {
                    let message = match e {
                        SessionError::WrongPassword => wrong_password_message(),
                        other => other.to_string(),
                    };
                    out.save(inner.set_save(SaveStatus::failed(message)));
                    if inner.db.is_some() {
                        self.schedule_job(&mut inner, PENDING_RETRY, false);
                    }
                }
                step
            };
            self.emit(out);
            if let Some((h, ev)) = hook {
                h(ev);
            }
            match step {
                Step::Done => return Ok(()),
                Step::Again => continue,
                Step::Fail(e) => return Err(e),
            }
        }
    }

    /// Merges `remote` into the live database. The session stays dirty (with an autosave
    /// scheduled) when the merge differs from `remote`, and becomes clean when it equals it.
    /// Returns whether the live content changed.
    fn apply_remote(&self, inner: &mut Inner, remote: &Database, out: &mut Outbox) -> bool {
        let Some(live) = &inner.db else {
            return false;
        };
        let merged = merge(live, remote);
        let changed = !same_content(&merged, live);
        let on_disk = same_content(&merged, remote);
        if changed {
            inner.db = Some(merged);
            out.vault_changed = true;
        }
        if on_disk {
            inner.saved_gen = inner.dirty_gen;
        } else if changed || !inner.is_dirty() {
            inner.dirty_gen = inner.dirty_gen.wrapping_add(1);
        }
        changed
    }

    /// Writes unsaved edits now. `Ok` ONLY when everything is on disk: if a background write is
    /// in flight it waits for it, and when edits arrived meanwhile it saves again. A failed write
    /// is `Err` and the session stays dirty (`Failed`, retry scheduled). While locked with kept
    /// edits it tries to write them (only over an unchanged file). Works while quitting.
    pub fn flush(&self) -> Result<(), SessionError> {
        let unlocked = {
            let inner = self.wait_idle(lock(&self.0.inner));
            if inner.db.is_none() && inner.pending_bytes.is_none() {
                return Ok(());
            }
            inner.db.is_some()
        };
        if unlocked {
            return self.run_save(Caller::Flush);
        }
        self.retry_pending()
    }

    /// [`VaultSession::flush`] with a deadline (the quit path uses 5 s). On expiry it returns
    /// `Err(Io("flush timed out"))` and the session stays dirty; the write keeps running on a
    /// helper thread and completes (or fails) on its own.
    pub fn flush_blocking(&self, timeout: Duration) -> Result<(), SessionError> {
        let (tx, rx) = mpsc::channel();
        let session = self.clone();
        let spawned = std::thread::Builder::new()
            .name("mypass-flush".into())
            .spawn(move || {
                let _ = tx.send(session.flush());
            });
        if let Err(e) = spawned {
            return Err(SessionError::Io(e.to_string()));
        }
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => Err(SessionError::Io("flush timed out".into())),
        }
    }

    /// Saves pending edits, then drops the database and the password (zeroized) and keeps the
    /// path. If the save fails, the encrypted bytes are kept (see the module docs). Works while
    /// quitting.
    pub fn lock(&self, reason: LockReason) {
        {
            let mut inner = lock(&self.0.inner);
            if inner.db.is_none() {
                return;
            }
            inner.cancel_save_job();
        }
        let _ = self.flush();
        let mut out = Outbox::default();
        {
            let mut inner = self.wait_idle(lock(&self.0.inner));
            if inner.db.is_none() {
                return;
            }
            inner.cancel_save_job();
            if inner.is_dirty() {
                if let (Some(db), Some(password)) = (&inner.db, &inner.password) {
                    // Only fails when the OS has no randomness left; nothing more can be kept.
                    inner.pending_bytes = db.save(password).ok();
                }
                if inner.pending_bytes.is_some() {
                    inner.pending_file = inner.file.take();
                    self.schedule_job(&mut inner, PENDING_RETRY, true);
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

    /// Writes the kept edits while locked, only over an unchanged file. `Ok` when nothing is
    /// kept any more.
    fn retry_pending(&self) -> Result<(), SessionError> {
        let (bytes, file, vault_id) = {
            let mut inner = self.wait_idle(lock(&self.0.inner));
            if inner.db.is_some() {
                return Ok(());
            }
            let Some(bytes) = inner.pending_bytes.clone() else {
                return Ok(());
            };
            let Some(file) = inner.pending_file.take() else {
                return Err(SessionError::Io(
                    "unsaved changes are kept until the next unlock".into(),
                ));
            };
            inner.in_flight = true;
            (bytes, file, inner.vault_id.clone())
        };
        let result = match file.has_changed_on_disk() {
            Ok(false) => {
                if let Some(id) = &vault_id {
                    self.0
                        .backups
                        .snapshot_before_save(id, file.path(), self.now().0, true);
                }
                self.0
                    .io
                    .write_atomic(file.path(), &bytes)
                    .map(|()| true)
                    .map_err(SessionError::from)
            }
            Ok(true) => Ok(false),
            Err(e) => Err(e.into()),
        };
        let mut out = Outbox::default();
        let r = {
            let mut inner = lock(&self.0.inner);
            inner.in_flight = false;
            self.0.idle.notify_all();
            match result {
                Ok(true) => {
                    if inner.pending_bytes.as_ref() == Some(&bytes) {
                        inner.pending_bytes = None;
                        inner.saved_gen = inner.dirty_gen;
                        out.save(inner.set_save(SaveStatus::idle()));
                    }
                    Ok(())
                }
                Ok(false) => {
                    // The file changed on disk: the next unlock merges the kept edits.
                    inner.pending_file = Some(file);
                    Err(SessionError::ChangedOnDisk)
                }
                Err(e) => {
                    inner.pending_file = Some(file);
                    out.save(inner.set_save(SaveStatus::failed(e.to_string())));
                    self.schedule_job(&mut inner, PENDING_RETRY, true);
                    Err(e)
                }
            }
        };
        self.emit(out);
        r
    }

    /// Before another vault replaces this one: saves the live edits, or (while locked with kept
    /// edits that cannot be written) writes them as an `unsaved-*` copy into this vault's backup
    /// folder. `Err` means the switch must not happen.
    pub(crate) fn prepare_switch(&self) -> Result<(), SessionError> {
        let (unlocked, kept) = {
            let inner = self.wait_idle(lock(&self.0.inner));
            (inner.db.is_some(), inner.pending_bytes.is_some())
        };
        if unlocked {
            return self.flush();
        }
        if !kept || self.retry_pending().is_ok() {
            return Ok(());
        }
        self.keep_unsaved_copy().map(|_| ())
    }

    /// Writes the kept edits to `<backups>/<vault_id>/unsaved-<secs>[-<n>].kdbx` and lets them
    /// go from memory. Returns the copy's path.
    fn keep_unsaved_copy(&self) -> Result<PathBuf, SessionError> {
        let (bytes, vault_id) = {
            let inner = self.wait_idle(lock(&self.0.inner));
            match (&inner.pending_bytes, &inner.vault_id) {
                (Some(b), Some(id)) => (b.clone(), id.clone()),
                (None, _) => return Err(SessionError::NotFound),
                (Some(_), None) => {
                    return Err(SessionError::InvalidInput(
                        "the open vault has unsaved changes; unlock it to save them first".into(),
                    ))
                }
            }
        };
        let folder = self.0.backups.ensure_folder(&vault_id)?;
        let secs = self.now().0.max(0);
        let mut path = folder.join(format!("{UNSAVED_COPY_PREFIX}{secs}.kdbx"));
        let mut n = 1u32;
        while std::fs::symlink_metadata(&path).is_ok() {
            path = folder.join(format!("{UNSAVED_COPY_PREFIX}{secs}-{n}.kdbx"));
            n += 1;
            if n > 99 {
                return Err(SessionError::AlreadyExists);
            }
        }
        mypass_kdbx::write_atomic(&path, &bytes)?;
        let mut inner = lock(&self.0.inner);
        if inner.pending_bytes.as_ref() == Some(&bytes) {
            inner.pending_bytes = None;
            inner.pending_file = None;
            inner.cancel_save_job();
        }
        Ok(path)
    }

    /// Checks whether the vault changed on disk and, if so, merges the remote content into the
    /// session (called on window focus and every 30 s while unlocked). A reload that fails with
    /// `WrongPassword` (the password was changed elsewhere) is returned as such and leaves the
    /// session untouched.
    pub fn check_external_change(&self) -> Result<ExternalChange, SessionError> {
        let (mut file, password) = {
            let mut inner = self.wait_idle(lock(&self.0.inner));
            let Some(password) = inner.password.clone() else {
                return Err(inner.not_unlocked());
            };
            let Some(file) = inner.file.take() else {
                return Err(inner.not_unlocked());
            };
            inner.in_flight = true;
            (file, password)
        };
        let result = match file.has_changed_on_disk() {
            Ok(false) => Ok(None),
            Ok(true) => file.reload(&password).map(Some),
            Err(e) => Err(e),
        };
        let mut out = Outbox::default();
        let mut hook = None;
        let r = {
            let mut inner = lock(&self.0.inner);
            inner.in_flight = false;
            inner.file = Some(file);
            self.0.idle.notify_all();
            match result {
                Err(KdbxError::WrongPassword) => Err(SessionError::WrongPassword),
                Err(e) => Err(e.into()),
                Ok(None) => Ok(ExternalChange::None),
                Ok(Some(remote)) => {
                    let was_dirty = inner.is_dirty();
                    let changed = self.apply_remote(&mut inner, &remote, &mut out);
                    if inner.is_dirty() {
                        if !was_dirty || changed {
                            out.save(inner.set_save(SaveStatus::pending()));
                        }
                        self.schedule_autosave(&mut inner);
                    } else {
                        inner.cancel_save_job();
                        out.save(inner.set_save(SaveStatus::idle()));
                    }
                    let ev = if changed {
                        ConflictEvent::MergedOnCheck
                    } else {
                        ConflictEvent::ReloadedOnCheck
                    };
                    hook = Some((inner.conflict_hook.clone(), ev));
                    Ok(if changed {
                        ExternalChange::Merged
                    } else {
                        ExternalChange::Reloaded
                    })
                }
            }
        };
        self.emit(out);
        if let Some((h, ev)) = hook {
            h(ev);
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{ManualScheduler, Scheduler, ThreadScheduler};
    use crate::session::{SaveState, SessionDeps, SessionEvent, SessionState};
    use crate::settings::{BackupInterval, SettingsPatch, SettingsStore};
    use crate::testing::fast_database;
    use mypass_kdbx::test_util::FailingFileIo;
    use mypass_kdbx::{Entry, FileMeta, PlainFileIo, Timestamp};
    use std::fs;
    use std::io::ErrorKind;
    use std::path::Path;
    use std::sync::{Condvar, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    const T0: i64 = 1_700_000_000;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    /// Test-only IO that blocks the next `write_atomic` after `arm` until `release`.
    #[derive(Default)]
    struct Gate {
        armed: bool,
        entered: bool,
        open: bool,
    }

    #[derive(Default)]
    struct GateIo {
        gate: Mutex<Gate>,
        cv: Condvar,
    }

    impl GateIo {
        fn arm(&self) {
            *self.gate.lock().unwrap() = Gate {
                armed: true,
                ..Gate::default()
            };
        }
        fn wait_entered(&self) {
            let g = self.gate.lock().unwrap();
            let (g, _) = self
                .cv
                .wait_timeout_while(g, Duration::from_secs(10), |g| !g.entered)
                .unwrap();
            assert!(g.entered, "the gated write never started");
        }
        fn release(&self) {
            self.gate.lock().unwrap().open = true;
            self.cv.notify_all();
        }
    }

    impl FileIo for GateIo {
        fn read(&self, path: &Path) -> mypass_kdbx::Result<Vec<u8>> {
            PlainFileIo.read(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> mypass_kdbx::Result<()> {
            {
                let mut g = self.gate.lock().unwrap();
                if g.armed {
                    g.armed = false;
                    g.entered = true;
                    self.cv.notify_all();
                    let _g = self.cv.wait_while(g, |g| !g.open).unwrap();
                }
            }
            PlainFileIo.write_atomic(path, bytes)
        }
        fn metadata(&self, path: &Path) -> mypass_kdbx::Result<FileMeta> {
            PlainFileIo.metadata(path)
        }
    }

    struct Fx {
        dir: TempDir,
        session: VaultSession,
        settings: SettingsStore,
        clock: Arc<ManualScheduler>,
        _thread: Option<Arc<ThreadScheduler>>,
        path: PathBuf,
    }

    fn start() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(T0 as u64)
    }

    /// A vault `v.kdbx` (password "pw", one entry "first") written outside the session, then
    /// opened through `io`.
    fn setup_with(io: Arc<dyn FileIo>, threaded: bool) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        let mut db = fast_database("Test Vault");
        let mut e = Entry::new(Timestamp(T0 - 1000));
        e.set_field("Title", "first", false);
        let root = db.root_id();
        db.add_entry(root, e, Timestamp(T0 - 1000)).unwrap();
        fs::write(&path, db.save(&pw("pw")).unwrap()).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let settings = SettingsStore::in_memory();
        let clock = ManualScheduler::new(start());
        let thread = threaded.then(|| ThreadScheduler::new("test-scheduler").unwrap());
        let scheduler: Arc<dyn Scheduler> = match &thread {
            Some(t) => t.clone(),
            None => clock.clone(),
        };
        let session = VaultSession::new(SessionDeps {
            settings: settings.clone(),
            scheduler,
            backups_root: dir.path().join("backups"),
            file_io: io,
        });
        session.open_vault(&path, pw("pw")).unwrap();
        Fx {
            dir,
            session,
            settings,
            clock,
            _thread: thread,
            path,
        }
    }

    fn setup() -> Fx {
        setup_with(Arc::new(PlainFileIo), false)
    }

    fn failing_setup() -> (Fx, Arc<FailingFileIo>) {
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        (setup_with(io.clone(), false), io)
    }

    fn titles(db: &Database) -> Vec<String> {
        let mut t: Vec<String> = db
            .root
            .entries
            .iter()
            .map(|e| e.field("Title").unwrap_or_default().to_owned())
            .collect();
        t.sort();
        t
    }

    fn disk_titles(path: &Path, password: &str) -> Vec<String> {
        titles(&VaultFile::open(path, &pw(password)).unwrap().1)
    }

    /// Another writer (a second device, KeePassXC) adds an entry through its own handle.
    fn external_add(path: &Path, title: &str, at: i64) {
        let (mut file, mut db) = VaultFile::open(path, &pw("pw")).unwrap();
        let mut e = Entry::new(Timestamp(at));
        e.set_field("Title", title, false);
        let root = db.root_id();
        db.add_entry(root, e, Timestamp(at)).unwrap();
        assert_eq!(file.save(&db, &pw("pw")).unwrap(), SaveOutcome::Saved);
    }

    fn record(session: &VaultSession) -> Arc<Mutex<Vec<SessionEvent>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        session.add_observer(Arc::new(move |e: &SessionEvent| {
            l.lock().unwrap().push(e.clone())
        }));
        log
    }

    impl Fx {
        fn add(&self, title: &str) -> Result<(), SessionError> {
            self.session.mutate(|db, now| {
                let mut e = Entry::new(now);
                e.set_field("Title", title, false);
                let root = db.root_id();
                db.add_entry(root, e, now)?;
                Ok(())
            })
        }
        fn titles(&self) -> Vec<String> {
            self.session.with_db(titles).unwrap()
        }
        fn disk(&self) -> Vec<String> {
            disk_titles(&self.path, "pw")
        }
        fn state(&self) -> SaveState {
            self.session.status().save.state
        }
        fn gens(&self) -> (u64, u64) {
            let inner = lock(&self.session.0.inner);
            (inner.saved_gen, inner.dirty_gen)
        }
        fn has_pending_bytes(&self) -> bool {
            lock(&self.session.0.inner).pending_bytes.is_some()
        }
        fn backups(&self) -> usize {
            let id = self.session.vault_id().unwrap();
            self.session.0.backups.list(&id).len()
        }
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn mutation_schedules_save_after_debounce_not_before() {
        let f = setup();
        let before = fs::read(&f.path).unwrap();
        f.add("A").unwrap();
        f.clock.advance(Duration::from_millis(900));
        assert_eq!(fs::read(&f.path).unwrap(), before);
        assert_eq!(f.state(), SaveState::Pending);
        f.clock.advance(Duration::from_millis(100));
        assert_eq!(f.disk(), v(&["A", "first"]));
        assert_eq!(f.state(), SaveState::Idle);
    }

    #[test]
    fn rapid_mutations_coalesce_into_one_write() {
        let (f, io) = failing_setup();
        let writes = io.write_calls();
        for i in 0..5 {
            f.add(&format!("e{i}")).unwrap();
            f.clock.advance(Duration::from_millis(200));
        }
        assert_eq!(io.write_calls(), writes);
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        assert_eq!(io.write_calls(), writes + 1);
        assert_eq!(f.backups(), 1, "exactly one snapshot");
        assert_eq!(f.disk().len(), 6);
    }

    #[test]
    fn lock_flushes_dirty_database_before_dropping_it() {
        let f = setup();
        f.add("A").unwrap();
        f.session.lock(LockReason::Manual);
        assert_eq!(f.session.status().state, SessionState::Locked);
        assert!(!f.has_pending_bytes());
        let other = setup();
        other.session.open_vault(&f.path, pw("pw")).unwrap();
        assert_eq!(other.titles(), v(&["A", "first"]));
    }

    #[test]
    fn unlock_after_lock_sees_saved_edit() {
        let f = setup();
        f.add("A").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        f.session.lock(LockReason::Manual);
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.titles(), v(&["A", "first"]));
        assert_eq!(f.state(), SaveState::Idle);
    }

    #[test]
    fn external_change_is_merged_not_overwritten() {
        let f = setup();
        f.add("A").unwrap();
        external_add(&f.path, "B", T0 + 100);
        f.session.flush().unwrap();
        assert_eq!(f.disk(), v(&["A", "B", "first"]));
        assert_eq!(f.titles(), v(&["A", "B", "first"]));
        assert_eq!(f.gens().0, f.gens().1);
    }

    #[test]
    fn background_merge_result_is_kept_and_saved() {
        let f = setup();
        let log = record(&f.session);
        external_add(&f.path, "B", T0 + 100);
        f.add("A").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        // The old bug: the merged result was dropped until the next unlock.
        assert_eq!(f.titles(), v(&["A", "B", "first"]));
        assert_eq!(f.disk(), v(&["A", "B", "first"]));
        assert_eq!(f.state(), SaveState::Idle);
        let vault_changes = log
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == SessionEvent::VaultChanged)
            .count();
        assert_eq!(vault_changes, 2, "the local edit and the merge");
    }

    #[test]
    fn external_edit_of_same_entry_newer_wins_loser_in_history() {
        let f = setup();
        let id = f.session.with_db(|db| db.root.entries[0].uuid).unwrap();
        f.session
            .mutate(|db, now| {
                db.update_entry(id, now, |e| e.set_field("Title", "local", false))?;
                Ok(())
            })
            .unwrap();
        {
            let (mut file, mut db) = VaultFile::open(&f.path, &pw("pw")).unwrap();
            db.update_entry(id, Timestamp(T0 + 100), |e| {
                e.set_field("Title", "remote", false)
            })
            .unwrap();
            file.save(&db, &pw("pw")).unwrap();
        }
        f.session.flush().unwrap();
        for db in [
            f.session.with_db(Clone::clone).unwrap(),
            VaultFile::open(&f.path, &pw("pw")).unwrap().1,
        ] {
            let e = db.find_entry(id).unwrap();
            assert_eq!(e.field("Title"), Some("remote"));
            let history: Vec<_> = e.history.iter().filter_map(|h| h.field("Title")).collect();
            assert!(history.contains(&"local"), "{history:?}");
            assert!(history.contains(&"first"), "{history:?}");
        }
    }

    #[test]
    fn check_external_change_reports_none_when_unchanged() {
        let f = setup();
        assert_eq!(f.session.check_external_change(), Ok(ExternalChange::None));
    }

    #[test]
    fn check_external_change_merges_without_local_edits() {
        let f = setup();
        let log = record(&f.session);
        external_add(&f.path, "B", T0 + 100);
        assert_eq!(
            f.session.check_external_change(),
            Ok(ExternalChange::Merged)
        );
        assert!(log.lock().unwrap().contains(&SessionEvent::VaultChanged));
        assert_eq!(f.titles(), v(&["B", "first"]));
        assert_eq!(
            f.state(),
            SaveState::Idle,
            "the disk already holds the merge"
        );
        assert_eq!(f.clock.pending(), 0);
        assert_eq!(f.session.check_external_change(), Ok(ExternalChange::None));
    }

    #[test]
    fn check_external_change_with_local_edits_schedules_save() {
        let f = setup();
        f.add("A").unwrap();
        external_add(&f.path, "B", T0 + 100);
        assert_eq!(
            f.session.check_external_change(),
            Ok(ExternalChange::Merged)
        );
        assert_eq!(f.state(), SaveState::Pending);
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        assert_eq!(f.disk(), v(&["A", "B", "first"]));
        assert_eq!(f.state(), SaveState::Idle);
    }

    #[test]
    fn check_external_change_touch_only_is_reloaded() {
        let f = setup();
        let log = record(&f.session);
        let file = fs::File::options().write(true).open(&f.path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(3600))
            .unwrap();
        drop(file);
        assert_eq!(
            f.session.check_external_change(),
            Ok(ExternalChange::Reloaded)
        );
        assert!(!log.lock().unwrap().contains(&SessionEvent::VaultChanged));
        assert_eq!(f.state(), SaveState::Idle);
        assert_eq!(f.session.check_external_change(), Ok(ExternalChange::None));
    }

    #[test]
    fn check_external_change_wrong_password_reports() {
        let f = setup();
        f.add("A").unwrap();
        let (_, db) = VaultFile::open(&f.path, &pw("pw")).unwrap();
        fs::write(&f.path, db.save(&pw("other")).unwrap()).unwrap();
        assert_eq!(
            f.session.check_external_change(),
            Err(SessionError::WrongPassword)
        );
        assert!(f.session.is_unlocked());
        assert_eq!(f.titles(), v(&["A", "first"]));
        assert_eq!(
            f.session.check_external_change(),
            Err(SessionError::WrongPassword),
            "the stamp was not refreshed"
        );
    }

    #[test]
    fn check_external_change_while_locked_is_locked() {
        let f = setup();
        f.session.lock(LockReason::Manual);
        assert_eq!(f.session.check_external_change(), Err(SessionError::Locked));
    }

    #[test]
    fn failed_flush_on_lock_keeps_pending_bytes_and_retries() {
        let (f, io) = failing_setup();
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        assert_eq!(f.session.status().state, SessionState::Locked);
        assert!(f.has_pending_bytes());
        assert_eq!(f.disk(), v(&["first"]));
        io.heal();
        f.clock.advance(PENDING_RETRY);
        assert_eq!(f.disk(), v(&["A", "first"]));
        assert!(!f.has_pending_bytes());
        assert_eq!(f.state(), SaveState::Idle);
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.titles(), v(&["A", "first"]));
    }

    #[test]
    fn failed_save_on_lock_is_saved_by_the_next_unlock() {
        let (f, io) = failing_setup();
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        assert_eq!(f.state(), SaveState::Failed);
        io.heal();
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.titles(), v(&["A", "first"]));
        assert_eq!(f.state(), SaveState::Idle);
        assert_eq!(f.disk(), v(&["A", "first"]));
        assert!(!f.has_pending_bytes());
    }

    #[test]
    fn pending_bytes_merged_on_unlock_when_file_changed() {
        let (f, io) = failing_setup();
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        external_add(&f.path, "B", T0 + 100);
        io.heal();
        // The retry timer never writes over the changed file.
        f.clock.advance(PENDING_RETRY);
        assert_eq!(f.disk(), v(&["B", "first"]));
        assert!(f.has_pending_bytes());
        assert_eq!(f.session.flush(), Err(SessionError::ChangedOnDisk));
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.titles(), v(&["A", "B", "first"]));
        assert_eq!(f.disk(), v(&["A", "B", "first"]));
        assert!(!f.has_pending_bytes());
        assert_eq!(f.state(), SaveState::Idle);
    }

    #[test]
    fn pending_bytes_survive_a_password_change_and_are_kept_as_a_copy_on_switch() {
        let (f, io) = failing_setup();
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        io.heal();
        // Another device changes the master password and adds B.
        let (_, mut db) = VaultFile::open(&f.path, &pw("pw")).unwrap();
        let mut e = Entry::new(Timestamp(T0 + 100));
        e.set_field("Title", "B", false);
        let root = db.root_id();
        db.add_entry(root, e, Timestamp(T0 + 100)).unwrap();
        fs::write(&f.path, db.save(&pw("new")).unwrap()).unwrap();
        assert_eq!(f.session.unlock(pw("pw")), Err(SessionError::WrongPassword));
        assert_eq!(
            f.session.unlock(pw("new")),
            Err(SessionError::WrongPassword)
        );
        assert!(f.has_pending_bytes(), "a failed unlock keeps the edits");
        assert!(f.session.flush().is_err(), "the quit flow sees them");
        // Re-opening the vault with the new password keeps the edits as an `unsaved-*` copy.
        f.session.open_vault(&f.path, pw("new")).unwrap();
        assert_eq!(f.titles(), v(&["B", "first"]));
        assert!(!f.has_pending_bytes());
        let folder = f
            .session
            .0
            .backups
            .folder(&f.session.vault_id().unwrap())
            .unwrap();
        let copies: Vec<_> = fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(UNSAVED_COPY_PREFIX)
            })
            .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_eq!(disk_titles(&copies[0], "pw"), v(&["A", "first"]));
    }

    #[test]
    fn opening_another_vault_with_kept_edits_writes_a_copy_first() {
        let (f, io) = failing_setup();
        let vault_id = f.session.vault_id().unwrap();
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        io.heal();
        external_add(&f.path, "B", T0 + 100);
        let other = f.dir.path().join("other.kdbx");
        f.session
            .create_vault_with(&other, pw("x"), fast_database("Other"))
            .unwrap();
        assert_eq!(f.session.status().vault_name.as_deref(), Some("Other"));
        let folder = f.session.0.backups.folder(&vault_id).unwrap();
        let copy = fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(UNSAVED_COPY_PREFIX)
            })
            .unwrap();
        assert_eq!(disk_titles(&copy, "pw"), v(&["A", "first"]));
        assert_eq!(disk_titles(&f.path, "pw"), v(&["B", "first"]));
    }

    #[test]
    fn opening_another_vault_saves_live_edits_first() {
        let f = setup();
        f.add("A").unwrap();
        let other = f.dir.path().join("other.kdbx");
        f.session
            .create_vault_with(&other, pw("x"), fast_database("Other"))
            .unwrap();
        assert_eq!(f.disk(), v(&["A", "first"]));
    }

    #[test]
    fn save_failed_state_reports_message_and_retries() {
        let (f, io) = failing_setup();
        let log = record(&f.session);
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.add("A").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        let s = f.session.status().save;
        assert_eq!(s.state, SaveState::Failed);
        assert!(s.message.is_some());
        assert!(log.lock().unwrap().iter().any(|e| matches!(
            e,
            SessionEvent::SaveChanged(SaveStatus {
                state: SaveState::Failed,
                ..
            })
        )));
        io.heal();
        f.clock.advance(PENDING_RETRY);
        assert_eq!(f.session.status().save, SaveStatus::idle());
        assert_eq!(f.disk(), v(&["A", "first"]));
    }

    #[test]
    fn flush_returns_ok_only_when_saved_gen_reaches_dirty_gen() {
        let (f, io) = failing_setup();
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        assert!(matches!(f.session.flush(), Err(SessionError::Io(_))));
        assert_eq!(f.state(), SaveState::Failed);
        let (saved, dirty) = f.gens();
        assert_ne!(saved, dirty);
        io.heal();
        f.session.flush().unwrap();
        let (saved, dirty) = f.gens();
        assert_eq!(saved, dirty);
        assert_eq!(f.state(), SaveState::Idle);
        assert_eq!(f.session.flush(), Ok(()), "nothing dirty is Ok");
    }

    #[test]
    fn flush_waits_for_in_flight_write_then_covers_edits_made_meanwhile() {
        let io = Arc::new(GateIo::default());
        let f = setup_with(io.clone(), true);
        io.arm();
        f.add("A").unwrap();
        io.wait_entered();
        // The write runs outside the session lock: status and edits stay available.
        assert_eq!(f.state(), SaveState::Saving);
        f.add("B").unwrap();
        let releaser = {
            let io = io.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                io.release();
            })
        };
        f.session.flush().unwrap();
        releaser.join().unwrap();
        assert_eq!(f.disk(), v(&["A", "B", "first"]));
        let (saved, dirty) = f.gens();
        assert_eq!(saved, dirty);
    }

    #[test]
    fn lock_waits_for_in_flight_write() {
        let io = Arc::new(GateIo::default());
        let f = setup_with(io.clone(), true);
        io.arm();
        f.add("A").unwrap();
        io.wait_entered();
        let releaser = {
            let io = io.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                io.release();
            })
        };
        f.session.lock(LockReason::Manual);
        releaser.join().unwrap();
        assert_eq!(f.session.status().state, SessionState::Locked);
        assert!(!f.has_pending_bytes());
        assert_eq!(f.disk(), v(&["A", "first"]));
    }

    #[test]
    fn flush_blocking_times_out_when_write_hangs() {
        let io = Arc::new(GateIo::default());
        let f = setup_with(io.clone(), false);
        io.arm();
        f.add("A").unwrap();
        let r = f.session.flush_blocking(Duration::from_millis(200));
        assert_eq!(r, Err(SessionError::Io("flush timed out".into())));
        assert_ne!(f.state(), SaveState::Idle);
        let (saved, dirty) = f.gens();
        assert_ne!(saved, dirty);
        io.release();
        f.session.flush().unwrap();
        assert_eq!(f.disk(), v(&["A", "first"]));
    }

    #[test]
    fn flush_blocking_waits_for_in_flight_save() {
        let io = Arc::new(GateIo::default());
        let f = setup_with(io.clone(), true);
        io.arm();
        f.add("A").unwrap();
        io.wait_entered();
        let releaser = {
            let io = io.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                io.release();
            })
        };
        f.session.flush_blocking(Duration::from_secs(5)).unwrap();
        releaser.join().unwrap();
        assert_eq!(f.disk(), v(&["A", "first"]));
    }

    #[test]
    fn conflict_hook_reports_merge_on_save_and_on_check() {
        let f = setup();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        f.session
            .set_conflict_hook(Arc::new(move |e| s.lock().unwrap().push(e)));
        f.add("A").unwrap();
        external_add(&f.path, "B", T0 + 100);
        f.session.flush().unwrap();
        external_add(&f.path, "C", T0 + 200);
        assert_eq!(
            f.session.check_external_change(),
            Ok(ExternalChange::Merged)
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![ConflictEvent::MergedOnSave, ConflictEvent::MergedOnCheck]
        );
    }

    #[test]
    fn first_write_of_session_always_snapshots_even_when_interval_daily() {
        let f = setup();
        f.settings
            .update(SettingsPatch {
                backup_interval: Some(BackupInterval::Daily),
                ..SettingsPatch::default()
            })
            .unwrap();
        f.add("A").unwrap();
        f.session.flush().unwrap();
        assert_eq!(f.backups(), 1);
        f.add("B").unwrap();
        f.session.flush().unwrap();
        assert_eq!(f.backups(), 1, "daily: not due again");
        f.session.lock(LockReason::Manual);
        f.session.unlock(pw("pw")).unwrap();
        f.add("C").unwrap();
        f.session.flush().unwrap();
        assert_eq!(f.backups(), 2, "first write after unlock");
    }

    #[test]
    fn no_snapshot_for_brand_new_file() {
        let f = setup();
        let path = f.dir.path().join("new.kdbx");
        f.session
            .create_vault_with(&path, pw("pw"), fast_database("New"))
            .unwrap();
        f.add("A").unwrap();
        f.session.flush().unwrap();
        assert_eq!(f.backups(), 0, "the fresh file holds nothing of the user's");
        f.add("B").unwrap();
        f.session.flush().unwrap();
        assert_eq!(f.backups(), 1);
    }

    #[test]
    fn same_content_ignores_child_order() {
        let mut a = fast_database("x");
        let root = a.root_id();
        for t in ["one", "two", "three"] {
            let mut e = Entry::new(Timestamp(T0));
            e.set_field("Title", t, false);
            a.add_entry(root, e, Timestamp(T0)).unwrap();
        }
        let mut b = a.clone();
        b.root.entries.reverse();
        let merged = merge(&a, &b);
        assert!(same_content(&merged, &a));
        assert!(same_content(&merged, &b));
        let mut c = a.clone();
        c.root.entries.pop();
        assert!(!same_content(&merged, &c));
    }

    #[test]
    fn scheduled_save_after_the_session_is_dropped_does_nothing() {
        let f = setup();
        f.add("A").unwrap();
        let clock = f.clock.clone();
        let path = f.path.clone();
        let before = fs::read(&path).unwrap();
        drop(f.session);
        clock.advance(AUTOSAVE_DEBOUNCE);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
