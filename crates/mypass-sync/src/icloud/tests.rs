use super::*;
use mypass_kdbx::{Argon2Variant, Entry, KdfParams, Timestamp};
use std::cell::RefCell;
use std::fs;
use std::sync::Mutex;

// ---- shared helpers (also used by the ICloudFileIo tests) ----

pub(crate) fn pw() -> SecretString {
    SecretString::from("correct horse battery")
}

/// Argon2id 8 MiB, one pass: real KDBX, fast enough for tests.
pub(crate) fn fast_db(name: &str) -> Database {
    let mut db = Database::new(name);
    if let KdfParams::Argon2 {
        variant,
        memory_bytes,
        iterations,
        parallelism,
        ..
    } = &mut db.settings.kdf
    {
        *variant = Argon2Variant::Id;
        *memory_bytes = 8 * 1024 * 1024;
        *iterations = 1;
        *parallelism = 1;
    }
    db
}

pub(crate) fn add(db: &mut Database, title: &str, at: i64) {
    let mut e = Entry::new(Timestamp(at));
    e.set_field("Title", title, false);
    let root = db.root.uuid;
    db.add_entry(root, e, Timestamp(at)).unwrap();
}

pub(crate) fn titles_of(bytes: &[u8], password: &SecretString) -> Vec<String> {
    let db = Database::open(bytes, password).unwrap();
    let mut t: Vec<String> = db
        .root
        .entries
        .iter()
        .filter_map(|e| e.field("Title").map(str::to_owned))
        .collect();
    t.sort();
    t
}

/// A vault with the given titles plus a copy of it with other titles (same root, so the two
/// merge), both encoded with `password`.
pub(crate) fn vault_pair(
    base: &[&str],
    other: &[&str],
    password: &SecretString,
) -> (Vec<u8>, Vec<u8>) {
    let mut a = fast_db("v");
    for (i, t) in base.iter().enumerate() {
        add(&mut a, t, 1_700_000_000 + i as i64);
    }
    let mut b = a.clone();
    for (i, t) in other.iter().enumerate() {
        add(&mut b, t, 1_700_001_000 + i as i64);
    }
    (a.save(password).unwrap(), b.save(password).unwrap())
}

// ---- location ----

#[test]
fn icloud_root_is_under_home_library_mobile_documents() {
    let home = Path::new("/Users/someone");
    assert_eq!(
        ICloudDriveLocation::root(home),
        PathBuf::from("/Users/someone/Library/Mobile Documents/com~apple~CloudDocs")
    );
}

#[test]
fn is_inside_true_for_root_and_children_false_for_prefix_sibling() {
    let home = tempfile::tempdir().unwrap();
    let root = ICloudDriveLocation::root(home.path());
    assert!(ICloudDriveLocation::is_inside(&root, home.path()));
    assert!(ICloudDriveLocation::is_inside(
        &root.join("Vaults").join("v.kdbx"),
        home.path()
    ));
    let sibling = root.with_file_name("com~apple~CloudDocs2").join("v.kdbx");
    assert!(!ICloudDriveLocation::is_inside(&sibling, home.path()));
    assert!(!ICloudDriveLocation::is_inside(
        &home.path().join("Documents").join("v.kdbx"),
        home.path()
    ));
    // `..` cannot smuggle a path in or out.
    let escaping = root.join("..").join("..").join("v.kdbx");
    assert!(!ICloudDriveLocation::is_inside(&escaping, home.path()));
}

#[cfg(unix)]
#[test]
fn is_inside_resolves_symlink_into_icloud() {
    let home = tempfile::tempdir().unwrap();
    let root = ICloudDriveLocation::root(home.path());
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("v.kdbx"), b"x").unwrap();
    let docs = home.path().join("Documents");
    fs::create_dir_all(&docs).unwrap();
    std::os::unix::fs::symlink(root.join("v.kdbx"), docs.join("link.kdbx")).unwrap();
    std::os::unix::fs::symlink(&root, docs.join("cloud")).unwrap();
    assert!(ICloudDriveLocation::is_inside(
        &docs.join("link.kdbx"),
        home.path()
    ));
    // A file about to be created inside a linked folder resolves through its parent.
    assert!(ICloudDriveLocation::is_inside(
        &docs.join("cloud").join("new.kdbx"),
        home.path()
    ));
}

