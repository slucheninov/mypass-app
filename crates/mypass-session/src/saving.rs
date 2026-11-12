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
//!   the save reloads it and merges it into the LIVE session database (edits made while the
//!   write ran are included, see "Remote content" below), emits `VaultChanged`, and writes the
//!   merge (up to [`MAX_SAVE_ROUNDS`] rounds). The merge result is kept in the session and
//!   reaches disk: this is the fix for the old app's bug that discarded the merged result
//!   (`SyncResult.merged`) until the next unlock.
//! - **A dirty session always has a save job armed.** Every operation that runs outside the
//!   session lock (a write, a retry, an external-change check) ends through one finish path
//!   that clears the in-flight flag and, when the session is still dirty and no save job is
//!   armed, schedules the autosave. A scheduled job that met the operation in flight and
//!   returned is therefore never the last word. A panicking operation (a bug) still clears the
//!   flag, puts the `VaultFile` back and wakes every waiter, so `lock` never hangs on it.
//! - **Locking keeps unsaved edits.** `lock` flushes first. If that fails, the database is
//!   serialised (encrypted with the master password, so safe to hold) into `pending_bytes`
//!   before it is dropped. A timer retries writing them every [`PENDING_RETRY`], but only while
//!   the file on disk is unchanged since the session last read or wrote it. Otherwise they wait
//!   for the next `unlock`, which decrypts them with the supplied password, merges them with the
//!   vault on disk and saves.
//! - **A failing disk does not rotate the backups away.** The snapshot before a write is taken
//!   once per disk state: the retries of a failed write (and of the kept edits) take none until
//!   a write succeeds or the disk is seen changed. The backup manager also skips a snapshot whose
//!   bytes equal the newest backup.
//!
//! # Remote content: entries, metadata and save settings
//!
//! `mypass_core::merge::merge` merges entries, groups and tombstones and keeps the LOCAL
//! metadata and save settings. The session applies remote content with these rules:
//!
//! - **Clean session** (no unsaved edits): the remote database is ADOPTED wholesale (entries,
//!   groups, metadata, save settings). The session stays clean and nothing is written: the disk
//!   already holds exactly that.
//! - **Dirty session**: entries and groups are merged by the merge engine. The model has no
//!   per-field change timestamps for the metadata (KeePass `*Changed` elements are not parsed),
//!   so the session remembers the metadata last read from or written to disk (`disk_meta`) as
//!   the base: when the live metadata still equals that base (ignoring the recycle bin id, which
//!   the merge chooses), the remote's metadata is taken; when the user changed it locally, the
//!   local one is kept. The save settings (cipher, compression, KDF and its parameters, minor
//!   version) are ALWAYS taken from the remote: a KDF or cipher change made by another device
//!   is never reverted by our next save (the preserve-KDF rule: a re-save keeps what the file
//!   has; salts and seeds are redrawn on every save anyway).
//! - **Is the disk already what we would write?** The merge result is compared with the remote
//!   through the normalising comparison (`same_content`, which covers entries, groups,
//!   tombstones and metadata) AND the save settings (salt and seed ignored). Equal means clean
//!   and no write; different means dirty and one write. Two sessions over one file therefore
//!   converge: the writer's result is adopted by the clean reader, and nobody writes again.
//!
//! # Ordering between writers
//!
//! A write runs OUTSIDE the session lock (so `status()` and reads stay responsive during the
//! KDF), with `Inner::in_flight` set and the `VaultFile` taken out of the session. Every path
//! that writes or replaces the file (`flush`, the autosave job, the retry jobs, `lock`,
//! `check_external_change`, `unlock`, opening another vault, `relocate`) first waits on the
//! `Shared::idle` condvar until no write is in flight, so two writes never overlap and never
//! land out of order. A scheduled job that finds a write in flight returns at once: the finish
//! path of the running operation re-arms the autosave while the session is dirty, and `flush`
//! loops until nothing is dirty. Scheduled jobs hold only a weak reference to the session and
//! re-check its state when they run (a cancelled job may still run once).
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
//!    switches, the session writes the kept edits as `unsaved-<unix_secs>[-<n>].kdbx` into this
//!    vault's backup folder (the folder "Reveal backups" opens), still encrypted with the OLD
//!    password, and only then lets them go. That file opens in MyPass or KeePassXC with the old
//!    password, so the edits can be copied into the vault. The copy is written through the
//!    `FileIo` seam to a temporary name and then hard-linked to its final name, which fails
//!    instead of replacing an existing file (the next free `-<n>` suffix is taken). Backup
//!    rotation never deletes `unsaved-*` files. If that copy cannot be written, the switch is
//!    refused and the edits stay kept.
//!
//! Kept edits otherwise live only in memory: `flush` (and `flush_blocking` on the quit path)
//! returns an error while they cannot be written, so the quit flow can tell the user.

use crate::scheduler::lock;
use crate::session::{
    ConflictEvent, Inner, LockReason, Outbox, SaveStatus, SessionError, Shared, VaultSession,
};
use mypass_core::merge::merge;
use mypass_kdbx::{
    Database, FileIo, KdbxError, KdfParams, Meta, SaveOutcome, SaveSettings, VaultFile,
};
use secrecy::SecretString;
use std::io::ErrorKind;
use std::ops::{Deref, DerefMut};
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
/// Highest `-<n>` suffix tried for an `unsaved-*` copy within one second.
const MAX_UNSAVED_SUFFIX: u32 = 99;

