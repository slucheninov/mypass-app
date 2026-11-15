use super::*;
use crate::app_core::ErrorCode;
use crate::scheduler::{ManualScheduler, ThreadScheduler};
use crate::session::{LockReason, SessionDeps};
use crate::testing::fast_database;
use mypass_kdbx::test_util::FailingFileIo;
use mypass_kdbx::{Entry, PlainFileIo, Timestamp};
use secrecy::SecretString;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::mpsc;
use std::time::SystemTime;
use tempfile::TempDir;

const FILE: &str = "main.kdbx";

fn pw() -> SecretString {
    SecretString::from("secondary-test-pw")
}

fn start() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// A folder watcher the tests fire by hand (no OS events, so runs are deterministic).
#[derive(Default)]
struct FakeWatch {
    watches: Mutex<Vec<(PathBuf, WatchCallback, Arc<AtomicBool>)>>,
    /// The next this many starts fail (watch resources exhausted).
    fail_next: AtomicUsize,
    /// Every start attempt.
    attempts: AtomicUsize,
}

struct FakeGuard(Arc<AtomicBool>);

impl WatchGuard for FakeGuard {
    fn stop(self: Box<Self>) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl FolderWatch for FakeWatch {
    fn watch(
        &self,
        dir: &Path,
        _file_name: &str,
        on_change: WatchCallback,
    ) -> Result<Box<dyn WatchGuard>, StorageError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if !dir.is_dir() {
            return Err(StorageError::Unavailable("missing".into()));
        }
        let failing = self
            .fail_next
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if failing {
            return Err(StorageError::Io("no watch resources".into()));
        }
        let active = Arc::new(AtomicBool::new(true));
        self.watches
            .lock()
            .unwrap()
            .push((dir.to_path_buf(), on_change, active.clone()));
        Ok(Box::new(FakeGuard(active)))
    }
}

impl FakeWatch {
    fn active(&self) -> usize {
        self.watches
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, _, a)| a.load(Ordering::SeqCst))
            .count()
    }

    /// Fires every running watcher, like a debounced change event.
    fn fire(&self) {
        let callbacks: Vec<WatchCallback> = self
            .watches
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, _, a)| a.load(Ordering::SeqCst))
            .map(|(_, c, _)| c.clone())
            .collect();
        for c in callbacks {
            c();
        }
    }

    /// Fires every watcher ever started, stopped ones included (a late OS event).
    fn fire_all(&self) {
        let callbacks: Vec<WatchCallback> = self
            .watches
            .lock()
            .unwrap()
            .iter()
            .map(|(_, c, _)| c.clone())
            .collect();
        for c in callbacks {
            c();
        }
    }
}

struct World {
    tmp: TempDir,
    scheduler: Arc<ManualScheduler>,
    settings: SettingsStore,
    session: VaultSession,
    sync: SecondarySync,
    watch: Arc<FakeWatch>,
    primary: PathBuf,
    folder: PathBuf,
    statuses: Arc<Mutex<Vec<SecondaryStatus>>>,
}

fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("secondary");
    world_in(tmp, folder)
}

/// A world whose secondary folder is `folder` (created; the setting holds it as given).
fn world_in(tmp: TempDir, folder: PathBuf) -> World {
    fs::create_dir_all(&folder).unwrap();
    let vaults = tmp.path().join("vaults");
    fs::create_dir_all(&vaults).unwrap();
    let primary = vaults.join(FILE);
    let scheduler = ManualScheduler::new(start());
    let settings = SettingsStore::in_memory();
    settings
        .set_secondary_folder(Some(folder.to_string_lossy().into_owned()))
        .unwrap();
    let session = VaultSession::new(SessionDeps {
        settings: settings.clone(),
        scheduler: scheduler.clone(),
        backups_root: tmp.path().join("backups"),
        file_io: Arc::new(PlainFileIo),
    });
    let paths = AppPaths::macos(&tmp.path().join("home"));
    let watch = Arc::new(FakeWatch::default());
    let sync = SecondarySync::with_watch(
        session.clone(),
        settings.clone(),
        scheduler.clone(),
        &paths,
        watch.clone(),
    );
    settle_after_every_job(&scheduler, &sync);
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let log = statuses.clone();
    sync.add_observer(Arc::new(move |s: &SecondaryStatus| {
        log.lock().unwrap().push(s.clone())
    }));
    sync.attach();
    World {
        tmp,
        scheduler,
        settings,
        session,
        sync,
        watch,
        primary,
        folder,
        statuses,
    }
}

/// The deadlock bound of every wait on the worker (never a timing rule).
const WORKER_BOUND: Duration = Duration::from_secs(20);