#[test]
fn vault_location_of_table() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let md = ICloudDriveLocation::mobile_documents(h);
    let cases = [
        (
            ICloudDriveLocation::root(h).join("v.kdbx"),
            VaultLocation::ICloudDrive,
        ),
        (
            ICloudDriveLocation::root(h)
                .join("a")
                .join("b")
                .join("v.kdbx"),
            VaultLocation::ICloudDrive,
        ),
        // An app's folder in iCloud Drive syncs the same way.
        (
            md.join("iCloud~com~example~app").join("v.kdbx"),
            VaultLocation::ICloudDrive,
        ),
        (
            md.join("com~apple~CloudDocs2").join("v.kdbx"),
            VaultLocation::ICloudDrive,
        ),
        (h.join("Documents").join("v.kdbx"), VaultLocation::Local),
        (
            h.join("Library")
                .join("Mobile Documents backup")
                .join("v.kdbx"),
            VaultLocation::Local,
        ),
        (PathBuf::from("/Volumes/USB/v.kdbx"), VaultLocation::Local),
    ];
    for (path, expected) in cases {
        assert_eq!(vault_location_of(&path, h), expected, "{}", path.display());
    }
    assert_eq!(
        serde_json::to_value(VaultLocation::ICloudDrive).unwrap(),
        serde_json::json!("iCloudDrive")
    );
}

#[test]
fn placeholder_name_is_dot_name_dot_icloud() {
    assert_eq!(placeholder_name("vault.kdbx"), ".vault.kdbx.icloud");
    assert_eq!(
        placeholder_path(Path::new("/a/b/vault.kdbx")),
        Some(PathBuf::from("/a/b/.vault.kdbx.icloud"))
    );
}

// ---- download waiting ----

/// A fake clock: `sleep` advances time and records the slept durations.
#[derive(Default)]
struct FakeClock {
    start: Option<Instant>,
    elapsed: Mutex<Duration>,
    sleeps: Mutex<Vec<Duration>>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            start: Some(Instant::now()),
            ..Self::default()
        }
    }
    fn elapsed(&self) -> Duration {
        *self.elapsed.lock().unwrap()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.start.unwrap() + self.elapsed()
    }
    fn sleep(&self, d: Duration) {
        *self.elapsed.lock().unwrap() += d;
        self.sleeps.lock().unwrap().push(d);
    }
}

/// A probe that reports `initial` until `polls_until_current` polls after the request, then
/// `Current` (`None`: never).
struct ScriptedProbe {
    initial: DownloadState,
    polls_until_current: Option<usize>,
    polls_after_request: RefCell<usize>,
    requests: RefCell<usize>,
    states: RefCell<usize>,
}

impl ScriptedProbe {
    fn new(initial: DownloadState, polls_until_current: Option<usize>) -> Self {
        Self {
            initial,
            polls_until_current,
            polls_after_request: RefCell::new(0),
            requests: RefCell::new(0),
            states: RefCell::new(0),
        }
    }
}

impl DownloadProbe for ScriptedProbe {
    fn state(&self, _path: &Path) -> io::Result<DownloadState> {
        *self.states.borrow_mut() += 1;
        if *self.requests.borrow() == 0 {
            return Ok(self.initial);
        }
        let mut polls = self.polls_after_request.borrow_mut();
        *polls += 1;
        Ok(match self.polls_until_current {
            Some(n) if *polls >= n => DownloadState::Current,
            _ => self.initial,
        })
    }
    fn request_download(&self, _path: &Path) -> io::Result<()> {
        *self.requests.borrow_mut() += 1;
        Ok(())
    }
}

fn existing_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kdbx");
    fs::write(&path, b"vault").unwrap();
    (dir, path)
}

