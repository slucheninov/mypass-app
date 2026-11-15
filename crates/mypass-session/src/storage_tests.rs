//! The session over iCloud Drive storage (T8): deterministic, over the platform's
//! `FakeCoordinatedFiles`, a manual scheduler and a fake download clock. No real iCloud.

use super::*;
use crate::saving::ExternalChange;
use crate::scheduler::{ManualScheduler, Scheduler};
use crate::session::{LockReason, SessionDeps, SessionError, SessionState, VaultSession};
use crate::settings::SettingsStore;
use crate::testing::fast_database;
use crate::vault_files::VaultFileStatus;
use mypass_kdbx::{Database, Entry, PlainFileIo, Timestamp};
use mypass_platform::testing::{CoordinatedOp, FakeCoordinatedFiles};
use mypass_platform::ConflictVersion;
use mypass_sync::icloud::{Clock, ICloudDriveLocation, ICloudFileIo, ICLOUD_CACHE_DIR};
use secrecy::SecretString;
use std::fs;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const T0: i64 = 1_700_000_000;

fn pw() -> SecretString {
    SecretString::from("pw".to_owned())
}

/// A download clock whose `sleep` only advances time: a download timeout costs no real time.
struct InstantClock {
    start: Instant,
    elapsed: Mutex<Duration>,
}

impl Clock for InstantClock {
    fn now(&self) -> Instant {
        self.start + *self.elapsed.lock().unwrap()
    }
    fn sleep(&self, d: Duration) {
        *self.elapsed.lock().unwrap() += d;
    }
}

fn add(db: &mut Database, title: &str, at: i64) {
    let mut e = Entry::new(Timestamp(at));
    e.set_field("Title", title, false);
    let root = db.root_id();
    db.add_entry(root, e, Timestamp(at)).unwrap();
}

fn titles_in(db: &Database) -> Vec<String> {
    let mut t: Vec<String> = db
        .root
        .entries
        .iter()
        .filter_map(|e| e.field("Title").map(str::to_owned))
        .collect();
    t.sort();
    t
}

fn titles_of_bytes(bytes: &[u8], password: &SecretString) -> Vec<String> {
    titles_in(&Database::open(bytes, password).unwrap())
}

/// One Mac: a session whose resolver maps iCloud Drive paths to an `ICloudFileIo` over a fake
/// coordinator (its own cache folder), on a manual scheduler.
struct Mac {
    session: VaultSession,
    fake: Arc<FakeCoordinatedFiles>,
    clock: Arc<ManualScheduler>,
    cache_dir: PathBuf,
}