/// Every scheduler job of the service only queues a run on its worker; waiting for the worker
/// after each job keeps the manual timeline exactly as if the job had run the sync inline.
fn settle_after_every_job(scheduler: &ManualScheduler, sync: &SecondarySync) {
    let weak = Arc::downgrade(&sync.0);
    scheduler.set_after_job(Some(Arc::new(move || {
        if let Some(shared) = weak.upgrade() {
            assert!(
                SecondarySync(shared).wait_idle(WORKER_BOUND),
                "the sync worker did not go idle"
            );
        }
    })));
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

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn add_to(db: &mut Database, title: &str, at: Timestamp) {
    let mut e = Entry::new(at);
    e.set_field("Title", title, false);
    let root = db.root_id();
    db.add_entry(root, e, at).unwrap();
}

impl World {
    /// Creates and unlocks the primary vault, then runs the arm sync.
    fn open(&self) {
        self.session
            .create_vault_with(&self.primary, pw(), fast_database("Main"))
            .unwrap();
        self.advance_ms(0);
    }

    fn advance_ms(&self, ms: u64) {
        self.scheduler.advance(Duration::from_millis(ms));
    }

    fn advance(&self, secs: u64) {
        self.scheduler.advance(Duration::from_secs(secs));
    }

    fn remote(&self) -> PathBuf {
        self.folder.join(FILE)
    }

    fn remote_bytes(&self) -> Vec<u8> {
        fs::read(self.remote()).unwrap()
    }

    fn remote_titles(&self) -> Vec<String> {
        titles(&Database::open(&self.remote_bytes(), &pw()).unwrap())
    }

    fn primary_titles(&self) -> Vec<String> {
        titles(&Database::open(&fs::read(&self.primary).unwrap(), &pw()).unwrap())
    }

    fn live_titles(&self) -> Vec<String> {
        self.session.with_db(titles).unwrap()
    }

    fn add(&self, title: &str) {
        self.session
            .mutate(|db, now| {
                add_to(db, title, now);
                Ok(())
            })
            .unwrap();
    }

    /// Another device edits the secondary copy through its own handle.
    fn foreign_add(&self, title: &str) {
        let mut db = Database::open(&self.remote_bytes(), &pw()).unwrap();
        add_to(&mut db, title, Timestamp(1_700_000_500));
        fs::write(self.remote(), db.save(&pw()).unwrap()).unwrap();
    }

    fn engine_runs(&self) -> usize {
        self.sync.probe().engine_runs.load(Ordering::SeqCst)
    }

    fn checkpoint_file(&self) -> PathBuf {
        AppPaths::macos(&self.tmp.path().join("home"))
            .data_dir
            .join(SYNC_STATE_DIR)
            .join(self.session.vault_id().unwrap())
            .join(secondary_checkpoint_file(
                &self.session.vault_path().unwrap(),
            ))
    }

    fn idle_synced(&self) -> bool {
        matches!(
            self.sync.status(),
            SecondaryStatus::Idle { last_sync: Some(_) }
        )
    }

    /// Moves the secondary folder away (an unmounted drive) and returns where it went.
    fn unplug(&self) -> PathBuf {
        let away = self.tmp.path().join("unplugged");
        fs::rename(&self.folder, &away).unwrap();
        away
    }
}

#[test]
fn arming_on_unlock_uploads_the_initial_copy_and_watches_the_folder() {
    let w = world();
    assert_eq!(w.sync.status(), SecondaryStatus::Idle { last_sync: None });
    w.open();
    assert!(w.remote().is_file());
    assert_eq!(
        w.sync.status(),
        SecondaryStatus::Idle {
            last_sync: Some(1_700_000_000_000.0)
        },
        "unix milliseconds of the injected clock"
    );
    assert_eq!(w.watch.active(), 1);
}

#[test]
fn save_triggers_push_after_two_seconds_not_before() {
    let w = world();
    w.open();
    w.add("A");
    w.advance(1);
    assert_eq!(w.primary_titles(), v(&["A"]), "the autosave ran");
    w.advance_ms(1_999);
    assert_eq!(w.remote_titles(), Vec::<String>::new());
    w.advance_ms(1);
    assert_eq!(w.remote_titles(), v(&["A"]));
}

#[test]
fn push_debounce_restarts_on_every_save() {
    let w = world();
    w.open();
    w.add("A");
    w.advance(1); // first save at t = 1 s: push due at 3 s
    w.advance_ms(500);
    w.add("B");
    w.advance(1); // second save at t = 2.5 s: push moves to 4.5 s
    w.advance_ms(1_000);
    // t = 3.5 s: past the first deadline, before the second.
    assert_eq!(w.remote_titles(), Vec::<String>::new());
    w.advance(1);
    assert_eq!(w.remote_titles(), v(&["A", "B"]));
}

#[test]
fn push_creates_remote_file_that_opens_with_same_password() {
    let w = world();
    w.open();
    w.add("A");
    w.advance(3);
    let remote = Database::open(&w.remote_bytes(), &pw()).unwrap();
    let live_root = w.session.with_db(|db| db.root_id()).unwrap();
    assert_eq!(remote.root_id(), live_root);
    assert_eq!(titles(&remote), v(&["A"]));
}

#[test]
fn secondary_failure_never_blocks_primary_save() {
    let w = world();
    w.open();
    w.unplug();
    w.add("A");
    w.advance(1);
    assert_eq!(w.primary_titles(), v(&["A"]));
    assert_eq!(w.session.status().save.state, SaveState::Idle);
    w.advance(2);
    let missing = SecondaryStatus::Unavailable {
        reason: SecondaryUnavailable::FolderMissing,
    };
    assert_eq!(w.sync.status(), missing);
    assert!(w.statuses.lock().unwrap().contains(&missing));
    assert!(!w.remote().exists(), "the folder is never re-created");
    assert_eq!(w.watch.active(), 0, "no watcher while unavailable");
}

#[test]
fn unavailable_retries_after_five_minutes_and_recovers() {
    let w = world();
    w.open();
    let away = w.unplug();
    w.add("A");
    w.advance(3);
    assert!(matches!(
        w.sync.status(),
        SecondaryStatus::Unavailable { .. }
    ));
    fs::rename(&away, &w.folder).unwrap();
    w.advance(SECONDARY_RETRY.as_secs() - 1);
    assert!(matches!(
        w.sync.status(),
        SecondaryStatus::Unavailable { .. }
    ));
    assert_eq!(w.remote_titles(), Vec::<String>::new());
    w.advance(1);
    assert!(w.idle_synced());
    assert_eq!(w.remote_titles(), v(&["A"]));
    assert_eq!(w.watch.active(), 1, "the watcher is back");
}

#[test]
fn remote_change_merges_into_live_session_and_primary_file() {
    let w = world();
    w.open();
    w.foreign_add("B");
    w.watch.fire();
    w.advance_ms(0);
    assert_eq!(w.live_titles(), v(&["B"]));
    // The run itself makes the merge durable in the primary before it commits its checkpoint.
    assert_eq!(
        w.primary_titles(),
        v(&["B"]),
        "the merge reaches the primary"
    );
    assert!(w.idle_synced());
}

#[test]
fn own_write_echo_is_ignored_by_watcher() {
    let w = world();
    w.open();
    w.add("A");
    w.advance(3);
    let runs = w.engine_runs();
    w.watch.fire();
    w.advance_ms(0);
    assert_eq!(w.engine_runs(), runs, "an echo makes no engine call");
    // Positive control: a foreign change does.
    w.foreign_add("B");
    w.watch.fire();
    w.advance_ms(0);
    assert_eq!(w.engine_runs(), runs + 1);
}

#[test]
fn edit_during_sync_is_kept_after_adopt() {
    let w = world();
    w.open();
    w.foreign_add("B");
    let session = w.session.clone();
    let mut once = true;
    *w.sync.probe().before_adopt.lock().unwrap() = Some(Box::new(move || {
        if std::mem::take(&mut once) {
            session
                .mutate(|db, now| {
                    add_to(db, "C", now);
                    Ok(())
                })
                .unwrap();
        }
    }));
    w.watch.fire();
    w.advance_ms(0);
    assert_eq!(w.live_titles(), v(&["B", "C"]));
    w.advance(1);
    assert_eq!(w.primary_titles(), v(&["B", "C"]));
    w.advance(2);
    assert_eq!(w.remote_titles(), v(&["B", "C"]));
}

#[test]
fn failed_adoption_commits_nothing_and_the_next_sync_merges_again() {
    let w = world();
    w.open();
    let checkpoint = fs::read(w.checkpoint_file()).unwrap();
    w.foreign_add("B");
    let session = w.session.clone();
    *w.sync.probe().before_adopt.lock().unwrap() = Some(Box::new(move || {
        session.lock(LockReason::Manual);
    }));
    w.watch.fire();
    w.advance_ms(0);
    assert!(!w.session.is_unlocked());
    assert_eq!(
        fs::read(w.checkpoint_file()).unwrap(),
        checkpoint,
        "no checkpoint for a merge that was not adopted"
    );
    *w.sync.probe().before_adopt.lock().unwrap() = None;
    w.session.unlock(pw()).unwrap();
    w.advance_ms(0);
    assert_eq!(w.live_titles(), v(&["B"]));
    assert_eq!(w.remote_titles(), v(&["B"]));
}

#[test]
fn wrong_password_remote_is_never_overwritten() {
    let w = world();
    w.open();
    let db = Database::open(&w.remote_bytes(), &pw()).unwrap();
    fs::write(
        w.remote(),
        db.save(&SecretString::from("another-password")).unwrap(),
    )
    .unwrap();
    let foreign = w.remote_bytes();
    w.watch.fire();
    w.advance_ms(0);
    let wrong = SecondaryStatus::Error {
        code: SecondaryError::WrongPassword,
    };
    assert_eq!(w.sync.status(), wrong);
    w.add("A");
    w.advance(3);
    assert_eq!(w.sync.status(), wrong);
    assert_eq!(w.remote_bytes(), foreign);
    assert_eq!(w.primary_titles(), v(&["A"]));
    assert_eq!(w.scheduler.pending(), 0, "no retry for a wrong password");
}

#[test]
fn lock_stops_watcher_and_pushes_nothing() {
    let w = world();
    w.open();
    assert_eq!(w.watch.active(), 1);
    let runs = w.engine_runs();
    w.add("A");
    w.session.lock(LockReason::Manual);
    assert_eq!(w.primary_titles(), v(&["A"]), "lock saved the primary");
    assert_eq!(w.watch.active(), 0);
    w.watch.fire_all();
    w.advance(10);
    assert_eq!(w.engine_runs(), runs);
    assert_eq!(w.remote_titles(), Vec::<String>::new());
    assert!(matches!(w.sync.status(), SecondaryStatus::Idle { .. }));
    assert_eq!(w.scheduler.pending(), 0);
}

#[test]
fn settings_change_to_none_stops_everything_and_status_off() {
    let w = world();
    w.open();
    w.settings.set_secondary_folder(None).unwrap();
    w.sync.rearm();
    assert_eq!(w.sync.status(), SecondaryStatus::Off);
    assert_eq!(w.watch.active(), 0);
    let runs = w.engine_runs();
    w.add("A");
    w.advance(5);
    assert_eq!(w.engine_runs(), runs);
    assert_eq!(w.remote_titles(), Vec::<String>::new());
}

#[test]
fn a_setting_cleared_without_rearm_is_followed_by_the_next_run() {
    let w = world();
    w.open();
    w.settings.set_secondary_folder(None).unwrap();
    let runs = w.engine_runs();
    w.add("A");
    w.advance(3);
    assert_eq!(w.engine_runs(), runs);
    assert_eq!(w.sync.status(), SecondaryStatus::Off);
    assert_eq!(w.watch.active(), 0);
    assert_eq!(w.remote_titles(), Vec::<String>::new());
}

#[test]
fn remote_of_another_vault_is_refused_and_left_alone() {
    // Stage 1's merge aliases different roots; the engine refuses them instead (batch C).
    let w = world();
    let mut other = fast_database("Other");
    add_to(&mut other, "Foreign", Timestamp(1_700_000_000));
    fs::write(w.remote(), other.save(&pw()).unwrap()).unwrap();
    let foreign = w.remote_bytes();
    w.open();
    assert_eq!(
        w.sync.status(),
        SecondaryStatus::Error {
            code: SecondaryError::DifferentVault
        }
    );
    assert_eq!(w.remote_bytes(), foreign);
    assert_eq!(w.live_titles(), Vec::<String>::new());
}

#[test]
fn not_a_vault_remote_is_never_overwritten() {
    let w = world();
    fs::write(w.remote(), b"not a vault at all").unwrap();
    w.open();
    assert_eq!(
        w.sync.status(),
        SecondaryStatus::Error {
            code: SecondaryError::NotAVault
        }
    );
    assert_eq!(w.remote_bytes(), b"not a vault at all");
}

#[cfg(unix)]
#[test]
fn read_only_secondary_dir_reports_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let w = world();
    fs::set_permissions(&w.folder, fs::Permissions::from_mode(0o555)).unwrap();
    w.open();
    let status = w.sync.status();
    fs::set_permissions(&w.folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        status,
        SecondaryStatus::Unavailable {
            reason: SecondaryUnavailable::ReadOnly
        }
    );
    assert!(!w.remote().exists());
    w.add("A");
    w.advance(1);
    assert_eq!(w.primary_titles(), v(&["A"]));
}

