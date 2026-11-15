use super::*;
use crate::icloud::merge_vault_bytes;
use crate::icloud::tests::{add, fast_db, pw, titles_of, vault_pair, FakeClock};
use mypass_platform::testing::{CoordinatedOp, FakeCoordinatedFiles};
use secrecy::SecretString;
use std::sync::atomic::AtomicUsize;

/// A temp home with `Drive/v.kdbx` (the "iCloud" folder) and a cache folder, paths canonical.
struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    drive: PathBuf,
    vault: PathBuf,
    cache_dir: PathBuf,
    fake: Arc<FakeCoordinatedFiles>,
    clock: Arc<FakeClock>,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let drive = root.join("Drive");
        fs::create_dir_all(&drive).unwrap();
        Self {
            _dir: dir,
            vault: drive.join("v.kdbx"),
            cache_dir: root.join("data").join(ICLOUD_CACHE_DIR),
            drive,
            root,
            fake: Arc::new(FakeCoordinatedFiles::new()),
            clock: Arc::new(FakeClock::new()),
        }
    }

    fn io(&self) -> ICloudFileIo {
        ICloudFileIo::new(self.fake.clone(), &self.vault, &self.cache_dir)
            .with_clock(self.clock.clone())
    }

    /// iCloud Drive turned off: the folder goes away (kept aside, never deleted).
    fn take_folder_away(&self) {
        fs::rename(&self.drive, self.root.join("Drive-away")).unwrap();
    }

    fn bring_folder_back(&self) {
        fs::rename(self.root.join("Drive-away"), &self.drive).unwrap();
    }
}

fn kind(e: &KdbxError) -> Option<io::ErrorKind> {
    match e {
        KdbxError::Io { kind, .. } => Some(*kind),
        _ => None,
    }
}

#[test]
fn read_and_write_go_through_the_coordinator_and_refresh_the_cache() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"one").unwrap();
    assert_eq!(io.read(&s.vault).unwrap(), b"one");
    assert_eq!(s.fake.count(CoordinatedOp::Write), 1);
    assert_eq!(s.fake.count(CoordinatedOp::Read), 1);
    assert_eq!(
        s.fake.last_timeout(CoordinatedOp::Read),
        Some(COORDINATION_TIMEOUT)
    );
    assert_eq!(fs::read(io.cache_path()).unwrap(), b"one");
    assert_eq!(io.health(), IoHealth::default());
}

#[test]
fn cache_path_is_keyed_by_the_resolved_vault_path() {
    let s = Setup::new();
    let a = ICloudFileIo::cache_path_for(&s.cache_dir, &s.vault);
    let b = ICloudFileIo::cache_path_for(&s.cache_dir, &s.drive.join(".").join("v.kdbx"));
    let other = ICloudFileIo::cache_path_for(&s.cache_dir, &s.drive.join("w.kdbx"));
    assert_eq!(a, b);
    assert_ne!(a, other);
    assert_eq!(a.parent(), Some(s.cache_dir.as_path()));
    assert_eq!(a.extension().and_then(|e| e.to_str()), Some("kdbx"));
}

#[test]
fn placeholder_read_times_out_as_timed_out_never_not_found_and_never_the_cache() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"cached").unwrap();
    // The file is evicted: only the legacy placeholder is left.
    fs::rename(&s.vault, s.drive.join(".v.kdbx.icloud")).unwrap();
    s.fake.set_placeholder(&s.vault, None);
    let err = io.read(&s.vault).unwrap_err();
    assert_eq!(kind(&err), Some(io::ErrorKind::TimedOut));
    assert!(io.health().downloading);
    assert!(!io.health().fallback_active);
    assert_eq!(s.clock.elapsed(), DOWNLOAD_TIMEOUT);
    assert_eq!(
        s.fake.count(CoordinatedOp::Read),
        0,
        "never read a placeholder"
    );
    // metadata waits the same way instead of reporting a missing file.
    let err = io.metadata(&s.vault).unwrap_err();
    assert_eq!(kind(&err), Some(io::ErrorKind::TimedOut));
}