impl Mac {
    fn new(root: &Path, home: &Path, name: &str) -> Self {
        let fake = Arc::new(FakeCoordinatedFiles::new());
        let clock = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(T0 as u64));
        let session = VaultSession::new(SessionDeps {
            settings: SettingsStore::in_memory(),
            scheduler: clock.clone() as Arc<dyn Scheduler>,
            backups_root: root.join(name).join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        let cache_dir = root.join(name).join(ICLOUD_CACHE_DIR);
        let (coordinated, home, cache) = (fake.clone(), home.to_path_buf(), cache_dir.clone());
        session.set_io_resolver(Some(Arc::new(move |path: &Path| {
            if !ICloudDriveLocation::is_ubiquitous(path, &home) {
                return None;
            }
            let clock = Arc::new(InstantClock {
                start: Instant::now(),
                elapsed: Mutex::new(Duration::ZERO),
            });
            Some(ResolvedStorage::icloud(Arc::new(
                ICloudFileIo::new(coordinated.clone(), path, &cache).with_clock(clock),
            )))
        })));
        Self {
            session,
            fake,
            clock,
            cache_dir,
        }
    }

    fn titles(&self) -> Vec<String> {
        self.session.with_db(titles_in).unwrap()
    }

    fn add(&self, title: &str) {
        let title = title.to_owned();
        self.session
            .mutate(move |db, now| {
                add(db, &title, now.0);
                Ok(())
            })
            .unwrap();
    }
}

/// A home with an iCloud Drive folder holding `v.kdbx` (one entry "first"), paths canonical.
struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    drive: PathBuf,
    vault: PathBuf,
    base: Database,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let home = root.join("home");
        let drive = ICloudDriveLocation::root(&home);
        fs::create_dir_all(&drive).unwrap();
        let vault = drive.join("v.kdbx");
        let mut base = fast_database("Cloud Vault");
        add(&mut base, "first", T0 - 1000);
        fs::write(&vault, base.save(&pw()).unwrap()).unwrap();
        Self {
            _dir: dir,
            root,
            home,
            drive,
            vault,
            base,
        }
    }

    fn mac(&self, name: &str) -> Mac {
        Mac::new(&self.root, &self.home, name)
    }

    /// Another Mac saves the base vault plus `title` straight into the file.
    fn remote_save(&self, title: &str) {
        let bytes = fs::read(&self.vault).unwrap();
        let mut db = Database::open(&bytes, &pw()).unwrap();
        add(&mut db, title, T0 + 5000);
        fs::write(&self.vault, db.save(&pw()).unwrap()).unwrap();
    }

    fn file_titles(&self) -> Vec<String> {
        titles_of_bytes(&fs::read(&self.vault).unwrap(), &pw())
    }

    /// iCloud evicts the file: only the legacy placeholder stays.
    fn evict(&self, mac: &Mac) {
        fs::rename(&self.vault, self.drive.join(".v.kdbx.icloud")).unwrap();
        mac.fake.set_placeholder(&self.vault, None);
    }

    /// iCloud Drive is turned off: the folder goes away (kept aside, never deleted).
    fn take_folder_away(&self) {
        fs::rename(&self.drive, self.root.join("drive-away")).unwrap();
    }

    fn bring_folder_back(&self) {
        fs::rename(self.root.join("drive-away"), &self.drive).unwrap();
    }

    fn version(&self, name: &str, db: &Database, password: &SecretString) -> ConflictVersion {
        let path = self.root.join(name);
        fs::write(&path, db.save(password).unwrap()).unwrap();
        ConflictVersion {
            path,
            modified: Some(SystemTime::now()),
            saving_computer: Some("Other Mac".into()),
        }
    }
}

#[test]
fn icloud_vault_opens_through_the_coordinator_with_its_location() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    assert!(mac.fake.count(CoordinatedOp::Read) >= 1);
    assert_eq!(mac.fake.presenter_count(), 1);
    let status = mac.session.storage_status();
    assert_eq!(status.location, VaultLocation::ICloudDrive);
    assert!(!status.fallback_active);
    assert_eq!(mac.titles(), vec!["first"]);
}

#[test]
fn io_resolver_is_applied_per_vault_path_by_the_session() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    let local = fx.root.join("local.kdbx");
    fs::copy(&fx.vault, &local).unwrap();
    mac.session.open_vault(&local, pw()).unwrap();
    assert_eq!(mac.session.storage_status().location, VaultLocation::Local);
    assert_eq!(mac.fake.count(CoordinatedOp::Read), 0);
    assert_eq!(mac.fake.presenter_count(), 0);
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    assert_eq!(
        mac.session.storage_status().location,
        VaultLocation::ICloudDrive
    );
    assert_eq!(mac.fake.presenter_count(), 1);
    // Switching back to a local vault stops the presenter.
    mac.session.open_vault(&local, pw()).unwrap();
    assert_eq!(mac.fake.presenter_count(), 0);
}

#[test]
fn unlock_on_undownloaded_file_reports_downloading_and_keeps_state() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    mac.session.lock(LockReason::Manual);
    fx.evict(&mac);
    let placeholder = fs::read(fx.drive.join(".v.kdbx.icloud")).unwrap();
    assert_eq!(mac.session.unlock(pw()), Err(SessionError::Downloading));
    let status = mac.session.status();
    assert_eq!(status.state, SessionState::Locked);
    assert_eq!(status.file_status, VaultFileStatus::Downloading);
    assert!(mac.session.storage_status().downloading);
    assert!(!fx.vault.exists(), "no vault created");
    assert_eq!(
        fs::read(fx.drive.join(".v.kdbx.icloud")).unwrap(),
        placeholder
    );
    // A fresh open of the evicted vault says the same.
    let other = fx.mac("other");
    assert_eq!(
        other.session.open_vault(&fx.vault, pw()),
        Err(SessionError::Downloading)
    );
    assert_eq!(other.session.status().state, SessionState::NoVault);
}