#[test]
fn secondary_folder_with_non_ascii_and_long_path_syncs() {
    let tmp = tempfile::tempdir().unwrap();
    // Cyrillic, a combining accent and a space, spelled as escapes (the Cyrillic guard).
    let mut folder = tmp
        .path()
        .join("\u{0421}\u{0435}\u{0439}\u{0444} e\u{0301}t\u{00e9}");
    while folder.as_os_str().len() <= 300 {
        folder = folder.join("a-long-folder-name-for-the-path-limit");
    }
    let w = world_in(tmp, folder);
    w.open();
    assert!(w.idle_synced());
    w.add("A");
    w.advance(3);
    assert_eq!(w.remote_titles(), v(&["A"]));
}

#[test]
fn secondary_folder_setting_is_the_2a_field() {
    let w = world();
    let json = serde_json::to_value(w.settings.get()).unwrap();
    assert_eq!(
        json["secondaryFolder"].as_str(),
        Some(w.folder.to_string_lossy().as_ref())
    );
    w.open();
    assert!(w.remote().is_file(), "the sync used that folder");
}

#[test]
fn invalid_folders_are_unavailable_and_never_written() {
    let w = world();
    let invalid = SecondaryStatus::Unavailable {
        reason: SecondaryUnavailable::InvalidFolder,
    };
    w.open();
    let vaults = w.primary.parent().unwrap().to_path_buf();
    let a_file = w.tmp.path().join("a-file");
    fs::write(&a_file, b"x").unwrap();
    let cases = [
        "relative/folder".to_owned(),
        String::new(),
        vaults.to_string_lossy().into_owned(),
        format!("{}/../vaults", vaults.to_string_lossy()),
        a_file.to_string_lossy().into_owned(),
    ];
    for case in cases {
        w.settings.set_secondary_folder(Some(case.clone())).unwrap();
        w.sync.rearm();
        w.advance_ms(0);
        assert_eq!(w.sync.status(), invalid, "{case}");
        assert_eq!(w.scheduler.pending(), 0, "no retry for {case}");
    }
    assert_eq!(fs::read(&a_file).unwrap(), b"x");
    assert_eq!(w.primary_titles(), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn a_symlink_to_the_primary_folder_is_invalid() {
    let w = world();
    w.open();
    let link = w.tmp.path().join("alias");
    std::os::unix::fs::symlink(w.primary.parent().unwrap(), &link).unwrap();
    w.settings
        .set_secondary_folder(Some(link.to_string_lossy().into_owned()))
        .unwrap();
    w.sync.rearm();
    w.advance_ms(0);
    assert_eq!(
        w.sync.status(),
        SecondaryStatus::Unavailable {
            reason: SecondaryUnavailable::InvalidFolder
        }
    );
}

#[test]
fn sync_now_needs_an_open_vault_and_answers_off_without_a_folder() {
    let w = world();
    let err = w.sync.sync_now().unwrap_err();
    assert_eq!(err.code, ErrorCode::NoVault);
    w.open();
    w.session.lock(LockReason::Manual);
    assert_eq!(w.sync.sync_now().unwrap_err().code, ErrorCode::Locked);
    w.settings.set_secondary_folder(None).unwrap();
    assert_eq!(w.sync.sync_now().unwrap(), SecondaryStatus::Off);
}

#[test]
fn sync_now_pushes_and_pulls() {
    let w = world();
    w.open();
    w.foreign_add("B");
    w.add("A");
    let status = w.sync.sync_now().unwrap();
    assert!(matches!(
        status,
        SecondaryStatus::Idle { last_sync: Some(_) }
    ));
    assert_eq!(w.live_titles(), v(&["A", "B"]));
    assert_eq!(w.remote_titles(), v(&["A", "B"]));
}

#[test]
fn disconnect_clears_the_setting_and_checkpoint_and_leaves_the_copy() {
    let w = world();
    w.open();
    let copy = w.remote_bytes();
    w.sync.disconnect().unwrap();
    assert_eq!(w.settings.get().secondary_folder, None);
    assert_eq!(w.sync.status(), SecondaryStatus::Off);
    assert_eq!(w.watch.active(), 0);
    assert_eq!(w.remote_bytes(), copy);
    assert_eq!(fs::read(w.checkpoint_file()).unwrap(), b"null");
}

#[test]
fn switching_vaults_rearms_for_the_new_file() {
    let w = world();
    w.open();
    let second = w.primary.with_file_name("second.kdbx");
    w.session
        .create_vault_with(&second, pw(), fast_database("Second"))
        .unwrap();
    w.advance_ms(0);
    assert!(w.folder.join("second.kdbx").is_file());
    assert_eq!(w.watch.active(), 1);
    // The old vault's copy is left alone.
    assert!(w.remote().is_file());
}

#[test]
fn quitting_runs_no_sync() {
    let w = world();
    w.open();
    w.add("A");
    w.advance(1);
    w.session.begin_quit();
    let runs = w.engine_runs();
    w.advance(5);
    assert_eq!(w.engine_runs(), runs);
}

#[test]
fn status_serialises_with_a_state_tag() {
    let json = |s: SecondaryStatus| serde_json::to_value(s).unwrap();
    assert_eq!(
        json(SecondaryStatus::Off),
        serde_json::json!({"state": "off"})
    );
    assert_eq!(
        json(SecondaryStatus::Idle {
            last_sync: Some(5.0)
        }),
        serde_json::json!({"state": "idle", "lastSync": 5.0})
    );
    assert_eq!(
        json(SecondaryStatus::Unavailable {
            reason: SecondaryUnavailable::FolderMissing
        }),
        serde_json::json!({"state": "unavailable", "reason": "folderMissing"})
    );
    assert_eq!(
        json(SecondaryStatus::Error {
            code: SecondaryError::WrongPassword
        }),
        serde_json::json!({"state": "error", "code": "wrongPassword"})
    );
}

impl World {
    /// A crash and a restart: everything in memory is dropped without any further write, then a
    /// new app (same data folders) opens the same primary file and arms.
    fn restart(self) -> World {
        let World {
            tmp,
            folder,
            session,
            sync,
            scheduler,
            ..
        } = self;
        drop(sync);
        drop(session);
        drop(scheduler);
        let w = world_in(tmp, folder);
        w.session.open_vault(&w.primary, pw()).unwrap();
        w.advance_ms(0);
        w
    }

    fn set_hook(&self, hook: &Hook, f: impl FnMut() + Send + 'static) {
        let _ = self;
        *hook.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(f));
    }
}

/// D-2 crash points: the merge of a local "A" with the copy's "B" is on the copy, then the app
/// dies at `point`. After a restart nothing is lost: "B" reaches the primary and both stay on
/// the copy (an idempotent re-merge, never a blind push of the stale primary).
fn crash_at(point: fn(&Probe) -> &Hook) {
    let w = world();
    w.open();
    w.foreign_add("B");
    let reached = Arc::new(AtomicBool::new(false));
    let flag = reached.clone();
    w.set_hook(point(w.sync.probe()), move || {
        flag.store(true, Ordering::SeqCst);
        panic!("simulated crash")
    });
    w.add("A");
    w.advance(1);
    // The panic ends the run on the worker (contained there, like a process that died at this
    // point: nothing after it runs).
    w.advance(SECONDARY_PUSH_DEBOUNCE.as_secs());
    assert!(
        reached.load(Ordering::SeqCst),
        "the crash point was reached"
    );
    assert_eq!(w.remote_titles(), v(&["A", "B"]));
    let w = w.restart();
    assert_eq!(w.live_titles(), v(&["A", "B"]));
    assert_eq!(w.primary_titles(), v(&["A", "B"]));
    assert_eq!(w.remote_titles(), v(&["A", "B"]));
    // The re-merge recorded a checkpoint again: an edit is now pushed plainly.
    w.add("C");
    w.advance(1 + SECONDARY_PUSH_DEBOUNCE.as_secs());
    assert_eq!(w.remote_titles(), v(&["A", "B", "C"]));
}

#[test]
fn crash_before_the_merge_is_adopted_loses_nothing() {
    crash_at(|p| &p.before_adopt);
}

#[test]
fn crash_before_the_adopted_merge_is_on_disk_loses_nothing() {
    crash_at(|p| &p.before_durable);
}

#[test]
fn crash_before_the_checkpoint_is_committed_loses_nothing() {
    crash_at(|p| &p.before_commit);
}

/// D-2: a primary that cannot be written keeps the checkpoint as it was, so a restart (with the
/// adopted entry lost from memory) merges the copy again instead of pushing over it.
#[test]
fn primary_write_failure_keeps_the_checkpoint_and_a_restart_merges_again() {
    let w = world();
    w.open();
    let checkpoint = fs::read(w.checkpoint_file()).unwrap();
    let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
    io.fail_write_atomic(std::io::ErrorKind::PermissionDenied);
    w.session.set_file_io(io);
    w.foreign_add("B");
    w.watch.fire();
    w.advance_ms(0);
    assert_eq!(w.live_titles(), v(&["B"]), "adopted in memory");
    assert!(w.primary_titles().is_empty(), "not on disk");
    assert_eq!(
        fs::read(w.checkpoint_file()).unwrap(),
        checkpoint,
        "no checkpoint while the primary lacks the merge"
    );
    let w = w.restart();
    assert_eq!(w.live_titles(), v(&["B"]));
    assert_eq!(w.primary_titles(), v(&["B"]));
    assert_eq!(w.remote_titles(), v(&["B"]));
}

/// D-1: the engine (remote I/O, KDF) runs without the session lock: an edit made from another
/// thread in the middle of the engine call completes at once.
#[test]
fn the_session_stays_usable_while_the_engine_runs() {
    let w = world();
    w.open();
    w.add("A");
    let session = w.session.clone();
    let mut once = true;
    w.set_hook(&w.sync.probe().during_engine, move || {
        if !std::mem::take(&mut once) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let s = session.clone();
        std::thread::spawn(move || {
            let r = s.mutate(|db, now| {
                add_to(db, "C", now);
                Ok(())
            });
            let _ = tx.send(r.is_ok());
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(20)),
            Ok(true),
            "an edit during the engine call did not complete: the session lock is held"
        );
    });
    w.sync.sync_now().unwrap();
    assert_eq!(w.live_titles(), v(&["A", "C"]));
    assert_eq!(w.remote_titles(), v(&["A"]));
    w.advance(1 + SECONDARY_PUSH_DEBOUNCE.as_secs());
    assert_eq!(w.remote_titles(), v(&["A", "C"]));
}

/// D-1 and D-3: a lock from another thread during the engine call completes at once, and the
/// run then writes nothing and records nothing for the locked vault.
#[test]
fn lock_during_the_engine_call_is_not_blocked_and_cancels_the_write() {
    let w = world();
    w.open();
    let checkpoint = fs::read(w.checkpoint_file()).unwrap();
    let remote = w.remote_bytes();
    w.add("A");
    let session = w.session.clone();
    let mut once = true;
    w.set_hook(&w.sync.probe().during_engine, move || {
        if !std::mem::take(&mut once) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let s = session.clone();
        std::thread::spawn(move || {
            s.lock(LockReason::Manual);
            let _ = tx.send(());
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(20)),
            Ok(()),
            "lock during the engine call did not complete: the session lock is held"
        );
    });
    w.sync.sync_now().unwrap();
    assert!(!w.session.is_unlocked());
    assert_eq!(w.remote_bytes(), remote, "nothing written after the lock");
    assert_eq!(fs::read(w.checkpoint_file()).unwrap(), checkpoint);
    assert_eq!(w.primary_titles(), v(&["A"]), "the lock saved the edit");
}

/// D-3: a vault switched during the run (here a copy with the same root UUID) never receives
/// the run's merge.
#[test]
fn merge_is_never_adopted_into_a_vault_switched_during_the_run() {
    let w = world();
    w.open();
    w.foreign_add("B");
    let copy = w.tmp.path().join("vaults").join("copy.kdbx");
    fs::copy(&w.primary, &copy).unwrap();
    let session = w.session.clone();
    let settings = w.settings.clone();
    let mut once = true;
    w.set_hook(&w.sync.probe().before_adopt, move || {
        if std::mem::take(&mut once) {
            settings.set_secondary_folder(None).unwrap();
            session.open_vault(&copy, pw()).unwrap();
        }
    });
    w.watch.fire();
    w.advance_ms(0);
    assert_eq!(
        w.session.vault_path().unwrap().file_name().unwrap(),
        "copy.kdbx"
    );
    assert!(w.live_titles().is_empty(), "nothing adopted into the copy");
    w.advance(5);
    let copy_db = Database::open(
        &fs::read(w.tmp.path().join("vaults").join("copy.kdbx")).unwrap(),
        &pw(),
    )
    .unwrap();
    assert!(titles(&copy_db).is_empty());
}

/// D-4: a watcher that cannot start shows in the status while the data still syncs, and is
/// retried on the scheduler with a growing delay until it runs, without any local edit.
#[test]
fn watcher_start_failure_is_reported_and_retried_until_it_runs() {
    let w = world();
    w.watch.fail_next.store(2, Ordering::SeqCst);
    w.open();
    w.advance(SECONDARY_PUSH_DEBOUNCE.as_secs() + 1);
    assert!(w.remote().exists(), "the data sync still ran");
    assert_eq!(w.watch.active(), 0);
    assert_eq!(
        w.sync.status(),
        SecondaryStatus::Error {
            code: SecondaryError::WatchFailed
        }
    );
    let attempts = w.watch.attempts.load(Ordering::SeqCst);
    // Every failure doubles the delay: no attempt runs back to back.
    w.advance(SECONDARY_WATCH_RETRY_MIN.as_secs() * 8);
    assert_eq!(w.watch.active(), 1, "the watcher runs again");
    assert!(w.watch.attempts.load(Ordering::SeqCst) <= attempts + 2);
    assert!(w.idle_synced(), "{:?}", w.sync.status());
    w.foreign_add("B");
    w.watch.fire();
    w.advance_ms(0);
    assert_eq!(w.live_titles(), v(&["B"]));
}

#[test]
fn watcher_retry_delay_doubles_up_to_the_retry_interval() {
    let w = world();
    w.watch.fail_next.store(usize::MAX, Ordering::SeqCst);
    w.open();
    w.advance(SECONDARY_PUSH_DEBOUNCE.as_secs() + 1);
    let mut gaps = Vec::new();
    let mut last = w.watch.attempts.load(Ordering::SeqCst);
    let mut since = 0;
    for _ in 0..(SECONDARY_RETRY.as_secs() * 3) {
        w.advance(1);
        since += 1;
        let now = w.watch.attempts.load(Ordering::SeqCst);
        if now != last {
            gaps.push(since);
            since = 0;
            last = now;
        }
    }
    let first = SECONDARY_WATCH_RETRY_MIN.as_secs();
    assert!(gaps.len() >= 4, "{gaps:?}");
    assert!(gaps[0] <= first * 2, "{gaps:?}");
    for pair in gaps.windows(2) {
        assert!(pair[1] >= pair[0], "{gaps:?}");
    }
    assert!(
        gaps.iter().all(|g| *g <= SECONDARY_RETRY.as_secs()),
        "{gaps:?}"
    );
}

/// D-6: locking and unlocking with nothing new writes nothing to the copy; a change that
/// reached the primary while locked is still pushed on unlock (positive control).
#[test]
fn unlock_with_nothing_new_does_not_rewrite_the_copy() {
    let w = world();
    w.open();
    w.add("A");
    w.advance(1 + SECONDARY_PUSH_DEBOUNCE.as_secs());
    let before = w.remote_bytes();
    w.session.lock(LockReason::Manual);
    w.session.unlock(pw()).unwrap();
    w.advance(1 + SECONDARY_PUSH_DEBOUNCE.as_secs());
    assert_eq!(w.remote_bytes(), before, "no fresh encoding on unlock");
    assert!(w.idle_synced());
    w.session.lock(LockReason::Manual);
    let mut db = Database::open(&fs::read(&w.primary).unwrap(), &pw()).unwrap();
    add_to(&mut db, "C", Timestamp(1_700_000_900));
    fs::write(&w.primary, db.save(&pw()).unwrap()).unwrap();
    w.session.unlock(pw()).unwrap();
    w.advance(1 + SECONDARY_PUSH_DEBOUNCE.as_secs());
    assert_eq!(w.remote_titles(), v(&["A", "C"]));
}

/// D-2: two copies of one vault (same root UUID) keep separate checkpoints.
#[test]
fn checkpoint_is_per_primary_file() {
    let a = secondary_checkpoint_file(Path::new("/Users/u/Vaults/main.kdbx"));
    let b = secondary_checkpoint_file(Path::new("/Users/u/Backup/main.kdbx"));
    assert_ne!(a, b);
    assert!(a.starts_with("secondary-") && a.ends_with(".json"), "{a}");
    assert_eq!(a.len(), "secondary-".len() + 16 + ".json".len());
}

/// D-5: only a path that does not exist is a missing folder; one that cannot be inspected is
/// invalid (not retried).
#[cfg(unix)]
#[test]
fn folder_that_cannot_be_inspected_is_invalid_not_missing() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let primary = tmp.path().join("vaults").join(FILE);
    let looped = tmp.path().join("loop");
    std::os::unix::fs::symlink(&looped, &looped).unwrap();
    assert_eq!(
        validate_folder(looped.to_str().unwrap(), &primary, FILE),
        Err(SecondaryUnavailable::InvalidFolder)
    );
    let closed = tmp.path().join("closed");
    fs::create_dir_all(closed.join("inner")).unwrap();
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    let denied = validate_folder(closed.join("inner").to_str().unwrap(), &primary, FILE);
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(denied, Err(SecondaryUnavailable::InvalidFolder));
    assert_eq!(
        validate_folder(tmp.path().join("gone").to_str().unwrap(), &primary, FILE),
        Err(SecondaryUnavailable::FolderMissing)
    );
}