#[test]
fn placeholder_downloads_then_reads() {
    let s = Setup::new();
    fs::write(&s.vault, b"remote").unwrap();
    s.fake.set_placeholder(&s.vault, Some(1));
    let io = s.io();
    assert_eq!(io.read(&s.vault).unwrap(), b"remote");
    assert_eq!(s.fake.count(CoordinatedOp::RequestDownload), 1);
    assert!(!io.health().downloading);
}

#[test]
fn coordination_timeout_is_timed_out_and_not_a_fallback() {
    let s = Setup::new();
    fs::write(&s.vault, b"v").unwrap();
    let io = s
        .io()
        .with_timeouts(DOWNLOAD_TIMEOUT, Duration::from_millis(20));
    let hold = s.fake.hold(&s.vault);
    let err = io.read(&s.vault).unwrap_err();
    assert_eq!(kind(&err), Some(io::ErrorKind::TimedOut));
    let err = io.write_atomic(&s.vault, b"new").unwrap_err();
    assert_eq!(kind(&err), Some(io::ErrorKind::TimedOut));
    drop(hold);
    assert_eq!(fs::read(&s.vault).unwrap(), b"v");
    assert!(!io.health().fallback_active);
    assert!(!io.health().cache_ahead);
}

#[test]
fn fallback_reads_the_cache_when_the_folder_is_missing() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"synced").unwrap();
    s.take_folder_away();
    assert_eq!(io.read(&s.vault).unwrap(), b"synced");
    assert_eq!(io.metadata(&s.vault).unwrap().len, 6);
    let health = io.health();
    assert!(health.fallback_active);
    assert!(!health.cache_ahead);
    // Back again, nothing ahead: reads go to iCloud again.
    s.bring_folder_back();
    fs::write(&s.vault, b"changed on another mac").unwrap();
    assert_eq!(io.read(&s.vault).unwrap(), b"changed on another mac");
    assert!(!io.health().fallback_active);
}

#[test]
fn missing_folder_without_a_cache_is_not_found() {
    let s = Setup::new();
    let io = s.io();
    s.take_folder_away();
    let err = io.read(&s.vault).unwrap_err();
    assert_eq!(kind(&err), Some(io::ErrorKind::NotFound));
    s.fake.set_icloud_available(true);
    let err = io.metadata(&s.vault).unwrap_err();
    assert_eq!(kind(&err), Some(io::ErrorKind::NotFound));
}

#[test]
fn offline_write_marks_the_cache_ahead_and_reads_keep_the_cache_after_the_folder_returns() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"synced").unwrap();
    s.take_folder_away();
    io.write_atomic(&s.vault, b"offline edit").unwrap();
    assert!(io.health().cache_ahead);
    let state = format!("{}{CACHE_STATE_SUFFIX}", io.cache_path().display());
    assert!(Path::new(&state).exists());
    s.bring_folder_back();
    // Still ahead: the edit is not in iCloud yet, so the cache keeps serving.
    assert_eq!(io.read(&s.vault).unwrap(), b"offline edit");
    assert_eq!(fs::read(&s.vault).unwrap(), b"synced", "iCloud untouched");
    io.write_atomic(&s.vault, b"second offline edit").unwrap();
    assert_eq!(fs::read(&s.vault).unwrap(), b"synced");
    assert_eq!(fs::read(io.cache_path()).unwrap(), b"second offline edit");
    assert!(io.health().fallback_active);
}

#[test]
fn replace_if_refuses_while_offline_and_goes_through_the_coordinator_otherwise() {
    let s = Setup::new();
    let io = s.io();
    assert!(matches!(
        io.replace_if(&s.vault, None, b"new", MAX_VAULT_FILE_BYTES)
            .unwrap(),
        ReplaceOutcome::Replaced
    ));
    assert_eq!(s.fake.count(CoordinatedOp::Write), 1);
    assert!(matches!(
        io.replace_if(&s.vault, None, b"newer", MAX_VAULT_FILE_BYTES)
            .unwrap(),
        ReplaceOutcome::Conflict
    ));
    s.take_folder_away();
    assert!(io
        .replace_if(&s.vault, None, b"x", MAX_VAULT_FILE_BYTES)
        .is_err());
}

