use super::*;
use crate::app_core::ErrorCode;
use crate::scheduler::ManualScheduler;
use crate::session::{LockReason, SessionDeps};
use crate::testing::fast_database;
use mypass_kdbx::{Entry, PlainFileIo, Timestamp};
use secrecy::SecretString;
use std::sync::atomic::AtomicBool;
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
        if !dir.is_dir() {
            return Err(StorageError::Unavailable("missing".into()));
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
            .join(SECONDARY_CHECKPOINT_FILE)
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
    assert_eq!(w.primary_titles(), Vec::<String>::new());
    w.advance(1);
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