// ---- CD-sched: runs happen on the service's own worker, never on the scheduler ----

/// A world on the production `ThreadScheduler`, with a `during_engine` hook that parks the first
/// run inside the engine (a hung network folder) until the test releases it.
struct Hung {
    tmp: TempDir,
    scheduler: Arc<ThreadScheduler>,
    session: VaultSession,
    sync: SecondarySync,
    watch: Arc<FakeWatch>,
    folder: PathBuf,
    entered: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

fn hung_world() -> Hung {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("secondary");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(tmp.path().join("vaults")).unwrap();
    let scheduler = ThreadScheduler::new("test-scheduler").unwrap();
    let settings = SettingsStore::in_memory();
    settings
        .set_secondary_folder(Some(folder.to_string_lossy().into_owned()))
        .unwrap();
    let session = VaultSession::new(SessionDeps {
        settings: settings.clone(),
        scheduler: scheduler.clone(),
        backups_root: tmp.path().join("backups"),
        file_io: Arc::new(PlainFileIo),
    });
    let watch = Arc::new(FakeWatch::default());
    let sync = SecondarySync::with_watch(
        session.clone(),
        settings,
        scheduler.clone(),
        &AppPaths::macos(&tmp.path().join("home")),
        watch.clone(),
    );
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let mut first = true;
    *sync.probe().during_engine.lock().unwrap() = Some(Box::new(move || {
        if std::mem::take(&mut first) {
            entered_tx.send(()).unwrap();
            // Bounded only against a deadlock of the test itself.
            let _ = release_rx.recv_timeout(WORKER_BOUND);
        }
    }));
    sync.attach();
    session
        .create_vault_with(
            &tmp.path().join("vaults").join(FILE),
            pw(),
            fast_database("Main"),
        )
        .unwrap();
    entered
        .recv_timeout(WORKER_BOUND)
        .expect("the arm run reached the engine");
    Hung {
        tmp,
        scheduler,
        session,
        sync,
        watch,
        folder,
        entered,
        release,
    }
}

impl Hung {
    /// Runs `f` on a helper thread; panics when it does not return within the bound.
    fn returns<R: Send + 'static>(&self, what: &str, f: impl FnOnce() -> R + Send + 'static) -> R {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(WORKER_BOUND)
            .unwrap_or_else(|_| panic!("{what} waited for the hung sync"))
    }