#[test]
fn resolve_merges_the_ahead_cache_into_the_real_file_without_removing_anything() {
    let s = Setup::new();
    let password = pw();
    let mut base_db = fast_db("v");
    add(&mut base_db, "base", 1_700_000_000);
    let mut offline_db = base_db.clone();
    add(&mut offline_db, "offline", 1_700_001_000);
    let mut remote_db = base_db.clone();
    add(&mut remote_db, "remote", 1_700_002_000);
    let base = base_db.save(&password).unwrap();
    let offline = offline_db.save(&password).unwrap();
    let remote = remote_db.save(&password).unwrap();
    let io = s.io();
    io.write_atomic(&s.vault, &base).unwrap();
    s.take_folder_away();
    io.write_atomic(&s.vault, &offline).unwrap();
    s.bring_folder_back();
    // Another Mac saved meanwhile.
    fs::write(&s.vault, &remote).unwrap();
    let mut merge = |a: &[u8], b: &[u8]| merge_vault_bytes(a, b, &password);
    let outcome = io.resolve_conflicts(&mut merge).unwrap();
    assert_eq!(outcome.merged, 1);
    assert_eq!(
        titles_of(&fs::read(&s.vault).unwrap(), &password),
        vec!["base", "offline", "remote"]
    );
    assert!(!io.health().cache_ahead);
    assert!(!io.health().fallback_active);
    assert!(io.cache_path().exists(), "the cache is never removed");
    assert_eq!(
        fs::read(io.cache_path()).unwrap(),
        fs::read(&s.vault).unwrap()
    );
    // Reads come from iCloud again.
    assert_eq!(io.read(&s.vault).unwrap(), fs::read(&s.vault).unwrap());
    // Nothing left to merge.
    assert!(!io.resolve_conflicts(&mut merge).unwrap().changed());
}

#[test]
fn resolve_is_a_no_op_while_the_folder_is_unavailable() {
    let s = Setup::new();
    let password = pw();
    let (base, offline) = vault_pair(&["base"], &["offline"], &password);
    let io = s.io();
    io.write_atomic(&s.vault, &base).unwrap();
    s.take_folder_away();
    io.write_atomic(&s.vault, &offline).unwrap();
    let mut merge = |a: &[u8], b: &[u8]| merge_vault_bytes(a, b, &password);
    assert_eq!(
        io.resolve_conflicts(&mut merge).unwrap(),
        ConflictOutcome::default()
    );
    assert!(io.health().cache_ahead);
    assert_eq!(s.fake.count(CoordinatedOp::Write), 1);
}

#[test]
fn resolve_with_another_password_keeps_the_cache_ahead() {
    let s = Setup::new();
    let password = pw();
    let (base, offline) = vault_pair(&["base"], &["offline"], &password);
    let io = s.io();
    io.write_atomic(&s.vault, &base).unwrap();
    s.take_folder_away();
    io.write_atomic(&s.vault, &offline).unwrap();
    s.bring_folder_back();
    let other = SecretString::from("changed elsewhere");
    let mut merge = |a: &[u8], b: &[u8]| merge_vault_bytes(a, b, &other);
    let outcome = io.resolve_conflicts(&mut merge).unwrap();
    assert_eq!(outcome.merged, 0);
    assert_eq!(outcome.unresolved, 1);
    assert!(io.health().cache_ahead);
    assert_eq!(fs::read(&s.vault).unwrap(), base, "nothing written");
    assert_eq!(fs::read(io.cache_path()).unwrap(), offline);
}

