use super::*;
use std::sync::mpsc::RecvTimeoutError;

/// Generous: hosted runners deliver FSEvents and `ReadDirectoryChangesW` with latency.
const FIRE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a test waits to be sure nothing (more) fires.
const QUIET: Duration = Duration::from_millis(1500);
/// Only a deadlock takes this long.
const DEADLOCK_BOUND: Duration = Duration::from_secs(20);
/// A short debounce for the tests that do not measure the production one.
const TEST_DEBOUNCE: Duration = Duration::from_millis(200);

#[test]
fn debouncer_fires_500ms_after_last_event() {
    let t0 = Instant::now();
    let mut d = Debouncer::new(WATCH_DEBOUNCE);
    assert_eq!(d.deadline(), None);
    assert!(!d.fire_due(t0));
    d.event(t0);
    assert_eq!(d.deadline(), Some(t0 + Duration::from_millis(500)));
    assert!(!d.fire_due(t0 + Duration::from_millis(499)));
    assert!(d.fire_due(t0 + Duration::from_millis(500)));
    // Consumed: it fires once per burst.
    assert!(!d.fire_due(t0 + Duration::from_millis(900)));
    assert_eq!(d.deadline(), None);
}

#[test]
fn debouncer_resets_on_new_event() {
    let t0 = Instant::now();
    let mut d = Debouncer::new(WATCH_DEBOUNCE);
    d.event(t0);
    d.event(t0 + Duration::from_millis(400));
    assert!(!d.fire_due(t0 + Duration::from_millis(500)));
    assert!(!d.fire_due(t0 + Duration::from_millis(899)));
    assert!(d.fire_due(t0 + Duration::from_millis(900)));
}

#[test]
fn echo_filter_recognises_own_write_and_not_foreign_change() {
    let filter = EchoFilter::new();
    let ours = SyncCheckpoint::of_bytes(b"what we uploaded", None);
    assert!(!filter.is_echo(&ours));
    filter.note_own_write(ours.clone().for_vault("v"));
    // The remote as read back after our write: same content, other time, no vault.
    let echo = SyncCheckpoint::of_bytes(b"what we uploaded", Some(std::time::SystemTime::now()));
    assert!(filter.is_echo(&echo));
    let foreign = SyncCheckpoint::of_bytes(b"what they uploaded", None);
    assert!(!filter.is_echo(&foreign));
    filter.clear();
    assert!(!filter.is_echo(&echo));
}

struct Watched {
    dir: tempfile::TempDir,
    fired: Receiver<Instant>,
    watcher: FolderWatcher,
}