    /// Waits until every job queued on the scheduler so far has run.
    fn scheduler_barrier(&self) -> bool {
        let (tx, rx) = mpsc::channel();
        self.scheduler.schedule(
            Duration::ZERO,
            Box::new(move || {
                let _ = tx.send(());
            }),
        );
        rx.recv_timeout(WORKER_BOUND).is_ok()
    }

    fn release(&self) {
        let _ = self.release.send(());
        assert!(self.sync.wait_idle(WORKER_BOUND), "the worker went idle");
        let _ = &self.entered;
        let _ = &self.tmp;
    }
}

/// CD-sched: while a run hangs in the engine, the scheduler keeps running other jobs (auto-lock,
/// autosave), and a lock completes without waiting for the run, which then writes nothing.
#[test]
fn a_hung_sync_never_delays_the_scheduler_or_a_lock() {
    let h = hung_world();
    assert!(
        h.scheduler_barrier(),
        "a scheduler job waited for the hung sync"
    );
    let session = h.session.clone();
    h.returns("lock", move || session.lock(LockReason::Manual));
    let sync = h.sync.clone();
    h.returns("status", move || sync.status());
    h.release();
    assert!(!h.session.is_unlocked());
    assert!(
        !h.folder.join(FILE).exists(),
        "the run stopped without writing after the lock"
    );
}