#[test]
fn timeout_never_falls_back_to_cache() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    let cache = ICloudFileIo::cache_path_for(&mac.cache_dir, &fx.vault);
    assert!(cache.exists(), "the open filled the offline copy");
    mac.session.lock(LockReason::Manual);
    fx.evict(&mac);
    assert_eq!(mac.session.unlock(pw()), Err(SessionError::Downloading));
    assert!(!mac.session.storage_status().fallback_active);
    assert!(!mac.session.is_unlocked());
}

#[test]
fn remote_change_callback_debounced_to_one_check() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    let checks_before = mac.fake.count(CoordinatedOp::UnresolvedConflicts);
    for _ in 0..5 {
        mac.fake.emit_change(&fx.vault);
        mac.clock.advance(Duration::from_millis(90));
    }
    assert_eq!(
        mac.fake.count(CoordinatedOp::UnresolvedConflicts),
        checks_before,
        "nothing checked inside the debounce window"
    );
    mac.clock.advance(REMOTE_CHANGE_DEBOUNCE);
    assert_eq!(
        mac.fake.count(CoordinatedOp::UnresolvedConflicts),
        checks_before + 1,
        "exactly one check"
    );
    mac.clock.advance(Duration::from_secs(5));
    assert_eq!(
        mac.fake.count(CoordinatedOp::UnresolvedConflicts),
        checks_before + 1
    );
}

#[test]
fn remote_change_is_merged_not_replaced() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    mac.add("A");
    fx.remote_save("B");
    mac.fake.emit_change(&fx.vault);
    mac.clock.advance(REMOTE_CHANGE_DEBOUNCE);
    assert_eq!(mac.titles(), vec!["A", "B", "first"]);
    mac.session.flush().unwrap();
    assert_eq!(fx.file_titles(), vec!["A", "B", "first"]);
}

#[test]
fn conflict_versions_merged_on_unlock() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    mac.session.lock(LockReason::Manual);
    let mut theirs = fx.base.clone();
    add(&mut theirs, "B", T0 + 100);
    let version = fx.version("version-b", &theirs, &pw());
    mac.fake.add_conflict(&fx.vault, version.clone());
    mac.session.unlock(pw()).unwrap();
    assert_eq!(mac.titles(), vec!["B", "first"]);
    assert_eq!(fx.file_titles(), vec!["B", "first"]);
    assert_eq!(mac.fake.resolved(), vec![version]);
}

#[test]
fn conflict_versions_merged_on_open_and_on_check() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    let mut theirs = fx.base.clone();
    add(&mut theirs, "on open", T0 + 100);
    let v1 = fx.version("version-1", &theirs, &pw());
    mac.fake.add_conflict(&fx.vault, v1);
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    assert_eq!(mac.titles(), vec!["first", "on open"]);
    let mut later = fx.base.clone();
    add(&mut later, "on check", T0 + 200);
    let v2 = fx.version("version-2", &later, &pw());
    mac.fake.add_conflict(&fx.vault, v2);
    assert_eq!(
        mac.session.check_external_change().unwrap(),
        ExternalChange::Merged
    );
    assert_eq!(mac.titles(), vec!["first", "on check", "on open"]);
    assert_eq!(fx.file_titles(), vec!["first", "on check", "on open"]);
    assert_eq!(mac.fake.resolved().len(), 2);
}

#[test]
fn conflict_with_other_password_left_unresolved_and_vault_still_opens() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    let mut foreign = fx.base.clone();
    add(&mut foreign, "foreign", T0 + 100);
    let version = fx.version(
        "version-x",
        &foreign,
        &SecretString::from("other".to_owned()),
    );
    mac.fake.add_conflict(&fx.vault, version);
    let before = fs::read(&fx.vault).unwrap();
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    assert_eq!(mac.titles(), vec!["first"]);
    assert!(mac.fake.resolved().is_empty());
    assert_eq!(fs::read(&fx.vault).unwrap(), before, "nothing written");
}