#[test]
fn ensure_downloaded_returns_immediately_when_current() {
    let (_d, path) = existing_file();
    let probe = ScriptedProbe::new(DownloadState::Current, None);
    let clock = FakeClock::new();
    ensure_downloaded(&probe, &clock, &path, DOWNLOAD_TIMEOUT).unwrap();
    assert_eq!(*probe.requests.borrow(), 0);
    assert_eq!(clock.elapsed(), Duration::ZERO);
}

#[test]
fn ensure_downloaded_skips_non_ubiquitous_files() {
    let (_d, path) = existing_file();
    let probe = ScriptedProbe::new(DownloadState::NotUbiquitous, None);
    let clock = FakeClock::new();
    ensure_downloaded(&probe, &clock, &path, DOWNLOAD_TIMEOUT).unwrap();
    assert_eq!(*probe.requests.borrow(), 0);
    assert_eq!(*probe.states.borrow(), 1);
    assert_eq!(clock.elapsed(), Duration::ZERO);
}

#[test]
fn ensure_downloaded_reads_an_older_local_version_without_waiting() {
    let (_d, path) = existing_file();
    let probe = ScriptedProbe::new(DownloadState::Downloaded, None);
    let clock = FakeClock::new();
    ensure_downloaded(&probe, &clock, &path, DOWNLOAD_TIMEOUT).unwrap();
    assert_eq!(
        *probe.requests.borrow(),
        1,
        "the newer version is requested"
    );
    assert_eq!(clock.elapsed(), Duration::ZERO);
}

#[test]
fn ensure_downloaded_polls_every_200ms_until_current() {
    let (_d, path) = existing_file();
    let probe = ScriptedProbe::new(DownloadState::NotDownloaded, Some(3));
    let clock = FakeClock::new();
    ensure_downloaded(&probe, &clock, &path, DOWNLOAD_TIMEOUT).unwrap();
    assert_eq!(*probe.requests.borrow(), 1);
    assert_eq!(clock.elapsed(), Duration::from_millis(600));
    assert!(clock
        .sleeps
        .lock()
        .unwrap()
        .iter()
        .all(|d| *d == DOWNLOAD_POLL));
}

#[test]
fn ensure_downloaded_times_out_after_timeout_with_fake_probe() {
    let (_d, path) = existing_file();
    let probe = ScriptedProbe::new(DownloadState::NotDownloaded, None);
    let clock = FakeClock::new();
    assert_eq!(
        ensure_downloaded(&probe, &clock, &path, DOWNLOAD_TIMEOUT),
        Err(ICloudError::DownloadTimedOut)
    );
    assert_eq!(clock.elapsed(), Duration::from_secs(10));
    assert_eq!(*probe.requests.borrow(), 1);
    let io: io::Error = ICloudError::DownloadTimedOut.into();
    assert_eq!(io.kind(), io::ErrorKind::TimedOut);
    let kdbx: KdbxError = ICloudError::DownloadTimedOut.into();
    assert!(matches!(
        kdbx,
        KdbxError::Io {
            kind: io::ErrorKind::TimedOut,
            ..
        }
    ));
}

#[test]
fn ensure_downloaded_over_the_platform_fake_waits_for_the_placeholder() {
    use mypass_platform::testing::{CoordinatedOp, FakeCoordinatedFiles};
    let (_d, path) = existing_file();
    let fake = FakeCoordinatedFiles::new();
    fake.set_placeholder(&path, Some(2));
    let clock = FakeClock::new();
    ensure_downloaded(&CoordinatedProbe(&fake), &clock, &path, DOWNLOAD_TIMEOUT).unwrap();
    assert_eq!(fake.count(CoordinatedOp::RequestDownload), 1);
    assert_eq!(clock.elapsed(), Duration::from_millis(600));
}