/// CD-sched: at most one run is in flight; triggers that arrive meanwhile queue one request
/// each (duplicates dropped) and run after it, one at a time.
#[test]
fn at_most_one_run_is_in_flight_and_queued_triggers_coalesce() {
    let h = hung_world();
    for _ in 0..3 {
        h.watch.fire();
        assert!(h.scheduler_barrier());
    }
    assert_eq!(h.sync.probe().engine_runs.load(Ordering::SeqCst), 1);
    {
        let w = lock(&h.sync.0.worker.state);
        assert!(w.running);
        assert_eq!(w.queue.len(), 1, "three pulls coalesced into one request");
    }
    h.release();
    // The queued pull found this device's own upload (an echo): no second engine run.
    assert_eq!(h.sync.probe().engine_runs.load(Ordering::SeqCst), 1);
    assert!(h.folder.join(FILE).is_file());
}

/// CD-sched: `shutdown` returns while a run hangs; the run then stops without writing, the
/// worker ends and nothing runs afterwards.
#[test]
fn shutdown_returns_at_once_and_stops_the_worker() {
    let h = hung_world();
    let sync = h.sync.clone();
    h.returns("shutdown", move || sync.shutdown());
    h.release();
    assert!(lock(&h.sync.0.worker.state).shutdown);
    assert!(!h.folder.join(FILE).exists());
    assert_eq!(h.sync.sync_now().unwrap(), h.sync.status());
    h.watch.fire();
    assert!(h.scheduler_barrier());
    assert!(h.sync.wait_idle(WORKER_BOUND));
    assert_eq!(h.sync.probe().engine_runs.load(Ordering::SeqCst), 1);
    assert!(!h.folder.join(FILE).exists());
}