#[test]
fn resolve_merges_platform_conflict_versions_and_marks_them_resolved() {
    let s = Setup::new();
    let password = pw();
    let (current, theirs) = vault_pair(&["base"], &["theirs"], &password);
    fs::write(&s.vault, &current).unwrap();
    let version_path = s.root.join("version-1");
    fs::write(&version_path, &theirs).unwrap();
    let version = ConflictVersion {
        path: version_path.clone(),
        modified: None,
        saving_computer: Some("Other Mac".into()),
    };
    s.fake.add_conflict(&s.vault, version.clone());
    let io = s.io();
    let mut merge = |a: &[u8], b: &[u8]| merge_vault_bytes(a, b, &password);
    let outcome = io.resolve_conflicts(&mut merge).unwrap();
    assert_eq!(outcome.merged, 1);
    assert_eq!(s.fake.resolved(), vec![version]);
    assert_eq!(
        titles_of(&fs::read(&s.vault).unwrap(), &password),
        vec!["base", "theirs"]
    );
    assert!(version_path.exists(), "the version store owns its files");
    assert_eq!(
        s.fake.count(CoordinatedOp::Write),
        1,
        "one coordinated write"
    );
}

#[test]
fn resolve_with_nothing_to_merge_takes_no_coordinated_write() {
    let s = Setup::new();
    fs::write(&s.vault, b"v").unwrap();
    let io = s.io();
    let mut merge = |_: &[u8], _: &[u8]| -> io::Result<Vec<u8>> { panic!("no merge expected") };
    assert!(!io.resolve_conflicts(&mut merge).unwrap().changed());
    assert_eq!(s.fake.count(CoordinatedOp::Write), 0);
}

#[test]
fn presenter_is_added_fires_and_is_removed() {
    let s = Setup::new();
    fs::write(&s.vault, b"v").unwrap();
    let io = s.io();
    let fired = Arc::new(AtomicUsize::new(0));
    let counter = fired.clone();
    io.set_on_remote_change(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }))
    .unwrap();
    assert_eq!(s.fake.presenter_count(), 1);
    s.fake.emit_change(&s.vault);
    assert_eq!(fired.load(Ordering::SeqCst), 1);
    // Replacing keeps exactly one.
    io.set_on_remote_change(Arc::new(|| {})).unwrap();
    assert_eq!(s.fake.presenter_count(), 1);
    io.clear_on_remote_change();
    assert_eq!(s.fake.presenter_count(), 0);
    io.set_on_remote_change(Arc::new(|| {})).unwrap();
    drop(io);
    assert_eq!(s.fake.presenter_count(), 0, "dropped with the FileIo");
}

#[test]
fn metadata_is_fresh() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"one").unwrap();
    assert_eq!(io.metadata(&s.vault).unwrap().len, 3);
    fs::write(&s.vault, b"written by another mac").unwrap();
    assert_eq!(io.metadata(&s.vault).unwrap().len, 22);
}

#[test]
fn reads_are_bounded_and_writes_leave_no_temp_files() {
    let s = Setup::new();
    let io = s.io();
    for i in 0..3 {
        io.write_atomic(&s.vault, format!("v{i}").as_bytes())
            .unwrap();
    }
    let names: Vec<_> = fs::read_dir(&s.drive)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("v.kdbx")]);
    let err = io.read_limited(&s.vault, 1).unwrap_err();
    assert_eq!(kind(&err), Some(io::ErrorKind::FileTooLarge));
    // A folder posing as the vault is refused (R20).
    let posing = s.drive.join("dir.kdbx");
    fs::create_dir(&posing).unwrap();
    assert!(io.read(&posing).is_err());
}

#[test]
fn conformance_round_trip_over_the_platform_coordinator() {
    // The real coordinator of this OS (NSFileCoordinator on macOS, pass-through elsewhere) on a
    // plain temp folder: no iCloud involved.
    let s = Setup::new();
    let io = ICloudFileIo::new(mypass_platform::coordinated_files(), &s.vault, &s.cache_dir);
    io.write_atomic(&s.vault, b"first").unwrap();
    io.write_atomic(&s.vault, b"second").unwrap();
    assert_eq!(io.read(&s.vault).unwrap(), b"second");
    assert_eq!(io.metadata(&s.vault).unwrap().len, 6);
    let mut merge = |_: &[u8], _: &[u8]| -> io::Result<Vec<u8>> { panic!("no merge expected") };
    assert!(!io.resolve_conflicts(&mut merge).unwrap().changed());
}