#[test]
fn two_macs_editing_concurrently_results_in_union() {
    let fx = Fx::new();
    let mac1 = fx.mac("mac1");
    let mac2 = fx.mac("mac2");
    mac1.session.open_vault(&fx.vault, pw()).unwrap();
    mac2.session.open_vault(&fx.vault, pw()).unwrap();
    mac1.add("A");
    mac1.session.flush().unwrap();
    // Mac 2 was loaded before; its save meets the change on disk and merges.
    mac2.add("B");
    mac2.session.flush().unwrap();
    assert_eq!(fx.file_titles(), vec!["A", "B", "first"]);
    assert_eq!(mac2.titles(), vec!["A", "B", "first"]);
    mac1.session.check_external_change().unwrap();
    assert_eq!(mac1.titles(), vec!["A", "B", "first"]);
}

#[test]
fn fallback_to_cache_when_folder_missing() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    mac.session
        .add_storage_observer(Arc::new(move |s: &StorageStatus| {
            sink.lock().unwrap().push(*s);
        }));
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    fx.take_folder_away();
    mac.add("offline");
    mac.session.flush().unwrap();
    let status = mac.session.storage_status();
    assert!(status.fallback_active);
    assert!(status.cache_ahead);
    let session = mac.session.status();
    assert_eq!(session.save, crate::session::SaveStatus::idle(), "no error");
    assert!(matches!(
        session.file_status,
        VaultFileStatus::Connected { .. }
    ));
    let cache = ICloudFileIo::cache_path_for(&mac.cache_dir, &fx.vault);
    assert_eq!(
        titles_of_bytes(&fs::read(&cache).unwrap(), &pw()),
        vec!["first", "offline"]
    );
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.fallback_active && s.cache_ahead));
}

#[test]
fn fallback_recovers_by_merging_not_overwriting() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    fx.take_folder_away();
    mac.add("offline");
    mac.session.flush().unwrap();
    fx.bring_folder_back();
    // Another Mac wrote meanwhile.
    fx.remote_save("B");
    mac.session.check_external_change().unwrap();
    assert_eq!(fx.file_titles(), vec!["B", "first", "offline"]);
    assert_eq!(mac.titles(), vec!["B", "first", "offline"]);
    let status = mac.session.storage_status();
    assert!(!status.cache_ahead);
    assert!(!status.fallback_active);
    let cache = ICloudFileIo::cache_path_for(&mac.cache_dir, &fx.vault);
    assert_eq!(
        titles_of_bytes(&fs::read(&cache).unwrap(), &pw()),
        vec!["B", "first", "offline"]
    );
    // Later saves go to iCloud again.
    mac.add("online");
    mac.session.flush().unwrap();
    assert_eq!(fx.file_titles(), vec!["B", "first", "offline", "online"]);
}

#[test]
fn offline_edits_survive_a_lock_and_merge_on_unlock() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    mac.session.open_vault(&fx.vault, pw()).unwrap();
    fx.take_folder_away();
    mac.add("offline");
    mac.session.lock(LockReason::Manual);
    fx.bring_folder_back();
    fx.remote_save("B");
    mac.session.unlock(pw()).unwrap();
    assert_eq!(mac.titles(), vec!["B", "first", "offline"]);
    assert_eq!(fx.file_titles(), vec!["B", "first", "offline"]);
}

#[test]
fn session_conflict_resolver_runs_before_a_plain_open() {
    let fx = Fx::new();
    let mac = fx.mac("mac");
    let local = fx.root.join("local.kdbx");
    fs::copy(&fx.vault, &local).unwrap();
    let calls = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let seen = calls.clone();
    mac.session.set_conflict_resolver(Some(Arc::new(
        move |path: &Path, _merge: ConflictMerge<'_>| {
            seen.lock().unwrap().push(path.to_path_buf());
            Ok(false)
        },
    )));
    mac.session.open_vault(&local, pw()).unwrap();
    assert_eq!(calls.lock().unwrap().as_slice(), std::slice::from_ref(&local));
    mac.session.set_conflict_resolver(None);
    mac.session.lock(LockReason::Manual);
    mac.session.unlock(pw()).unwrap();
    assert_eq!(calls.lock().unwrap().len(), 1);
}