/// CD-sched: a vault switch while a run hangs is not blocked either, and the run never writes
/// the old vault's content after it.
#[test]
fn a_vault_switch_is_not_blocked_by_a_hung_sync() {
    let h = hung_world();
    let session = h.session.clone();
    let second = h.tmp.path().join("vaults").join("second.kdbx");
    h.returns("switch", move || {
        session
            .create_vault_with(&second, pw(), fast_database("Second"))
            .unwrap()
    });
    h.release();
    assert!(
        !h.folder.join(FILE).exists(),
        "nothing written for the old vault"
    );
}

// --- D-3: every adoption and checkpoint publication re-checks the arm and the incarnation ---

impl World {
    /// The incarnation of the open vault right now (what a run's snapshot records).
    fn incarnation(&self) -> u64 {
        lock(&self.session.0.inner).incarnation
    }

    fn adopted(&self) -> Vec<(u64, u64)> {
        self.sync.probe().adopted.lock().unwrap().clone()
    }

    fn published(&self) -> Vec<(u64, u64)> {
        self.sync.probe().published.lock().unwrap().clone()
    }

    /// Arms a merge: the copy gains "B", the next watcher event runs a sync that merges it.
    fn foreign_change(&self) -> Vec<u8> {
        let checkpoint = fs::read(self.checkpoint_file()).unwrap();
        self.foreign_add("B");
        checkpoint
    }

    fn run_watch(&self) {
        self.watch.fire();
        self.advance_ms(0);
    }
}

/// Re-review reproduction 1: only the folder changes (no vault switch, the incarnation stays)
/// right before the adoption. The disarmed run's merge is never adopted.
#[test]
fn a_folder_change_before_the_adoption_discards_the_old_result() {
    let w = world();
    w.open();
    let checkpoint = w.foreign_change();
    let (settings, sync) = (w.settings.clone(), w.sync.clone());
    let mut once = true;
    w.set_hook(&w.sync.probe().before_adopt, move || {
        if std::mem::take(&mut once) {
            settings.set_secondary_folder(None).unwrap();
            sync.rearm();
        }
    });
    let incarnation = w.incarnation();
    let published = w.published().len();
    w.run_watch();
    assert_eq!(w.incarnation(), incarnation, "no vault change: folder only");
    assert!(w.live_titles().is_empty(), "stale arm adopted its merge");
    assert!(w.adopted().is_empty());
    w.advance(5);
    assert!(w.primary_titles().is_empty());
    assert_eq!(fs::read(w.checkpoint_file()).unwrap(), checkpoint);
    assert_eq!(w.published().len(), published);
}

/// Re-review reproduction 2: a lock between durability and the commit. No checkpoint is
/// published for the locked vault.
#[test]
fn a_lock_before_the_commit_publishes_no_checkpoint() {
    let w = world();
    w.open();
    let checkpoint = w.foreign_change();
    let published = w.published().len();
    let session = w.session.clone();
    let mut once = true;
    w.set_hook(&w.sync.probe().before_commit, move || {
        if std::mem::take(&mut once) {
            session.lock(LockReason::Manual);
        }
    });
    w.run_watch();
    assert!(!w.session.is_unlocked());
    assert_eq!(
        fs::read(w.checkpoint_file()).unwrap(),
        checkpoint,
        "stale incarnation committed"
    );
    assert_eq!(w.published().len(), published);
}

/// A folder change alone (the incarnation stays) between durability and the commit.
#[test]
fn a_folder_change_before_the_commit_publishes_no_checkpoint() {
    let w = world();
    w.open();
    let checkpoint = w.foreign_change();
    let published = w.published().len();
    let (settings, sync) = (w.settings.clone(), w.sync.clone());
    let mut once = true;
    w.set_hook(&w.sync.probe().before_commit, move || {
        if std::mem::take(&mut once) {
            settings.set_secondary_folder(None).unwrap();
            sync.rearm();
        }
    });
    w.run_watch();
    assert!(w.session.is_unlocked());
    assert_eq!(fs::read(w.checkpoint_file()).unwrap(), checkpoint);
    assert_eq!(w.published().len(), published);
}

/// Quitting between durability and the commit publishes nothing.
#[test]
fn quitting_before_the_commit_publishes_no_checkpoint() {
    let w = world();
    w.open();
    let checkpoint = w.foreign_change();
    let published = w.published().len();
    let session = w.session.clone();
    let mut once = true;
    w.set_hook(&w.sync.probe().before_commit, move || {
        if std::mem::take(&mut once) {
            session.begin_quit();
        }
    });
    w.run_watch();
    assert!(w.session.is_quitting());
    assert_eq!(fs::read(w.checkpoint_file()).unwrap(), checkpoint);
    assert_eq!(w.published().len(), published);
}

/// A lock and an unlock of the same vault before the adoption: the old run's merge is never
/// adopted into the new unlock; the re-arm that came with the unlock merges again.
#[test]
fn a_lock_and_unlock_before_the_adoption_never_adopts_the_old_result() {
    let w = world();
    w.open();
    w.foreign_change();
    let session = w.session.clone();
    let stale = Arc::new(Mutex::new(None));
    let seen = stale.clone();
    let mut once = true;
    w.set_hook(&w.sync.probe().before_adopt, move || {
        if std::mem::take(&mut once) {
            *seen.lock().unwrap() = Some(lock(&session.0.inner).incarnation);
            session.lock(LockReason::Manual);
            session.unlock(pw()).unwrap();
        }
    });
    let earlier = w.published().len();
    w.run_watch();
    w.advance(5);
    let stale = stale.lock().unwrap().expect("the hook ran");
    assert!(w.session.is_unlocked());
    assert!(
        w.adopted().iter().all(|(i, _)| *i != stale),
        "the old run adopted into the new unlock: {:?}",
        w.adopted()
    );
    let later = w.published().split_off(earlier);
    assert!(
        later.iter().all(|(i, _)| *i != stale),
        "the old run published: {later:?}"
    );
    assert_eq!(w.live_titles(), v(&["B"]), "the re-armed run merged it");
}