/// Real iCloud Drive (owner hardware only): `MYPASS_ICLOUD_INTEGRATION_TESTS=1 cargo test -p
/// mypass-sync real_icloud -- --ignored`. Writes and reads a throwaway (non-vault) file in a
/// test folder in iCloud Drive.
#[test]
#[ignore = "needs a signed-in iCloud Drive; set MYPASS_ICLOUD_INTEGRATION_TESTS=1"]
fn real_icloud_round_trip() {
    if std::env::var("MYPASS_ICLOUD_INTEGRATION_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: MYPASS_ICLOUD_INTEGRATION_TESTS is not 1");
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("skipped: HOME is not set");
        return;
    };
    let coordinated = mypass_platform::coordinated_files();
    if !coordinated.icloud_available() {
        eprintln!("skipped: no iCloud account is signed in");
        return;
    }
    let folder = crate::icloud::ICloudDriveLocation::root(&home)
        .join(format!("MyPass integration test {}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = folder.join("probe.bin");
    let io = ICloudFileIo::new(coordinated, &path, cache.path());
    io.write_atomic(&path, b"probe").unwrap();
    assert_eq!(io.read(&path).unwrap(), b"probe");
    assert!(!io.health().fallback_active);
}

#[test]
fn cache_marker_is_bounded_and_regular_only() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"cached").unwrap();
    fs::write(io.state_path(), vec![b'x'; 1024]).unwrap();
    assert!(io.health().cache_ahead);
    fs::remove_file(io.state_path()).unwrap();
    fs::create_dir(io.state_path()).unwrap();
    assert!(io.health().cache_ahead);
    fs::remove_dir(io.state_path()).unwrap();
    #[cfg(unix)]
    {
        assert!(std::process::Command::new("mkfifo")
            .arg(io.state_path())
            .status()
            .unwrap()
            .success());
        assert!(io.health().cache_ahead);
    }
}

#[test]
fn recovery_never_acknowledges_a_newer_cache_written_by_another_handle() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"base").unwrap();
    s.take_folder_away();
    io.write_atomic(&s.vault, b"offline").unwrap();
    s.bring_folder_back();
    let other = s.io();
    let mut once = false;
    let result = io.resolve_conflicts(&mut |_, _| {
        if !once {
            once = true;
            other.write_atomic(&s.vault, b"newest edit").unwrap();
        }
        Ok(b"older merge".to_vec())
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(io.health().cache_ahead);
    assert_eq!(fs::read(io.cache_path()).unwrap(), b"newest edit");
    io.resolve_conflicts(&mut |_, version| Ok(version.to_vec()))
        .unwrap();
    assert_eq!(fs::read(&s.vault).unwrap(), b"newest edit");
    assert!(!io.health().cache_ahead);
}

#[test]
fn creation_rechecks_cache_and_placeholder_at_write_acquisition() {
    for placeholder in [false, true] {
        let s = Setup::new();
        let io = s.io();
        fs::create_dir_all(&s.cache_dir).unwrap();
        let existing = if placeholder {
            s.drive.join(".v.kdbx.icloud")
        } else {
            io.cache_path().to_path_buf()
        };
        let path = existing.clone();
        s.fake.before_next_write(move || {
            fs::write(path, b"prior vault").unwrap();
        });
        assert_eq!(
            kind(&io.create_new(b"new vault").unwrap_err()),
            Some(io::ErrorKind::AlreadyExists)
        );
        assert_eq!(fs::read(existing).unwrap(), b"prior vault");
        assert!(!s.vault.exists());
    }
}

#[test]
fn recovery_refuses_a_writer_committed_during_merge() {
    let s = Setup::new();
    let io = s.io();
    io.write_atomic(&s.vault, b"base").unwrap();
    s.take_folder_away();
    io.write_atomic(&s.vault, b"offline").unwrap();
    s.bring_folder_back();
    let result = io.resolve_conflicts(&mut |_, _| {
        PlainFileIo
            .write_atomic(&s.vault, b"competing writer")
            .unwrap();
        Ok(b"stale merge".to_vec())
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert_eq!(fs::read(&s.vault).unwrap(), b"competing writer");
    assert_eq!(fs::read(io.cache_path()).unwrap(), b"offline");
    assert!(io.health().cache_ahead);
}