impl Watched {
    fn start(debounce: Duration) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (tx, fired) = mpsc::channel();
        let tx = Mutex::new(tx);
        let watcher = FolderWatcher::start(
            dir.path(),
            "vault.kdbx",
            debounce,
            Arc::new(move || {
                let _ = tx.lock().unwrap().send(Instant::now());
            }),
        )
        .unwrap();
        Self {
            dir,
            fired,
            watcher,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn expect_fire(&self) -> Instant {
        self.fired
            .recv_timeout(FIRE_TIMEOUT)
            .expect("the watcher fired")
    }
    fn expect_quiet(&self) {
        assert_eq!(
            self.fired.recv_timeout(QUIET),
            Err(RecvTimeoutError::Timeout),
            "the watcher fired"
        );
    }
}

#[test]
fn external_write_fires_once_after_debounce() {
    let w = Watched::start(WATCH_DEBOUNCE);
    // Taken before the write: the event (and so the debounce) can only start after it, whatever
    // the scheduling, so this lower bound holds deterministically.
    let before_write = Instant::now();
    fs::write(w.path("vault.kdbx"), b"from another device").unwrap();
    let fired = w.expect_fire();
    assert!(fired.duration_since(before_write) >= WATCH_DEBOUNCE);
    w.expect_quiet();
}

#[test]
fn burst_of_ten_writes_coalesces_into_one_callback() {
    let w = Watched::start(TEST_DEBOUNCE);
    for i in 0..10u8 {
        fs::write(w.path("vault.kdbx"), [i; 64]).unwrap();
    }
    // Exact coalescing is pinned by the pure `Debouncer` tests; with real notifications a loaded
    // runner may spread the writes over more than one debounce window. Here: the burst is
    // delivered, never once per event, and the watcher settles.
    w.expect_fire();
    let mut extra = 0;
    while w.fired.recv_timeout(QUIET).is_ok() {
        extra += 1;
        assert!(extra < 10, "the watcher fired once per write");
    }
}

#[test]
fn other_files_in_folder_do_not_fire() {
    let w = Watched::start(TEST_DEBOUNCE);
    fs::write(w.path("notes.txt"), b"x").unwrap();
    fs::write(w.path("vault.kdbx.bak"), b"x").unwrap();
    fs::create_dir(w.path("sub")).unwrap();
    fs::write(w.path("sub").join("vault.kdbx"), b"x").unwrap();
    w.expect_quiet();
    // Positive control: the watch is live.
    fs::write(w.path("vault.kdbx"), b"x").unwrap();
    w.expect_fire();
}

#[test]
fn temp_file_of_atomic_write_does_not_fire_by_itself() {
    let w = Watched::start(TEST_DEBOUNCE);
    let temp = w.path(".vault.kdbx.4242.1794690000000000000.tmp");
    fs::write(&temp, b"new vault bytes").unwrap();
    w.expect_quiet();
    // The rename that completes the atomic write fires, once.
    fs::rename(&temp, w.path("vault.kdbx")).unwrap();
    w.expect_fire();
    w.expect_quiet();
}

#[test]
fn atomic_write_through_the_backend_fires_once() {
    let w = Watched::start(TEST_DEBOUNCE);
    mypass_kdbx::write_atomic(&w.path("vault.kdbx"), b"bytes").unwrap();
    w.expect_fire();
    w.expect_quiet();
}

#[test]
fn stop_prevents_further_callbacks() {
    let w = Watched::start(TEST_DEBOUNCE);
    fs::write(w.path("vault.kdbx"), b"1").unwrap();
    w.expect_fire();
    let Watched {
        dir,
        fired,
        watcher,
    } = w;
    // A change whose firing is still pending when the watcher stops is dropped.
    fs::write(dir.path().join("vault.kdbx"), b"2").unwrap();
    watcher.stop();
    fs::write(dir.path().join("vault.kdbx"), b"3").unwrap();
    // The callback (and its sender) is gone once `stop` returned: the channel only disconnects.
    assert_eq!(
        fired.recv_timeout(QUIET),
        Err(RecvTimeoutError::Disconnected)
    );
}

#[test]
fn start_on_missing_dir_is_unavailable() {
    let d = tempfile::tempdir().unwrap();
    let missing = d.path().join("unplugged");
    let err =
        FolderWatcher::start(&missing, "vault.kdbx", WATCH_DEBOUNCE, Arc::new(|| {})).unwrap_err();
    assert!(matches!(err, StorageError::Unavailable(_)), "{err:?}");
    assert!(!missing.exists());
}

#[test]
fn dropping_the_watcher_from_its_own_callback_does_not_deadlock() {
    let d = tempfile::tempdir().unwrap();
    let slot: Arc<Mutex<Option<FolderWatcher>>> = Arc::new(Mutex::new(None));
    let (tx, done) = mpsc::channel();
    let tx = Mutex::new(tx);
    let inner = slot.clone();
    let watcher = FolderWatcher::start(
        d.path(),
        "vault.kdbx",
        TEST_DEBOUNCE,
        Arc::new(move || {
            drop(inner.lock().unwrap().take());
            let _ = tx.lock().unwrap().send(());
        }),
    )
    .unwrap();
    *slot.lock().unwrap() = Some(watcher);
    // A real event source may miss or delay a change made right after the watch starts, so keep
    // changing the file until the callback runs; the bound only catches a deadlock.
    let deadline = Instant::now() + DEADLOCK_BOUND;
    for i in 0u8.. {
        fs::write(d.path().join("vault.kdbx"), [i; 8]).unwrap();
        match done.recv_timeout(Duration::from_secs(1)) {
            Ok(()) => break,
            Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(e) => panic!("the callback never completed: {e:?}"),
        }
    }
    assert!(slot.lock().unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn non_ascii_name_in_another_normalisation_names_the_same_file() {
    let d = tempfile::tempdir().unwrap();
    // A Ukrainian name ending in the precomposed short i (U+0439), as the app holds it (NFC).
    let nfc = "\u{41c}\u{456}\u{439}.kdbx";
    // The same name with it decomposed (U+0438 U+0306), as a file system may report it (NFD).
    let nfd = "\u{41c}\u{456}\u{438}\u{306}.kdbx";
    fs::write(d.path().join(nfc), b"x").unwrap();
    let filter = NameFilter {
        dir: d.path().to_path_buf(),
        name: OsString::from(nfc),
    };
    assert!(filter.names_the_vault(&d.path().join(nfc)));
    // On a normalisation-insensitive file system (APFS) both spellings open the same file; on a
    // byte-exact one (ext4) the second spelling does not exist and must not match.
    let same = fs::metadata(d.path().join(nfd)).is_ok();
    assert_eq!(filter.names_the_vault(&d.path().join(nfd)), same);
    assert!(!filter.names_the_vault(&d.path().join("other.kdbx")));
}

#[test]
fn reads_and_rescans_are_classified() {
    use notify::event::{AccessKind, AccessMode, Flag, ModifyKind};
    let filter = NameFilter {
        dir: PathBuf::from("/nowhere"),
        name: OsString::from("vault.kdbx"),
    };
    let vault = PathBuf::from("/nowhere/vault.kdbx");
    let ev = |kind| Ok(notify::Event::new(kind).add_path(vault.clone()));
    assert!(!filter.is_relevant(&ev(EventKind::Access(AccessKind::Read))));
    assert!(!filter.is_relevant(&ev(EventKind::Access(AccessKind::Open(AccessMode::Any)))));
    assert!(filter.is_relevant(&ev(EventKind::Access(AccessKind::Close(AccessMode::Write)))));
    assert!(filter.is_relevant(&ev(EventKind::Modify(ModifyKind::Any))));
    let rescan = Ok(notify::Event::new(EventKind::Other).set_flag(Flag::Rescan));
    assert!(filter.is_relevant(&rescan));
    assert!(filter.is_relevant(&Err(notify::Error::generic("lost"))));
    let other = Ok(notify::Event::new(EventKind::Modify(ModifyKind::Any))
        .add_path(PathBuf::from("/nowhere/notes.txt")));
    assert!(!filter.is_relevant(&other));
}