#[test]
fn placeholder_present_never_reads_as_missing() {
    use mypass_platform::testing::FakeCoordinatedFiles;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.kdbx");
    // Only the legacy placeholder is on disk.
    fs::write(dir.path().join(".vault.kdbx.icloud"), b"plist").unwrap();
    assert!(!path.exists());
    assert!(exists_or_placeholder(&path));
    let fake = FakeCoordinatedFiles::new();
    let clock = FakeClock::new();
    let err =
        ensure_downloaded(&CoordinatedProbe(&fake), &clock, &path, DOWNLOAD_TIMEOUT).unwrap_err();
    assert_eq!(err, ICloudError::DownloadTimedOut);
    assert_eq!(io::Error::from(err).kind(), io::ErrorKind::TimedOut);
    // Without a placeholder a missing file stays missing.
    let gone = dir.path().join("gone.kdbx");
    assert!(!exists_or_placeholder(&gone));
    let err =
        ensure_downloaded(&CoordinatedProbe(&fake), &clock, &gone, DOWNLOAD_TIMEOUT).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

// ---- conflicts ----

/// In-memory conflict versions; `fail_read` makes one version unreadable.
#[derive(Default)]
struct MemVersions {
    versions: Vec<(ConflictVersion, Vec<u8>)>,
    unreadable: Vec<PathBuf>,
    resolved: RefCell<Vec<ConflictVersion>>,
    marks: RefCell<usize>,
}

impl MemVersions {
    fn push(&mut self, name: &str, bytes: Vec<u8>) {
        self.versions.push((
            ConflictVersion {
                path: PathBuf::from(name),
                modified: None,
                saving_computer: None,
            },
            bytes,
        ));
    }
}

impl ConflictVersions for MemVersions {
    fn unresolved(&self, _path: &Path) -> io::Result<Vec<ConflictVersion>> {
        let resolved = self.resolved.borrow();
        Ok(self
            .versions
            .iter()
            .map(|(v, _)| v.clone())
            .filter(|v| !resolved.contains(v))
            .collect())
    }
    fn read_version(&self, version: &ConflictVersion) -> io::Result<Vec<u8>> {
        if self.unreadable.contains(&version.path) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        }
        self.versions
            .iter()
            .find(|(v, _)| v == version)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
    fn mark_resolved(&self, _path: &Path, versions: &[ConflictVersion]) -> io::Result<()> {
        *self.marks.borrow_mut() += 1;
        self.resolved.borrow_mut().extend(versions.iter().cloned());
        Ok(())
    }
}

fn run(
    src: &MemVersions,
    current: Vec<u8>,
    written: &RefCell<Option<Vec<u8>>>,
    fail_write: bool,
) -> io::Result<ConflictOutcome> {
    let password = pw();
    let mut merge = |a: &[u8], b: &[u8]| merge_vault_bytes(a, b, &password);
    let mut write = |bytes: &[u8]| {
        if fail_write {
            return Err(io::Error::other("disk full"));
        }
        *written.borrow_mut() = Some(bytes.to_vec());
        Ok(())
    };
    resolve_conflicts(src, Path::new("v.kdbx"), current, &mut merge, &mut write)
}

#[test]
fn conflict_versions_merged_via_merge_engine() {
    let password = pw();
    let mut a = fast_db("v");
    add(&mut a, "base", 1_700_000_000);
    let mut current = a.clone();
    add(&mut current, "A edited later", 1_700_002_000);
    let mut conflict = a.clone();
    add(&mut conflict, "B", 1_700_001_000);
    let mut src = MemVersions::default();
    src.push("conflict-1", conflict.save(&password).unwrap());
    let written = RefCell::new(None);
    let outcome = run(&src, current.save(&password).unwrap(), &written, false).unwrap();
    assert_eq!(
        outcome,
        ConflictOutcome {
            merged: 1,
            unresolved: 0
        }
    );
    let bytes = written.borrow().clone().expect("the merge is written");
    assert_eq!(
        titles_of(&bytes, &password),
        vec!["A edited later", "B", "base"]
    );
    assert_eq!(src.resolved.borrow().len(), 1);
    // The current file's save settings are kept (fresh salt per save, same parameters).
    let merged = Database::open(&bytes, &password).unwrap();
    let params = |kdf: &KdfParams| match kdf {
        KdfParams::Argon2 {
            variant,
            memory_bytes,
            iterations,
            parallelism,
            ..
        } => (*variant, *memory_bytes, *iterations, *parallelism),
        _ => panic!("argon2 expected"),
    };
    assert_eq!(params(&merged.settings.kdf), params(&current.settings.kdf));
    assert_eq!(merged.settings.cipher, current.settings.cipher);
}

#[test]
fn conflict_resolution_failure_leaves_versions_unresolved() {
    let password = pw();
    let (current, other) = vault_pair(&["base"], &["B"], &password);
    let mut src = MemVersions::default();
    src.push("conflict-1", other);
    let written = RefCell::new(None);
    assert!(run(&src, current, &written, true).is_err());
    assert_eq!(*src.marks.borrow(), 0, "never marked before the write");
    assert!(src.resolved.borrow().is_empty());
}

#[test]
fn unreadable_conflict_version_is_skipped_and_stays_unresolved() {
    let password = pw();
    let (current, b) = vault_pair(&["base"], &["B"], &password);
    let (_, c) = vault_pair(&["base"], &["C"], &password);
    let mut src = MemVersions::default();
    src.push("unreadable", c);
    src.push("readable", b);
    src.unreadable.push(PathBuf::from("unreadable"));
    let written = RefCell::new(None);
    let outcome = run(&src, current, &written, false).unwrap();
    assert_eq!(
        outcome,
        ConflictOutcome {
            merged: 1,
            unresolved: 1
        }
    );
    let resolved = src.resolved.borrow();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].path, PathBuf::from("readable"));
}