/// What [`VaultSession::check_external_change`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalChange {
    /// The file is as the session last read or wrote it.
    None,
    /// Remote content was merged into (or adopted by) the session: the vault changed in memory.
    Merged,
    /// The file changed on disk but holds nothing new for the user (stamp refreshed; save
    /// settings such as the KDF are still taken over).
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

/// One write taken out of the session for the duration of the I/O (the `VaultFile` travels in
/// the [`InFlight`] guard).
struct Work {
    db: Database,
    password: SecretString,
    gen: u64,
    vault_id: Option<String>,
    first_write: bool,
    /// Take the snapshot before this write (once per disk state, never for a fresh file).
    snapshot: bool,
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

/// Where the `VaultFile` taken out for an operation goes back.
#[derive(Clone, Copy)]
enum Slot {
    /// `Inner::file` (unlocked session).
    File,
    /// `Inner::pending_file` (locked with kept edits).
    Pending,
}

/// Marks one operation in flight outside the session lock and owns the `VaultFile` meanwhile.
/// [`InFlight::finish`] ends it normally; if it is dropped unfinished (a panic), `Drop` still
/// clears the flag, puts the file back and wakes every waiter, so `lock` and `flush` never
/// hang on a dead operation.
struct InFlight<'a> {
    session: &'a VaultSession,
    file: Option<VaultFile>,
    slot: Slot,
    done: bool,
}

impl<'a> InFlight<'a> {
    fn begin(session: &'a VaultSession, inner: &mut Inner, file: VaultFile, slot: Slot) -> Self {
        inner.in_flight = true;
        Self {
            session,
            file: Some(file),
            slot,
            done: false,
        }
    }

    /// Runs `op` on the file; `None` only if the file is gone (never, by construction).
    fn run<R>(&mut self, op: impl FnOnce(&mut VaultFile) -> R) -> Option<R> {
        self.file.as_mut().map(op)
    }

    fn restore(&mut self, inner: &mut Inner) {
        inner.in_flight = false;
        if let Some(file) = self.file.take() {
            match self.slot {
                Slot::File => inner.file = Some(file),
                Slot::Pending => inner.pending_file = Some(file),
            }
        }
        self.session.0.idle.notify_all();
    }

    /// Ends the operation: retakes the session lock, clears the flag, puts the file back and
    /// wakes waiters. The returned guard re-arms the autosave when it is dropped (see
    /// [`Finished`]).
    fn finish(mut self) -> Finished<'a> {
        let mut inner = lock(&self.session.0.inner);
        self.restore(&mut inner);
        self.done = true;
        Finished {
            session: self.session,
            inner,
        }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let session = self.session;
        let mut inner = lock(&session.0.inner);
        self.restore(&mut inner);
        // Unwinding from a bug: keep the edits on the slow retry timer, not the 1 s debounce.
        if inner.save_job.is_none() {
            if inner.db.is_some() && inner.is_dirty() {
                session.schedule_job(&mut inner, PENDING_RETRY, false);
            } else if inner.db.is_none() && inner.pending_bytes.is_some() {
                session.schedule_job(&mut inner, PENDING_RETRY, true);
            }
        }
    }
}

/// The session lock held at the end of an operation. Dropping it (after the operation applied
/// its result) arms the autosave when the session is dirty and no save job is armed: the one
/// central place that keeps every edit on its way to disk.
struct Finished<'a> {
    session: &'a VaultSession,
    inner: MutexGuard<'a, Inner>,
}

impl Deref for Finished<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.inner
    }
}

impl DerefMut for Finished<'_> {
    fn deref_mut(&mut self) -> &mut Inner {
        &mut self.inner
    }
}

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        let inner = &mut *self.inner;
        if inner.db.is_some() && inner.is_dirty() && inner.save_job.is_none() {
            self.session.schedule_autosave(inner);
        }
    }
}

/// Whether `merged` (a merge output) holds the same content as `other`. The merge sorts children
/// and tombstones by UUID and normalises history, so `other` is compared in that canonical form
/// (`merge(other, other)`), never with a plain `==` that would see a different child order as a
/// change. Covers entries, groups, tombstones and metadata, not the save settings.
pub(crate) fn same_content(merged: &Database, other: &Database) -> bool {
    *merged == merge(other, other)
}

/// The KDF parameters without the salt or seed (redrawn on every save).
fn kdf_shape(kdf: &KdfParams) -> KdfParams {
    let mut kdf = kdf.clone();
    match &mut kdf {
        KdfParams::Argon2 { salt, .. } => salt.clear(),
        KdfParams::AesKdf { seed, .. } => seed.clear(),
    }
    kdf
}

/// Whether two sets of save settings write the same file format (salt and seed ignored).
fn same_save_settings(a: &SaveSettings, b: &SaveSettings) -> bool {
    a.minor_version == b.minor_version
        && a.cipher == b.cipher
        && a.compression == b.compression
        && kdf_shape(&a.kdf) == kdf_shape(&b.kdf)
}

/// Whether writing `db` would put on disk what `remote` (the disk) already holds: the content
/// through the normalising comparison and the save settings.
pub(crate) fn same_on_disk(db: &Database, remote: &Database) -> bool {
    same_content(db, remote) && same_save_settings(&db.settings, &remote.settings)
}

/// Metadata equality ignoring the recycle bin id (the merge chooses the live bin itself).
fn same_meta(a: &Meta, b: &Meta) -> bool {
    let strip = |m: &Meta| Meta {
        recycle_bin_uuid: None,
        ..m.clone()
    };
    strip(a) == strip(b)
}