/// A lock and an unlock between durability and the commit: only the run of the new unlock
/// publishes.
#[test]
fn a_lock_and_unlock_before_the_commit_publishes_only_for_the_new_unlock() {
    let w = world();
    w.open();
    w.foreign_change();
    let session = w.session.clone();
    let stale = Arc::new(Mutex::new(None));
    let seen = stale.clone();
    let mut once = true;
    w.set_hook(&w.sync.probe().before_commit, move || {
        if std::mem::take(&mut once) {
            *seen.lock().unwrap() = Some(lock(&session.0.inner).incarnation);
            session.lock(LockReason::Manual);
            session.unlock(pw()).unwrap();
        }
    });
    let earlier = w.published().len();
    w.run_watch();
    w.advance(5);
    let stale = stale.lock().unwrap().expect("the hook ran");
    let later = w.published().split_off(earlier);
    assert!(
        later.iter().all(|(i, _)| *i != stale),
        "the old run published: {later:?}"
    );
    assert!(
        later.iter().any(|(i, _)| *i == w.incarnation()),
        "the re-armed run published: {later:?}"
    );
    assert!(w.idle_synced());
}

/// The publication is atomic with the invalidations: a lock that arrives while the checkpoint
/// is being written waits for it (and so never sees a publication land after it).
#[test]
fn a_lock_waits_for_a_publication_in_progress() {
    let w = world();
    w.open();
    w.foreign_change();
    let before = w.incarnation();
    let session = w.session.clone();
    let early = Arc::new(AtomicBool::new(false));
    let pending = Arc::new(Mutex::new(None));
    let (flag, slot) = (early.clone(), pending.clone());
    let mut once = true;
    w.set_hook(&w.sync.probe().during_publish, move || {
        if !std::mem::take(&mut once) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let s = session.clone();
        std::thread::spawn(move || {
            s.lock(LockReason::Manual);
            let _ = tx.send(());
        });
        // The window only gives a broken fence the chance to fail; with the fence the lock
        // can neither drop the database nor return before the publication ends, whatever the
        // timing. The database is checked too: the lock observer (the arm shutdown) also waits
        // for the publication, so the return alone would not show a lock that went through.
        let returned = rx.recv_timeout(Duration::from_millis(200)).is_ok();
        flag.store(returned || !session.is_unlocked(), Ordering::SeqCst);
        *slot.lock().unwrap() = Some(rx);
    });
    w.run_watch();
    assert!(
        !early.load(Ordering::SeqCst),
        "the lock completed while a publication was in progress"
    );
    let rx = pending.lock().unwrap().take().expect("the hook ran");
    assert_eq!(rx.recv_timeout(WORKER_BOUND), Ok(()), "the lock completed");
    assert!(!w.session.is_unlocked());
    assert!(
        w.published().iter().any(|(i, _)| *i == before),
        "the publication that began before the lock completed: {:?}",
        w.published()
    );
}

/// C2: a folder on a volume that cannot replace the copy safely shows `unsupportedFolder` and
/// is not retried on the timer (the next folder change or unlock tries again).
#[test]
fn a_volume_without_a_safe_replacement_is_an_unsupported_folder() {
    let status = failure_status(&SyncError::Storage(StorageError::NoSafeReplace));
    assert_eq!(
        status,
        SecondaryStatus::Unavailable {
            reason: SecondaryUnavailable::UnsupportedFolder
        }
    );
    assert!(!is_transient(&status));
    assert_eq!(
        serde_json::to_value(&status).unwrap(),
        serde_json::json!({"state": "unavailable", "reason": "unsupportedFolder"})
    );
}

/// FR-C2: a checkpoint describes the secondary, not every later incarnation of primary content.
#[test]
fn clean_primary_reload_merges_checkpointed_secondary_only_entries() {
    let w = world();
    w.open();
    let base = w.session.with_db(Clone::clone).unwrap();
    w.foreign_add("secondary-only");
    w.sync.sync_now().unwrap();
    assert_eq!(w.primary_titles(), v(&["secondary-only"]));
    let checkpoint = fs::read(w.checkpoint_file()).unwrap();
    let mut delayed = base;
    add_to(
        &mut delayed,
        "delayed-primary-only",
        Timestamp(1_700_000_600),
    );
    fs::write(&w.primary, delayed.save(&pw()).unwrap()).unwrap();
    w.session.check_external_change().unwrap();
    assert_eq!(w.live_titles(), v(&["delayed-primary-only"]));
    assert_eq!(fs::read(w.checkpoint_file()).unwrap(), checkpoint);
    w.sync.sync_now().unwrap();
    assert_eq!(
        w.remote_titles(),
        v(&["delayed-primary-only", "secondary-only"])
    );
    assert_eq!(w.primary_titles(), w.remote_titles());
}

#[test]
fn restart_with_a_delayed_primary_merges_checkpointed_secondary_only_entries() {
    let w = world();
    w.open();
    let base = w.session.with_db(Clone::clone).unwrap();
    w.foreign_add("secondary-only");
    w.sync.sync_now().unwrap();
    assert_eq!(w.primary_titles(), v(&["secondary-only"]));
    let mut delayed = base;
    add_to(
        &mut delayed,
        "delayed-primary-only",
        Timestamp(1_700_000_600),
    );
    fs::write(&w.primary, delayed.save(&pw()).unwrap()).unwrap();
    let w = w.restart();
    assert_eq!(
        w.remote_titles(),
        v(&["delayed-primary-only", "secondary-only"])
    );
    assert_eq!(w.primary_titles(), w.remote_titles());
}

#[test]
fn primary_reload_before_checkpoint_publication_cannot_authorize_content_loss() {
    let w = world();
    w.open();
    let mut delayed = w.session.with_db(Clone::clone).unwrap();
    add_to(
        &mut delayed,
        "delayed-primary-only",
        Timestamp(1_700_000_600),
    );
    w.foreign_add("secondary-only");
    let session = w.session.clone();
    let primary = w.primary.clone();
    let reached = Arc::new(AtomicBool::new(false));
    let seen = reached.clone();
    w.set_hook(&w.sync.probe().before_commit, move || {
        if !seen.swap(true, Ordering::SeqCst) {
            fs::write(&primary, delayed.save(&pw()).unwrap()).unwrap();
            session.check_external_change().unwrap();
        }
    });
    w.sync.sync_now().unwrap();
    assert!(reached.load(Ordering::SeqCst));
    assert_eq!(w.live_titles(), v(&["delayed-primary-only"]));
    w.sync.sync_now().unwrap();
    assert_eq!(
        w.remote_titles(),
        v(&["delayed-primary-only", "secondary-only"])
    );
    assert_eq!(w.primary_titles(), w.remote_titles());
}