#[test]
fn conflict_with_wrong_password_is_left_unresolved_and_reported() {
    let password = pw();
    let (current, _) = vault_pair(&["base"], &[], &password);
    let mut other = fast_db("v");
    add(&mut other, "foreign", 1_700_000_000);
    let mut src = MemVersions::default();
    src.push(
        "other-password",
        other.save(&SecretString::from("another one")).unwrap(),
    );
    let written = RefCell::new(None);
    let outcome = run(&src, current, &written, false).unwrap();
    assert_eq!(
        outcome,
        ConflictOutcome {
            merged: 0,
            unresolved: 1
        }
    );
    assert!(!outcome.changed());
    assert!(
        written.borrow().is_none(),
        "nothing merged, nothing written"
    );
    assert_eq!(*src.marks.borrow(), 0);
}

#[test]
fn no_conflicts_is_noop_false() {
    let password = pw();
    let (current, _) = vault_pair(&["base"], &[], &password);
    let src = MemVersions::default();
    let written = RefCell::new(None);
    let outcome = run(&src, current, &written, false).unwrap();
    assert!(!outcome.changed());
    assert!(written.borrow().is_none());
    assert_eq!(*src.marks.borrow(), 0);
}

#[test]
fn platform_versions_are_read_bounded_and_resolved_through_the_seam() {
    use mypass_platform::testing::FakeCoordinatedFiles;
    let password = pw();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kdbx");
    let (current, b) = vault_pair(&["base"], &["B"], &password);
    fs::write(&path, &current).unwrap();
    let version_path = dir.path().join("version-b");
    fs::write(&version_path, &b).unwrap();
    let fake = FakeCoordinatedFiles::new();
    let version = ConflictVersion {
        path: version_path,
        modified: None,
        saving_computer: Some("Other Mac".into()),
    };
    fake.add_conflict(&path, version.clone());
    // A directory posing as a version is refused (R20), so it stays unresolved.
    let dir_version = ConflictVersion {
        path: dir.path().to_path_buf(),
        modified: None,
        saving_computer: None,
    };
    fake.add_conflict(&path, dir_version.clone());
    let mut merge = |a: &[u8], v: &[u8]| merge_vault_bytes(a, v, &password);
    let mut write = |bytes: &[u8]| fs::write(&path, bytes);
    let outcome = resolve_conflicts(
        &PlatformVersions(&fake),
        &path,
        current,
        &mut merge,
        &mut write,
    )
    .unwrap();
    assert_eq!(
        outcome,
        ConflictOutcome {
            merged: 1,
            unresolved: 1
        }
    );
    assert_eq!(fake.resolved(), vec![version]);
    assert_eq!(
        titles_of(&fs::read(&path).unwrap(), &password),
        vec!["B", "base"]
    );
}