/// Merges a DIRTY session's `live` database with `remote` (see the module docs): entries and
/// groups through the merge engine; the remote's metadata when the live metadata still equals
/// `base` (the metadata last read from or written to disk), the local one otherwise; the
/// remote's save settings always.
pub(crate) fn reconcile(live: &Database, remote: &Database, base: Option<&Meta>) -> Database {
    let mut merged = merge(live, remote);
    if base.is_some_and(|b| same_meta(b, &live.meta)) {
        let bin = merged.meta.recycle_bin_uuid;
        merged.meta = remote.meta.clone();
        merged.meta.recycle_bin_uuid = bin;
    }
    merged.settings = remote.settings.clone();
    merged
}

fn wrong_password_message() -> String {
    "the vault on disk no longer opens with the current password".to_owned()
}

fn file_unavailable() -> SessionError {
    SessionError::Io("the vault file is not available".into())
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
        inner.save_job_id = inner.save_job_id.wrapping_add(1);
        let id = inner.save_job_id;
        let weak = self.weak();
        let job = Box::new(move || {
            if let Some(shared) = weak.upgrade() {
                let session = VaultSession(shared);
                {
                    // This job is no longer armed: whoever finishes next may arm a new one.
                    let mut inner = lock(&session.0.inner);
                    if inner.save_job_id == id {
                        inner.save_job = None;
                    }
                }
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
    fn take_work(&self, inner: &mut Inner, out: &mut Outbox) -> Option<(Work, InFlight<'_>)> {
        if !inner.is_dirty() {
            return None;
        }
        let (Some(db), Some(password)) = (&inner.db, &inner.password) else {
            return None;
        };
        let (db, password) = (db.clone(), password.clone());
        let file = inner.file.take()?;
        inner.cancel_save_job();
        out.save(inner.set_save(SaveStatus::saving()));
        let fresh_file = std::mem::take(&mut inner.fresh_file);
        let snapshot = !fresh_file && !inner.snapshot_taken;
        if snapshot {
            inner.snapshot_taken = true;
        }
        let work = Work {
            db,
            password,
            gen: inner.dirty_gen,
            vault_id: inner.vault_id.clone(),
            first_write: std::mem::take(&mut inner.first_write_of_session),
            snapshot,
        };
        Some((work, InFlight::begin(self, inner, file, Slot::File)))
    }

    /// Snapshot (once per disk state), then one guarded write; on `ChangedOnDisk` the remote
    /// content is reloaded.
    fn write_work(&self, w: &Work, guard: &mut InFlight<'_>) -> WriteResult {
        let now = self.now().0;
        let backups = &self.0.backups;
        let result = guard.run(|file| {
            if let (true, Some(id)) = (w.snapshot, &w.vault_id) {
                backups.snapshot_before_save(id, file.path(), now, w.first_write);
            }
            match file.save(&w.db, &w.password) {
                Ok(SaveOutcome::Saved) => WriteResult::Saved(w.gen),
                Ok(SaveOutcome::ChangedOnDisk) => match file.reload(&w.password) {
                    Ok(remote) => WriteResult::Remote(Box::new(remote)),
                    Err(e) => WriteResult::Failed(e.into()),
                },
                Err(e) => WriteResult::Failed(e.into()),
            }
        });
        result.unwrap_or_else(|| WriteResult::Failed(file_unavailable()))
    }

    /// The core save loop shared by the autosave job, the retry job and `flush`.
    fn run_save(&self, caller: Caller) -> Result<(), SessionError> {
        let mut rounds = 0usize;
        loop {
            let mut out = Outbox::default();
            let taken = {
                let mut inner = lock(&self.0.inner);
                if caller == Caller::Flush {
                    inner = self.wait_idle(inner);
                } else if inner.in_flight {
                    // The running operation re-arms the autosave when it finishes.
                    return Ok(());
                }
                self.take_work(&mut inner, &mut out)
            };
            self.emit(out);
            let Some((work, mut guard)) = taken else {
                return Ok(());
            };
            let result = self.write_work(&work, &mut guard);
            let mut out = Outbox::default();
            let mut hook = None;
            let step = {
                let mut inner = guard.finish();
                let step = match result {
                    WriteResult::Saved(gen) => {
                        inner.saved_gen = gen;
                        inner.snapshot_taken = false;
                        inner.disk_meta = Some(work.db.meta.clone());
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

    /// Applies `remote` (the disk content just read) to the live database, following the rules
    /// in the module docs: a clean session adopts it wholesale and stays clean; a dirty one
    /// [`reconcile`]s and stays dirty only when the result differs from the disk. Returns
    /// whether the live content changed.
    fn apply_remote(&self, inner: &mut Inner, remote: &Database, out: &mut Outbox) -> bool {
        let Some(live) = &inner.db else {
            return false;
        };
        let dirty = inner.is_dirty();
        let (next, changed, on_disk) = if dirty {
            let merged = reconcile(live, remote, inner.disk_meta.as_ref());
            let changed = !same_content(&merged, live);
            let on_disk = same_on_disk(&merged, remote);
            (merged, changed, on_disk)
        } else {
            let changed = !same_content(&merge(remote, remote), live);
            (remote.clone(), changed, true)
        };
        if let Some(path) = &inner.path {
            // Not a `StateChanged`: background sync is not user activity (auto-lock).
            inner.vault_name = crate::session::vault_name_for(&next, path);
        }
        inner.db = Some(next);
        inner.disk_meta = Some(remote.meta.clone());
        inner.snapshot_taken = false;
        if changed {
            out.vault_changed = true;
        }
        if on_disk {
            inner.saved_gen = inner.dirty_gen;
        } else if changed {
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
    ///
    /// Every call spawns one helper thread (`mypass-flush`) that runs `flush` to its end, even
    /// after the deadline. It is meant for the single call on the quit path; a caller that
    /// loops on it while a write hangs piles up one blocked thread per call (each then waits for
    /// the hung write in turn).
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
    /// kept any more. The snapshot before the write is taken on the first attempt for a disk
    /// state only.
    fn retry_pending(&self) -> Result<(), SessionError> {
        let (bytes, mut guard, vault_id, snapshot) = {
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
            let snapshot = !inner.snapshot_taken;
            inner.snapshot_taken = true;
            let vault_id = inner.vault_id.clone();
            let guard = InFlight::begin(self, &mut inner, file, Slot::Pending);
            (bytes, guard, vault_id, snapshot)
        };
        let now = self.now().0;
        let result = guard
            .run(|file| match file.has_changed_on_disk() {
                Ok(false) => {
                    if let (true, Some(id)) = (snapshot, &vault_id) {
                        self.0
                            .backups
                            .snapshot_before_save(id, file.path(), now, true);
                    }
                    self.0
                        .io
                        .write_atomic(file.path(), &bytes)
                        .map(|()| true)
                        .map_err(SessionError::from)
                }
                Ok(true) => Ok(false),
                Err(e) => Err(e.into()),
            })
            .unwrap_or_else(|| Err(file_unavailable()));
        let mut out = Outbox::default();
        let r = {
            let mut inner = guard.finish();
            match result {
                Ok(true) => {
                    inner.snapshot_taken = false;
                    if inner.pending_bytes.as_ref() == Some(&bytes) {
                        inner.pending_bytes = None;
                        inner.pending_file = None;
                        inner.saved_gen = inner.dirty_gen;
                        out.save(inner.set_save(SaveStatus::idle()));
                    }
                    Ok(())
                }
                Ok(false) => {
                    // The file changed on disk: the next unlock merges the kept edits.
                    inner.snapshot_taken = false;
                    Err(SessionError::ChangedOnDisk)
                }
                Err(e) => {
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
    ///
    /// The bytes go through the `FileIo` seam to a temporary dot-file, which is then hard-linked
    /// to the first free final name: a hard link fails with `AlreadyExists` instead of replacing
    /// a file, so an existing copy (or anything else of that name) is never overwritten. A file
    /// system without hard links refuses the copy (and so the switch); the edits stay kept.
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
        let temp = folder.join(format!(".{UNSAVED_COPY_PREFIX}{secs}.tmp"));
        self.0.io.write_atomic(&temp, &bytes)?;
        let linked = (0..=MAX_UNSAVED_SUFFIX)
            .map(|n| match n {
                0 => folder.join(format!("{UNSAVED_COPY_PREFIX}{secs}.kdbx")),
                n => folder.join(format!("{UNSAVED_COPY_PREFIX}{secs}-{n}.kdbx")),
            })
            .find_map(|path| match std::fs::hard_link(&temp, &path) {
                Ok(()) => Some(Ok(path)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(SessionError::Io(e.to_string()))),
            })
            .unwrap_or(Err(SessionError::AlreadyExists));
        // The temporary name is ours alone; a leftover is harmless and overwritten next time.
        let _ = std::fs::remove_file(&temp);
        let path = linked?;
        let mut inner = lock(&self.0.inner);
        if inner.pending_bytes.as_ref() == Some(&bytes) {
            inner.pending_bytes = None;
            inner.pending_file = None;
            inner.cancel_save_job();
        }
        Ok(path)
    }

    /// Replaces the vault with the backup `backup` (the bytes of a backup file).
    ///
    /// Order: the bytes must open with the CURRENT master password (`InvalidInput` otherwise)
    /// BEFORE anything is written, so a refusal leaves the file, the live database and the backup
    /// folder untouched; then the current edits are flushed, the current file is snapshotted as a
    /// backup, the backup bytes are written atomically over the vault and the file is reloaded
    /// into the live database (`VaultChanged` is emitted). The session stays unlocked and clean.
    /// Refused while quitting and while locked. A reload that fails after the write (the disk
    /// changed again, I/O) is returned as an error and the session is LOCKED (`Manual`): the live
    /// database is the replaced content, and unlocking reads the restored file.
    pub fn restore_from_backup(&self, backup: &[u8]) -> Result<(), SessionError> {
        self.check_not_quitting()?;
        let password = self.with_db_and_password(|_, p| p.clone())?;
        match Database::open(backup, &password) {
            Ok(_) => {}
            Err(KdbxError::WrongPassword) => {
                return Err(SessionError::InvalidInput(
                    "backup does not open with the current master password".into(),
                ))
            }
            Err(e) => return Err(e.into()),
        }
        self.flush()?;
        let (mut guard, vault_id) = {
            let mut inner = self.wait_idle(lock(&self.0.inner));
            if inner.quitting {
                return Err(SessionError::Quitting);
            }
            if inner.db.is_none() {
                return Err(inner.not_unlocked());
            }
            if inner.is_dirty() {
                // Edits landed after the flush above; never throw them away unsaved.
                return Err(SessionError::InvalidInput(
                    "the vault changed while restoring; try again".into(),
                ));
            }
            let Some(file) = inner.file.take() else {
                return Err(file_unavailable());
            };
            inner.cancel_save_job();
            let vault_id = inner.vault_id.clone();
            (
                InFlight::begin(self, &mut inner, file, Slot::File),
                vault_id,
            )
        };
        let now = self.now().0;
        let written = std::sync::atomic::AtomicBool::new(false);
        let result = guard.run(|file| {
            if let Some(id) = &vault_id {
                // `first_write_of_session`: always due, so the file being replaced is kept.
                self.0
                    .backups
                    .snapshot_before_save(id, file.path(), now, true);
            }
            self.0.io.write_atomic(file.path(), backup)?;
            written.store(true, std::sync::atomic::Ordering::SeqCst);
            file.reload(&password)
        });
        let mut out = Outbox::default();
        let r = {
            let mut inner = guard.finish();
            match result {
                None => Err(file_unavailable()),
                Some(Err(e)) => Err(e.into()),
                Some(Ok(db)) => {
                    self.apply_remote(&mut inner, &db, &mut out);
                    // The restored content is on disk: nothing is pending.
                    out.save(inner.set_save(SaveStatus::idle()));
                    Ok(())
                }
            }
        };
        self.emit(out);
        if r.is_err() && written.load(std::sync::atomic::Ordering::SeqCst) {
            // The backup is on disk but the live database is still the old content: lock, so
            // nothing can later merge the old database over the restored file. Unlocking reads
            // the restored file.
            self.lock(LockReason::Manual);
        }
        r
    }

    /// Checks whether the vault changed on disk and, if so, applies the remote content to the
    /// session (called on window focus and every 30 s while unlocked; see the module docs for
    /// the rules). A reload that fails with `WrongPassword` (the password was changed elsewhere)
    /// is returned as such and leaves the session untouched.
    pub fn check_external_change(&self) -> Result<ExternalChange, SessionError> {
        let (mut guard, password) = {
            let mut inner = self.wait_idle(lock(&self.0.inner));
            let Some(password) = inner.password.clone() else {
                return Err(inner.not_unlocked());
            };
            let Some(file) = inner.file.take() else {
                return Err(inner.not_unlocked());
            };
            (
                InFlight::begin(self, &mut inner, file, Slot::File),
                password,
            )
        };
        let result = guard.run(|file| match file.has_changed_on_disk() {
            Ok(false) => Ok(None),
            Ok(true) => file.reload(&password).map(Some),
            Err(e) => Err(e),
        });
        let mut out = Outbox::default();
        let mut hook = None;
        let r = {
            let mut inner = guard.finish();
            match result {
                None => Err(file_unavailable()),
                Some(Err(KdbxError::WrongPassword)) => Err(SessionError::WrongPassword),
                Some(Err(e)) => Err(e.into()),
                Some(Ok(None)) => Ok(ExternalChange::None),
                Some(Ok(Some(remote))) => {
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
    use mypass_kdbx::{Entry, FileMeta, KdfParams, PlainFileIo, Timestamp};
    use std::fs;
    use std::io::ErrorKind;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::{Condvar, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    const T0: i64 = 1_700_000_000;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    /// How long a test helper waits for another thread before it fails the test.
    const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

    /// Which `FileIo` call a [`GateIo`] blocks.
    #[derive(Default, Clone, Copy, PartialEq, Eq)]
    enum GateOp {
        #[default]
        Write,
        Metadata,
    }

    /// Test-only IO that blocks the next call of the armed kind after `arm` until `release`.
    #[derive(Default)]
    struct Gate {
        armed: bool,
        op: GateOp,
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
            self.arm_on(GateOp::Write);
        }
        fn arm_on(&self, op: GateOp) {
            *self.gate.lock().unwrap() = Gate {
                armed: true,
                op,
                ..Gate::default()
            };
        }
        fn wait_entered(&self) {
            let g = self.gate.lock().unwrap();
            let (g, _) = self
                .cv
                .wait_timeout_while(g, HELPER_TIMEOUT, |g| !g.entered)
                .unwrap();
            assert!(g.entered, "the gated call never started within 5 s");
        }
        /// Opens the gate; never blocks.
        fn release(&self) {
            self.gate.lock().unwrap().open = true;
            self.cv.notify_all();
        }
        /// Blocks the caller while the gate is armed for `op`, at most [`HELPER_TIMEOUT`].
        fn pass(&self, op: GateOp) {
            let mut g = self.gate.lock().unwrap();
            if g.armed && g.op == op {
                g.armed = false;
                g.entered = true;
                self.cv.notify_all();
                let (g, _) = self
                    .cv
                    .wait_timeout_while(g, HELPER_TIMEOUT, |g| !g.open)
                    .unwrap();
                assert!(g.open, "the gate was not released within 5 s");
            }
        }
    }

    impl FileIo for GateIo {
        fn read(&self, path: &Path) -> mypass_kdbx::Result<Vec<u8>> {
            PlainFileIo.read(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> mypass_kdbx::Result<()> {
            self.pass(GateOp::Write);
            PlainFileIo.write_atomic(path, bytes)
        }
        fn metadata(&self, path: &Path) -> mypass_kdbx::Result<FileMeta> {
            self.pass(GateOp::Metadata);
            PlainFileIo.metadata(path)
        }
    }

    /// Test-only IO whose next `write_atomic` after `arm` panics.
    #[derive(Default)]
    struct PanicIo {
        armed: AtomicBool,
    }

    impl FileIo for PanicIo {
        fn read(&self, path: &Path) -> mypass_kdbx::Result<Vec<u8>> {
            PlainFileIo.read(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> mypass_kdbx::Result<()> {
            if self.armed.swap(false, AtomicOrdering::SeqCst) {
                panic!("injected write panic");
            }
            PlainFileIo.write_atomic(path, bytes)
        }
        fn metadata(&self, path: &Path) -> mypass_kdbx::Result<FileMeta> {
            PlainFileIo.metadata(path)
        }
    }

    /// Runs `f` on a helper thread and fails the test when it does not return within
    /// [`HELPER_TIMEOUT`].
    fn within_timeout<R: Send + 'static>(what: &str, f: impl FnOnce() -> R + Send + 'static) -> R {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(HELPER_TIMEOUT)
            .unwrap_or_else(|_| panic!("{what} did not return within 5 s"))
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

    // --- Remote metadata and save settings -------------------------------------------------

    /// Another device edits the file through its own handle.
    fn external_edit(path: &Path, edit: impl FnOnce(&mut Database)) {
        let (mut file, mut db) = VaultFile::open(path, &pw("pw")).unwrap();
        edit(&mut db);
        assert_eq!(file.save(&db, &pw("pw")).unwrap(), SaveOutcome::Saved);
    }

    fn kdf_iterations(db: &Database) -> u64 {
        match &db.settings.kdf {
            KdfParams::Argon2 { iterations, .. } => *iterations,
            KdfParams::AesKdf { rounds, .. } => *rounds,
        }
    }

    fn set_kdf_iterations(db: &mut Database, n: u64) {
        if let KdfParams::Argon2 { iterations, .. } = &mut db.settings.kdf {
            *iterations = n;
        }
    }

    fn disk_db(path: &Path) -> Database {
        VaultFile::open(path, &pw("pw")).unwrap().1
    }

    /// A second session (its own scheduler, IO counter and backups) over the same file.
    fn second_session(f: &Fx) -> (VaultSession, Arc<ManualScheduler>, Arc<FailingFileIo>) {
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        let clock = ManualScheduler::new(start());
        let session = VaultSession::new(SessionDeps {
            settings: SettingsStore::in_memory(),
            scheduler: clock.clone(),
            backups_root: f.dir.path().join("backups-2"),
            file_io: io.clone(),
        });
        session.open_vault(&f.path, pw("pw")).unwrap();
        (session, clock, io)
    }

    #[test]
    fn clean_session_adopts_remote_metadata_without_writing() {
        let (f, io) = failing_setup();
        external_edit(&f.path, |db| db.meta.database_name = "Renamed".into());
        let before = fs::read(&f.path).unwrap();
        let writes = io.write_calls();
        assert_eq!(
            f.session.check_external_change(),
            Ok(ExternalChange::Merged)
        );
        assert_eq!(
            f.session
                .with_db(|db| db.meta.database_name.clone())
                .unwrap(),
            "Renamed"
        );
        assert_eq!(f.session.status().vault_name.as_deref(), Some("Renamed"));
        assert_eq!(f.state(), SaveState::Idle);
        assert_eq!(f.clock.pending(), 0, "nothing to write");
        f.clock.advance(PENDING_RETRY);
        assert_eq!(io.write_calls(), writes);
        assert_eq!(fs::read(&f.path).unwrap(), before, "disk bytes untouched");
        assert_eq!(f.session.check_external_change(), Ok(ExternalChange::None));
    }

    #[test]
    fn remote_kdf_change_is_kept_by_the_next_local_save() {
        let f = setup();
        f.add("A").unwrap();
        external_edit(&f.path, |db| set_kdf_iterations(db, 2));
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        let disk = disk_db(&f.path);
        assert_eq!(titles(&disk), v(&["A", "first"]));
        assert_eq!(kdf_iterations(&disk), 2, "the other device's KDF is kept");
        assert_eq!(f.state(), SaveState::Idle);
    }

    #[test]
    fn clean_session_adopts_remote_kdf_on_check() {
        let f = setup();
        external_edit(&f.path, |db| set_kdf_iterations(db, 2));
        assert_eq!(
            f.session.check_external_change(),
            Ok(ExternalChange::Reloaded)
        );
        f.add("A").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        let disk = disk_db(&f.path);
        assert_eq!(titles(&disk), v(&["A", "first"]));
        assert_eq!(kdf_iterations(&disk), 2);
    }

    #[test]
    fn dirty_session_and_remote_rename_converge_without_ping_pong() {
        let (a, a_io) = failing_setup();
        let (b, b_clock, b_io) = second_session(&a);
        b.mutate(|db, _| {
            db.meta.database_name = "Renamed".into();
            Ok(())
        })
        .unwrap();
        b.flush().unwrap();
        a.add("A").unwrap();
        a.clock.advance(AUTOSAVE_DEBOUNCE);
        let disk = disk_db(&a.path);
        assert_eq!(disk.meta.database_name, "Renamed", "the rename survives");
        assert_eq!(titles(&disk), v(&["A", "first"]));
        assert_eq!(a.state(), SaveState::Idle);
        assert_eq!(b.check_external_change(), Ok(ExternalChange::Merged));
        assert_eq!(b.status().save.state, SaveState::Idle);
        let bytes = fs::read(&a.path).unwrap();
        let writes = (a_io.write_calls(), b_io.write_calls());
        for _ in 0..3 {
            assert_eq!(a.session.check_external_change(), Ok(ExternalChange::None));
            assert_eq!(b.check_external_change(), Ok(ExternalChange::None));
            a.clock.advance(PENDING_RETRY);
            b_clock.advance(PENDING_RETRY);
        }
        assert_eq!((a_io.write_calls(), b_io.write_calls()), writes);
        assert_eq!(fs::read(&a.path).unwrap(), bytes, "nobody writes any more");
        for s in [&a.session, &b] {
            assert_eq!(s.status().vault_name.as_deref(), Some("Renamed"));
            assert_eq!(
                titles(&s.with_db(Clone::clone).unwrap()),
                v(&["A", "first"])
            );
        }
    }

    #[test]
    fn local_rename_wins_over_an_untouched_remote_meta() {
        let f = setup();
        f.session
            .mutate(|db, _| {
                db.meta.database_name = "Local".into();
                Ok(())
            })
            .unwrap();
        external_add(&f.path, "B", T0 + 100);
        f.session.flush().unwrap();
        let disk = disk_db(&f.path);
        assert_eq!(disk.meta.database_name, "Local");
        assert_eq!(titles(&disk), v(&["B", "first"]));
    }

    // --- Autosave kept scheduled ----------------------------------------------------------

    #[test]
    fn autosave_that_meets_an_in_flight_check_is_rescheduled() {
        let io = Arc::new(GateIo::default());
        let f = setup_with(io.clone(), false);
        io.arm_on(GateOp::Metadata);
        let checker = {
            let session = f.session.clone();
            std::thread::spawn(move || session.check_external_change())
        };
        io.wait_entered();
        f.add("A").unwrap();
        // The autosave job fires while the check holds the file and returns at once.
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        assert_eq!(f.disk(), v(&["first"]));
        io.release();
        assert_eq!(checker.join().unwrap(), Ok(ExternalChange::None));
        assert_eq!(f.clock.pending(), 1, "a save job is armed after the check");
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        assert_eq!(f.disk(), v(&["A", "first"]));
        assert_eq!(f.state(), SaveState::Idle);
    }

    #[test]
    fn autosave_that_meets_a_failed_check_is_rescheduled() {
        let io = Arc::new(GateIo::default());
        let f = setup_with(io.clone(), false);
        // The check reloads a file that no longer opens with the password: `Err`.
        let (_, db) = VaultFile::open(&f.path, &pw("pw")).unwrap();
        let other = f.dir.path().join("other.kdbx");
        fs::write(&other, db.save(&pw("other")).unwrap()).unwrap();
        io.arm_on(GateOp::Metadata);
        let checker = {
            let session = f.session.clone();
            std::thread::spawn(move || session.check_external_change())
        };
        io.wait_entered();
        fs::rename(&other, &f.path).unwrap();
        f.add("A").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        io.release();
        assert_eq!(checker.join().unwrap(), Err(SessionError::WrongPassword));
        assert_eq!(
            f.clock.pending(),
            1,
            "a save job is armed after the failed check"
        );
    }

    // --- Panics -----------------------------------------------------------------------------

    #[test]
    fn panicking_write_clears_in_flight_and_lock_does_not_hang() {
        let io = Arc::new(PanicIo::default());
        let f = setup_with(io.clone(), false);
        f.add("A").unwrap();
        io.armed.store(true, AtomicOrdering::SeqCst);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.session.flush()));
        assert!(r.is_err(), "the injected panic propagates");
        {
            let inner = lock(&f.session.0.inner);
            assert!(!inner.in_flight);
            assert!(inner.file.is_some(), "the VaultFile is put back");
        }
        let session = f.session.clone();
        within_timeout("lock after a panicking write", move || {
            session.lock(LockReason::Manual)
        });
        assert!(!f.has_pending_bytes());
        assert_eq!(f.disk(), v(&["A", "first"]));
    }

    // --- The `unsaved-*` copy ---------------------------------------------------------------

    fn lock_with_kept_edits_and_external_change() -> (Fx, Arc<FailingFileIo>) {
        let (f, io) = failing_setup();
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        io.heal();
        external_add(&f.path, "B", T0 + 100);
        (f, io)
    }

    fn another_vault(f: &Fx) -> PathBuf {
        let other = f.dir.path().join("other.kdbx");
        fs::write(&other, fast_database("Other").save(&pw("x")).unwrap()).unwrap();
        other
    }

    #[test]
    fn unsaved_copy_never_overwrites_an_existing_file() {
        let (f, _io) = lock_with_kept_edits_and_external_change();
        let vault_id = f.session.vault_id().unwrap();
        let folder = f.session.0.backups.ensure_folder(&vault_id).unwrap();
        let secs = f.session.now().0;
        let taken = folder.join(format!("{UNSAVED_COPY_PREFIX}{secs}.kdbx"));
        fs::write(&taken, b"somebody else's file").unwrap();
        let other = another_vault(&f);
        f.session.open_vault(&other, pw("x")).unwrap();
        assert_eq!(fs::read(&taken).unwrap(), b"somebody else's file");
        let copy = folder.join(format!("{UNSAVED_COPY_PREFIX}{secs}-1.kdbx"));
        assert_eq!(disk_titles(&copy, "pw"), v(&["A", "first"]));
        let leftovers: Vec<_> = fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no temporary file left: {leftovers:?}"
        );
    }

    #[test]
    fn unsaved_copy_goes_through_the_file_io_seam() {
        let (f, io) = lock_with_kept_edits_and_external_change();
        let other = another_vault(&f);
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        assert!(f.session.open_vault(&other, pw("x")).is_err());
        assert!(
            f.has_pending_bytes(),
            "the switch is refused and the edits kept"
        );
        let folder = f
            .session
            .0
            .backups
            .folder(&f.session.vault_id().unwrap())
            .unwrap();
        let copies = fs::read_dir(&folder)
            .map(|d| {
                d.filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(UNSAVED_COPY_PREFIX)
                })
                .count()
            })
            .unwrap_or(0);
        assert_eq!(copies, 0);
    }

    // --- Retries and backups ----------------------------------------------------------------

    fn every_save(f: &Fx) {
        f.settings
            .update(SettingsPatch {
                backup_interval: Some(BackupInterval::EverySave),
                ..SettingsPatch::default()
            })
            .unwrap();
    }

    #[test]
    fn failed_autosave_retries_take_one_backup() {
        let (f, io) = failing_setup();
        every_save(&f);
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.add("A").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        for _ in 0..5 {
            f.clock.advance(PENDING_RETRY);
        }
        assert_eq!(f.state(), SaveState::Failed);
        assert_eq!(f.backups(), 1, "one copy of the unchanged disk");
        io.heal();
        f.clock.advance(PENDING_RETRY);
        assert_eq!(f.disk(), v(&["A", "first"]));
        assert_eq!(f.backups(), 1);
        f.add("B").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        assert_eq!(f.backups(), 2, "a new disk state is backed up again");
    }

    #[test]
    fn failed_pending_retries_take_one_backup() {
        let (f, io) = failing_setup();
        every_save(&f);
        f.add("A").unwrap();
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        f.session.lock(LockReason::Manual);
        assert!(f.has_pending_bytes());
        for _ in 0..5 {
            f.clock.advance(PENDING_RETRY);
        }
        assert_eq!(f.backups(), 1);
        io.heal();
        f.clock.advance(PENDING_RETRY);
        assert!(!f.has_pending_bytes());
        assert_eq!(f.disk(), v(&["A", "first"]));
        assert_eq!(f.backups(), 1);
    }

    /// The vault file as it is now, encrypted with `password`, holding one entry per title.
    fn vault_bytes(titles: &[&str], password: &str) -> Vec<u8> {
        let mut db = fast_database("Test Vault");
        let root = db.root_id();
        for t in titles {
            let mut e = Entry::new(Timestamp(T0 - 10));
            e.set_field("Title", t, false);
            db.add_entry(root, e, Timestamp(T0 - 10)).unwrap();
        }
        db.save(&pw(password)).unwrap()
    }

    #[test]
    fn restore_from_backup_replaces_the_database_and_snapshots_the_current_file() {
        let f = setup();
        f.add("unsaved").unwrap();
        let log = record(&f.session);
        let before = fs::read(&f.path).unwrap();
        f.session
            .restore_from_backup(&vault_bytes(&["old"], "pw"))
            .unwrap();
        assert_eq!(f.titles(), v(&["old"]));
        assert_eq!(f.disk(), v(&["old"]));
        assert_eq!(f.state(), SaveState::Idle);
        assert_eq!(f.gens().0, f.gens().1, "nothing is pending");
        assert!(log.lock().unwrap().contains(&SessionEvent::VaultChanged));
        // The flushed state (the file being replaced) was kept as a backup first.
        let id = f.session.vault_id().unwrap();
        let backups = f.session.backups();
        let kept: Vec<Vec<u8>> = backups
            .list(&id)
            .iter()
            .map(|b| backups.read(&id, &b.id).unwrap())
            .collect();
        assert_eq!(
            kept.len(),
            2,
            "the open-time snapshot and the pre-restore one"
        );
        assert!(kept.iter().any(|bytes| {
            Database::open(bytes, &pw("pw"))
                .map(|d| titles(&d) == v(&["first", "unsaved"]))
                .unwrap_or(false)
        }));
        assert_ne!(fs::read(&f.path).unwrap(), before);
    }

    #[test]
    fn restore_from_backup_with_another_password_touches_nothing() {
        let f = setup();
        f.add("A").unwrap();
        f.clock.advance(AUTOSAVE_DEBOUNCE);
        let file = fs::read(&f.path).unwrap();
        let live = f.session.with_db(Clone::clone).unwrap();
        let backups_before = f.backups();
        assert_eq!(
            f.session.restore_from_backup(&vault_bytes(&["x"], "other")),
            Err(SessionError::InvalidInput(
                "backup does not open with the current master password".into()
            ))
        );
        assert_eq!(fs::read(&f.path).unwrap(), file);
        assert_eq!(f.session.with_db(Clone::clone).unwrap(), live);
        assert_eq!(f.backups(), backups_before);
        assert_eq!(
            f.session.restore_from_backup(b"not a vault"),
            Err(SessionError::Corrupt("not a KDBX file".into())),
        );
    }

    #[test]
    fn restore_from_backup_is_refused_while_locked_and_while_quitting() {
        let f = setup();
        let bytes = vault_bytes(&["x"], "pw");
        f.session.begin_quit();
        assert_eq!(
            f.session.restore_from_backup(&bytes),
            Err(SessionError::Quitting)
        );
        f.session.cancel_quit();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.session.restore_from_backup(&bytes),
            Err(SessionError::Locked)
        );
    }
}
