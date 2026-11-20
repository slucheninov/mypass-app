use super::*;
use crate::{
    backend::StorageError,
    checkpoint::{FileCheckpointStore, InMemoryCheckpointStore, SyncCheckpoint},
    local_folder::LocalFolderBackend,
};
use mypass_kdbx::{Argon2Variant, Cipher, Compression, Entry, KdfParams, Timestamp};
use std::{
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::SystemTime,
};
use tempfile::TempDir;

fn pw() -> SecretString {
    SecretString::from("correct horse battery")
}

/// Argon2id 8 MiB, one pass: real KDBX, fast enough for tests.
fn fast_db(name: &str) -> Database {
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

fn add(db: &mut Database, title: &str) {
    let mut e = Entry::new(Timestamp::now());
    e.set_field("Title", title, false);
    let root = db.root.uuid;
    db.add_entry(root, e, Timestamp::now()).unwrap();
}

fn titles(db: &Database) -> Vec<String> {
    let mut t: Vec<String> = db
        .root
        .entries
        .iter()
        .filter_map(|e| e.field("Title").map(str::to_owned))
        .collect();
    t.sort();
    t
}

type Hook = Mutex<Box<dyn FnMut(usize) + Send>>;

fn no_hook() -> Hook {
    Mutex::new(Box::new(|_| {}))
}

/// A backend wrapper that counts calls and can act around the n-th conditional write (a
/// concurrent writer), or report that the n-th write displaced another writer's version.
struct Probe {
    inner: Arc<LocalFolderBackend>,
    reads: AtomicUsize,
    /// Writes that changed the remote (unconditional, or conditional and not refused).
    writes: AtomicUsize,
    write_attempts: AtomicUsize,
    /// Runs at the start of a conditional write, before the storage's final check.
    on_write: Hook,
    /// Runs right after a conditional write returned, before the engine sees the outcome.
    after_write: Hook,
    /// Reported as the displaced version of the next write that lands.
    displace_next: Mutex<Option<Vec<u8>>>,
    /// Like `displace_next`, but the copy survives only under this hidden file name.
    displace_hidden_next: Mutex<Option<(Vec<u8>, PathBuf)>>,
}
impl Probe {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}
impl StorageBackend for Probe {
    fn exists(&self) -> Result<bool, StorageError> {
        self.inner.exists()
    }
    fn modification_date(&self) -> Result<Option<SystemTime>, StorageError> {
        self.inner.modification_date()
    }
    fn read(&self) -> Result<Option<Vec<u8>>, StorageError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read()
    }
    fn write(&self, bytes: &[u8]) -> Result<(), StorageError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.write(bytes)
    }
    fn write_if(
        &self,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
    ) -> Result<WriteOutcome, StorageError> {
        let n = self.write_attempts.fetch_add(1, Ordering::SeqCst) + 1;
        (self.on_write.lock().unwrap())(n);
        let outcome = self.inner.write_if(expected, bytes)?;
        if !matches!(outcome, WriteOutcome::Conflict) {
            self.writes.fetch_add(1, Ordering::SeqCst);
        }
        (self.after_write.lock().unwrap())(n);
        if matches!(outcome, WriteOutcome::Written) {
            if let Some(theirs) = self.displace_next.lock().unwrap().take() {
                return Ok(WriteOutcome::Displaced(theirs));
            }
            if let Some((bytes, path)) = self.displace_hidden_next.lock().unwrap().take() {
                return Ok(WriteOutcome::DisplacedKept { bytes, path });
            }
        }
        Ok(outcome)
    }
}

struct Fx {
    _dir: TempDir,
    file: PathBuf,
    folder: Arc<LocalFolderBackend>,
    probe: Arc<Probe>,
    store: Arc<InMemoryCheckpointStore>,
    engine: SyncEngine,
}

impl Fx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let folder = Arc::new(LocalFolderBackend::new(dir.path(), "vault.kdbx").unwrap());
        let probe = Arc::new(Probe {
            inner: folder.clone(),
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            write_attempts: AtomicUsize::new(0),
            on_write: no_hook(),
            after_write: no_hook(),
            displace_next: Mutex::new(None),
            displace_hidden_next: Mutex::new(None),
        });
        let store = Arc::new(InMemoryCheckpointStore::new());
        let engine = SyncEngine::new(probe.clone(), store.clone());
        Self {
            file: dir.path().join("vault.kdbx"),
            _dir: dir,
            folder,
            probe,
            store,
            engine,
        }
    }
    fn remote_bytes(&self) -> Vec<u8> {
        fs::read(&self.file).unwrap()
    }
    fn remote_db(&self) -> Database {
        Database::open(&self.remote_bytes(), &pw()).unwrap()
    }
    /// Another device writes `db` to the remote (not through the engine).
    fn external_write(&self, db: &Database) {
        self.folder.write(&db.save(&pw()).unwrap()).unwrap();
    }
    /// `f(n)` runs at the start of the n-th conditional write from now on.
    fn on_write(&self, mut f: impl FnMut(usize) + Send + 'static) {
        let start = self.probe.write_attempts.load(Ordering::SeqCst);
        *self.probe.on_write.lock().unwrap() = Box::new(move |n| f(n - start));
    }
    /// `f(n)` runs right after the n-th conditional write from now on returned.
    fn after_write(&self, mut f: impl FnMut(usize) + Send + 'static) {
        let start = self.probe.write_attempts.load(Ordering::SeqCst);
        *self.probe.after_write.lock().unwrap() = Box::new(move |n| f(n - start));
    }
    fn checkpoint(&self) -> Option<SyncCheckpoint> {
        self.store.load().unwrap()
    }
}

fn merged(result: SyncResult) -> MergedRemote {
    match result {
        SyncResult::Merged(m) => *m,
        other => panic!("expected Merged, got {other:?}"),
    }
}

/// The caller's side of a merge: merge into the live vault (durable at once in these tests),
/// then commit the checkpoint.
fn adopt(engine: &SyncEngine, live: &mut Database, m: MergedRemote) {
    let (db, pending) = m.into_parts();
    *live = merge(live, &db);
    engine.commit(pending).unwrap();
}

fn mode(push: bool) -> SyncMode {
    if push {
        SyncMode::Push
    } else {
        SyncMode::Pull
    }
}

/// A caller whose local vault is always durable: sync, then commit the checkpoint of every
/// result that needs no adoption. A `Merged` result is returned uncommitted (see `adopt`).
fn run(engine: &SyncEngine, local: &Database, push: bool) -> Result<SyncResult, SyncError> {
    let result = engine.sync(local, &pw(), mode(push))?;
    if !matches!(result, SyncResult::Merged(_)) {
        if let Some(pending) = result.pending() {
            engine.commit(pending.clone())?;
        }
    }
    Ok(result)
}

#[test]
fn first_sync_uploads_initial() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    let r = run(&fx.engine, &local, false).unwrap();
    assert!(matches!(r, SyncResult::UploadedInitial(_)), "{r:?}");
    assert_eq!(titles(&fx.remote_db()), ["A"]);
    let cp = fx.checkpoint().unwrap();
    assert!(cp.describes(&fx.remote_bytes()));
    assert_eq!(cp.vault_uuid, Some(local.root.uuid.to_string()));
    assert!(cp.modified_secs.is_some());
}

#[test]
fn unchanged_remote_without_push_does_nothing() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    run(&fx.engine, &local, false).unwrap();
    let before = fx.remote_bytes();
    let writes = fx.probe.writes();
    add(&mut local, "B");
    let r = run(&fx.engine, &local, false).unwrap();
    assert!(matches!(r, SyncResult::Unchanged(_)), "{r:?}");
    assert_eq!(fx.remote_bytes(), before);
    assert_eq!(fx.probe.writes(), writes);
}

#[test]
fn unchanged_remote_with_push_uploads_local() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    run(&fx.engine, &local, false).unwrap();
    add(&mut local, "B");
    let r = run(&fx.engine, &local, true).unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal(_)), "{r:?}");
    assert_eq!(titles(&fx.remote_db()), ["A", "B"]);
    assert!(fx.checkpoint().unwrap().describes(&fx.remote_bytes()));
}

#[test]
fn changed_remote_merges_and_uploads_union() {
    let fx = Fx::new();
    let base = fast_db("v");
    let mut remote = base.clone();
    add(&mut remote, "B");
    fx.external_write(&remote);
    let mut local = base.clone();
    add(&mut local, "A");
    let before = local.clone();
    let m = merged(run(&fx.engine, &local, false).unwrap());
    assert_eq!(titles(m.database()), ["A", "B"]);
    assert_eq!(titles(&fx.remote_db()), ["A", "B"]);
    // The engine never touches `local`.
    assert!(local == before);
}

#[test]
fn merged_result_checkpoint_matches_written_file() {
    let fx = Fx::new();
    let base = fast_db("v");
    let mut remote = base.clone();
    add(&mut remote, "B");
    fx.external_write(&remote);
    let mut local = base.clone();
    add(&mut local, "A");
    let m = merged(run(&fx.engine, &local, false).unwrap());
    let (db, pending) = m.into_parts();
    assert!(pending.checkpoint().describes(&fx.remote_bytes()));
    // Not stored before the caller commits.
    assert_eq!(fx.checkpoint(), None);
    local = merge(&local, &db);
    fx.engine.commit(pending).unwrap();
    assert!(fx.checkpoint().unwrap().describes(&fx.remote_bytes()));
    // The anti-ping-pong base: the next sync without changes is not another merge.
    let writes = fx.probe.writes();
    let r = run(&fx.engine, &local, false).unwrap();
    assert!(matches!(r, SyncResult::Unchanged(_)), "{r:?}");
    assert_eq!(fx.probe.writes(), writes);
}

#[test]
fn uncommitted_merge_is_merged_again_not_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    let mut remote = fx.remote_db();
    add(&mut remote, "B");
    fx.external_write(&remote);
    let mut local = base.clone();
    add(&mut local, "A");
    // The caller fails to adopt the merge and never commits it.
    let _ = merged(run(&fx.engine, &local, false).unwrap());
    // A push from the stale local vault must not drop the remote's "B".
    let m = merged(run(&fx.engine, &local, true).unwrap());
    assert_eq!(titles(m.database()), ["A", "B"]);
    assert_eq!(titles(&fx.remote_db()), ["A", "B"]);
}

#[test]
fn wrong_password_remote_is_never_overwritten() {
    let fx = Fx::new();
    let other = fast_db("v");
    fx.folder
        .write(&other.save(&SecretString::from("other password")).unwrap())
        .unwrap();
    let before = fx.remote_bytes();
    let local = fast_db("v");
    for push in [false, true] {
        assert_eq!(
            run(&fx.engine, &local, push).unwrap_err(),
            SyncError::RemotePasswordMismatch
        );
    }
    assert_eq!(fx.remote_bytes(), before);
    assert_eq!(fx.probe.writes(), 0);
    assert_eq!(fx.checkpoint(), None);
}

#[test]
fn remote_kdf_above_limit_is_neither_adopted_nor_overwritten() {
    let fx = Fx::new();
    let local = fast_db("v");
    let before = mypass_kdbx::test_util::vault_header_only(&KdfParams::AesKdf {
        seed: vec![0; 32],
        rounds: 300_000_001,
    });
    fs::write(&fx.file, &before).unwrap();
    for push in [false, true] {
        assert!(matches!(
            run(&fx.engine, &local, push),
            Err(SyncError::SyncKdfLimitExceeded {
                recovery_file: None
            })
        ));
    }
    assert_eq!(fx.remote_bytes(), before);
    assert_eq!(fx.probe.writes(), 0);
    assert_eq!(fx.checkpoint(), None);
}

#[test]
fn sync_accepts_keepassxc_like_costs() {
    let fx = Fx::new();
    let local = fast_db("v");
    let mut remote = local.clone();
    let KdfParams::Argon2 {
        parallelism,
        iterations,
        ..
    } = &mut remote.settings.kdf
    else {
        panic!("expected Argon2");
    };
    *parallelism = 16;
    *iterations = 20;
    add(&mut remote, "remote entry");
    fx.external_write(&remote);

    let result = run(&fx.engine, &local, false).unwrap();
    let (merged, _) = result.into_parts();
    assert_eq!(titles(&merged.unwrap()), ["remote entry"]);
}

#[test]
fn non_kdbx_remote_is_never_overwritten() {
    let fx = Fx::new();
    fs::write(&fx.file, b"just some notes, not a vault").unwrap();
    let local = fast_db("v");
    assert_eq!(
        run(&fx.engine, &local, true).unwrap_err(),
        SyncError::RemoteNotAVault
    );
    assert_eq!(fx.remote_bytes(), b"just some notes, not a vault");
    assert_eq!(fx.probe.writes(), 0);
}

#[test]
fn damaged_remote_is_never_overwritten() {
    let fx = Fx::new();
    let local = fast_db("v");
    run(&fx.engine, &local, false).unwrap();
    let mut bytes = fx.remote_bytes();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    fs::write(&fx.file, &bytes).unwrap();
    let err = run(&fx.engine, &local, true).unwrap_err();
    assert!(
        matches!(
            err,
            SyncError::RemoteCorrupt(_) | SyncError::RemotePasswordMismatch
        ),
        "{err:?}"
    );
    assert_eq!(fx.remote_bytes(), bytes);
}

#[test]
fn same_size_different_content_remote_is_detected_by_hash() {
    let fx = Fx::new();
    let mut base = fast_db("v");
    // No compression: the file length depends only on the content length.
    base.settings.compression = Compression::None;
    add(&mut base, "AAAA");
    run(&fx.engine, &base, false).unwrap();
    let before = fx.remote_bytes();
    let mut remote = fx.remote_db();
    let entry = &mut remote.root.entries[0];
    entry.set_field("Title", "BBBB", false);
    // Later than the local copy, so the merge takes the remote title (KDBX keeps seconds).
    entry.modified = Timestamp(entry.modified.0 + 60);
    let bytes = remote.save(&pw()).unwrap();
    assert_eq!(bytes.len(), before.len(), "same size, different content");
    assert_ne!(bytes, before);
    fs::write(&fx.file, &bytes).unwrap();
    // A size or date check would push the stale local vault over "BBBB"; the hash does not.
    let r = run(&fx.engine, &base, true).unwrap();
    let m = merged(r);
    assert_eq!(titles(m.database()), ["BBBB"]);
    assert_eq!(titles(&fx.remote_db()), ["BBBB"]);
}

#[test]
fn engine_preserves_kdf_params_on_upload() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    local.settings.cipher = Cipher::ChaCha20;
    if let KdfParams::Argon2 { iterations, .. } = &mut local.settings.kdf {
        *iterations = 2;
    }
    add(&mut local, "A");
    run(&fx.engine, &local, false).unwrap();
    let kdf_shape = |k: &KdfParams| match k {
        KdfParams::Argon2 {
            variant,
            memory_bytes,
            iterations,
            parallelism,
            version,
            ..
        } => (*variant, *memory_bytes, *iterations, *parallelism, *version),
        other => panic!("unexpected KDF {other:?}"),
    };
    let remote = fx.remote_db();
    assert_eq!(remote.settings.cipher, Cipher::ChaCha20);
    assert_eq!(
        kdf_shape(&remote.settings.kdf),
        kdf_shape(&local.settings.kdf)
    );
    // A merge upload keeps them too.
    let mut other = remote.clone();
    add(&mut other, "B");
    fx.external_write(&other);
    add(&mut local, "C");
    let _ = merged(run(&fx.engine, &local, false).unwrap());
    let after = fx.remote_db();
    assert_eq!(after.settings.cipher, Cipher::ChaCha20);
    assert_eq!(
        kdf_shape(&after.settings.kdf),
        kdf_shape(&local.settings.kdf)
    );
}

#[test]
fn remote_of_a_different_vault_is_never_overwritten() {
    let fx = Fx::new();
    let mut other = fast_db("other vault");
    add(&mut other, "Theirs");
    fx.external_write(&other);
    let before = fx.remote_bytes();
    let mut local = fast_db("mine");
    add(&mut local, "Mine");
    assert_eq!(
        run(&fx.engine, &local, true).unwrap_err(),
        SyncError::RemoteIsDifferentVault
    );
    assert_eq!(fx.remote_bytes(), before);
}

#[test]
fn checkpoint_of_another_vault_does_not_allow_a_blind_push() {
    let fx = Fx::new();
    let mut a = fast_db("a");
    add(&mut a, "A");
    run(&fx.engine, &a, false).unwrap();
    let before = fx.remote_bytes();
    // Same store and remote, but now the local vault is another one.
    let b = fast_db("b");
    assert_eq!(
        run(&fx.engine, &b, true).unwrap_err(),
        SyncError::RemoteIsDifferentVault
    );
    assert_eq!(fx.remote_bytes(), before);
}

#[test]
fn remote_changed_between_read_and_write_is_merged_not_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    let concurrent = theirs.save(&pw()).unwrap();
    let folder = fx.folder.clone();
    // Another device commits right before the storage's final check of our first write.
    fx.on_write(move |n| {
        if n == 1 {
            folder.write(&concurrent).unwrap();
        }
    });
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
    assert!(m.into_parts().1.checkpoint().describes(&fx.remote_bytes()));
}

#[test]
fn remote_that_keeps_changing_is_never_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    let writes = fx.probe.writes();
    let folder = fx.folder.clone();
    let seed = fx.remote_db();
    // Every write meets a fresh external encoding at the final check, so every one is refused.
    fx.on_write(move |_| {
        folder.write(&seed.save(&pw()).unwrap()).unwrap();
    });
    let mut local = base.clone();
    add(&mut local, "Mine");
    assert_eq!(
        fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap_err(),
        SyncError::RemoteChangedDuringSync
    );
    assert_eq!(fx.probe.writes(), writes);
    assert!(titles(&fx.remote_db()).is_empty());
}

#[test]
fn remote_appearing_during_initial_upload_is_not_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    let mut theirs = base.clone();
    add(&mut theirs, "Theirs");
    let appearing = theirs.save(&pw()).unwrap();
    let folder = fx.folder.clone();
    // The file appears after our read found none, right before the initial write lands.
    fx.on_write(move |n| {
        if n == 1 {
            folder.write(&appearing).unwrap();
        }
    });
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(fx.engine.sync(&local, &pw(), SyncMode::Pull).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
}

#[test]
fn identical_content_without_checkpoint_is_not_rewritten() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    fx.external_write(&local);
    let local = fx.remote_db();
    let before = fx.remote_bytes();
    let r = run(&fx.engine, &local, true).unwrap();
    assert!(matches!(r, SyncResult::Unchanged(_)), "{r:?}");
    assert_eq!(fx.remote_bytes(), before);
    assert_eq!(fx.probe.writes(), 0);
    assert!(fx.checkpoint().unwrap().describes(&before));
}

#[test]
fn stale_remote_without_checkpoint_gets_the_local_vault() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    fx.external_write(&local);
    let mut local = fx.remote_db();
    add(&mut local, "B");
    let r = run(&fx.engine, &local, false).unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal(_)), "{r:?}");
    assert_eq!(titles(&fx.remote_db()), ["A", "B"]);
    assert!(fx.checkpoint().unwrap().describes(&fx.remote_bytes()));
}

#[test]
fn remote_ahead_of_local_is_adopted_without_a_write() {
    let fx = Fx::new();
    let local = fast_db("v");
    fx.external_write(&local);
    let mut theirs = fx.remote_db();
    add(&mut theirs, "B");
    fx.external_write(&theirs);
    let local = fx.remote_db_with_titles_removed();
    let writes = fx.probe.writes();
    let m = merged(run(&fx.engine, &local, true).unwrap());
    assert_eq!(titles(m.database()), ["B"]);
    assert_eq!(fx.probe.writes(), writes);
    assert!(m.into_parts().1.checkpoint().describes(&fx.remote_bytes()));
}

impl Fx {
    /// The remote vault without its entries (a local copy that is behind the remote).
    fn remote_db_with_titles_removed(&self) -> Database {
        let mut db = self.remote_db();
        db.root.entries.clear();
        db
    }
}

#[test]
fn two_devices_converge_without_ping_pong() {
    let fx = Fx::new();
    let y_store = Arc::new(InMemoryCheckpointStore::new());
    let y_engine = SyncEngine::new(fx.probe.clone(), y_store);
    let base = fast_db("v");
    let mut x = base.clone();
    add(&mut x, "X");
    let mut y = base.clone();
    add(&mut y, "Y");
    run(&fx.engine, &x, true).unwrap();
    let m = merged(run(&y_engine, &y, true).unwrap());
    adopt(&y_engine, &mut y, m);
    let m = merged(run(&fx.engine, &x, false).unwrap());
    adopt(&fx.engine, &mut x, m);
    assert_eq!(titles(&x), ["X", "Y"]);
    assert_eq!(titles(&y), ["X", "Y"]);
    let writes = fx.probe.writes();
    for _ in 0..3 {
        for (engine, db) in [(&fx.engine, &x), (&y_engine, &y)] {
            let r = run(engine, db, false).unwrap();
            assert!(matches!(r, SyncResult::Unchanged(_)), "{r:?}");
        }
    }
    assert_eq!(fx.probe.writes(), writes);
}

#[test]
fn too_large_remote_is_refused_and_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let folder = LocalFolderBackend::new(dir.path(), "vault.kdbx")
        .unwrap()
        .with_read_limit(16);
    fs::write(dir.path().join("vault.kdbx"), [1u8; 32]).unwrap();
    let engine = SyncEngine::new(Arc::new(folder), Arc::new(InMemoryCheckpointStore::new()));
    assert_eq!(
        run(&engine, &fast_db("v"), true).unwrap_err(),
        SyncError::Storage(StorageError::TooLarge { max_bytes: 16 })
    );
    assert_eq!(fs::read(dir.path().join("vault.kdbx")).unwrap(), [1u8; 32]);
}

#[test]
fn non_regular_remote_is_refused_and_left_alone() {
    let fx = Fx::new();
    fs::create_dir(&fx.file).unwrap();
    assert_eq!(
        run(&fx.engine, &fast_db("v"), true).unwrap_err(),
        SyncError::Storage(StorageError::NotRegularFile)
    );
    assert!(fx.file.is_dir());
    assert_eq!(fx.probe.writes(), 0);
}

#[test]
fn unavailable_folder_fails_without_creating_it() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("unplugged");
    let folder = LocalFolderBackend::new(&missing, "vault.kdbx").unwrap();
    let engine = SyncEngine::new(Arc::new(folder), Arc::new(InMemoryCheckpointStore::new()));
    assert!(matches!(
        run(&engine, &fast_db("v"), true).unwrap_err(),
        SyncError::Storage(StorageError::Unavailable(_))
    ));
    assert!(!missing.exists());
}

/// A store whose saves fail while `failing` is set.
struct FlakyStore {
    inner: InMemoryCheckpointStore,
    failing: std::sync::atomic::AtomicBool,
}
impl SyncCheckpointStore for FlakyStore {
    fn load(&self) -> io::Result<Option<SyncCheckpoint>> {
        self.inner.load()
    }
    fn save(&self, cp: Option<&SyncCheckpoint>) -> io::Result<()> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(io::Error::other("disk full"));
        }
        self.inner.save(cp)
    }
}

#[test]
fn failed_checkpoint_after_upload_leads_to_a_merge_not_a_blind_push() {
    let fx = Fx::new();
    let store = Arc::new(FlakyStore {
        inner: InMemoryCheckpointStore::new(),
        failing: true.into(),
    });
    let engine = SyncEngine::new(fx.probe.clone(), store.clone());
    let base = fast_db("v");
    let pending = engine.sync(&base, &pw(), SyncMode::Pull).unwrap();
    assert!(matches!(
        engine
            .commit(pending.pending().unwrap().clone())
            .unwrap_err(),
        SyncError::Checkpoint(_)
    ));
    store.failing.store(false, Ordering::SeqCst);
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    fx.external_write(&theirs);
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(run(&engine, &local, true).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
}

#[test]
fn engine_notes_its_own_writes_in_the_echo_filter() {
    let fx = Fx::new();
    let echo = Arc::new(crate::watcher::EchoFilter::new());
    let engine = SyncEngine::new(fx.probe.clone(), Arc::new(InMemoryCheckpointStore::new()))
        .with_echo_filter(echo.clone());
    let mut local = fast_db("v");
    add(&mut local, "A");
    run(&engine, &local, false).unwrap();
    let ours = SyncCheckpoint::of_bytes(&fx.remote_bytes(), None);
    assert!(echo.is_echo(&ours));
    let mut theirs = fx.remote_db();
    add(&mut theirs, "B");
    fx.external_write(&theirs);
    assert!(!echo.is_echo(&SyncCheckpoint::of_bytes(&fx.remote_bytes(), None)));
    // The merge upload is noted too.
    add(&mut local, "C");
    let _ = merged(run(&engine, &local, false).unwrap());
    assert!(echo.is_echo(&SyncCheckpoint::of_bytes(&fx.remote_bytes(), None)));
}

#[test]
fn sync_never_stores_a_checkpoint_before_the_caller_commits() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    let r = fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap();
    assert!(matches!(r, SyncResult::UploadedInitial(_)), "{r:?}");
    assert_eq!(fx.checkpoint(), None);
    add(&mut local, "B");
    // Without the commit the next push is still a merge-checked write, not a blind one.
    let r = fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal(_)), "{r:?}");
    assert_eq!(fx.checkpoint(), None);
}

/// C1, crash-point simulation: the merge reached the remote, but the device stopped before the
/// merged vault reached its own disk. After a restart (a fresh engine over the same checkpoint
/// file) a push of the stale local vault must merge the remote's entry back, never write over it.
#[test]
fn restart_after_an_unadopted_merge_merges_again_instead_of_pushing_blindly() {
    let fx = Fx::new();
    let state = tempfile::tempdir().unwrap();
    let cp_path = state.path().join("secondary.json");
    let engine = SyncEngine::new(
        fx.probe.clone(),
        Arc::new(FileCheckpointStore::new(&cp_path)),
    );
    let base = fast_db("v");
    run(&engine, &base, false).unwrap();
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    fx.external_write(&theirs);
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(engine.sync(&local, &pw(), SyncMode::Push).unwrap());
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
    // Crash: the merge is neither adopted nor durable locally, and nothing is committed.
    drop(m);
    drop(engine);
    let restarted = SyncEngine::new(
        fx.probe.clone(),
        Arc::new(FileCheckpointStore::new(&cp_path)),
    );
    add(&mut local, "Later");
    let m = merged(restarted.sync(&local, &pw(), SyncMode::Push).unwrap());
    assert_eq!(titles(m.database()), ["Later", "Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Later", "Mine", "Theirs"]);
}

/// C2: a writer that replaces the file right after our write landed (before the read-back) is
/// merged by the same sync, and our checkpoint never describes a remote it did not see.
#[test]
fn writer_right_after_our_write_is_merged_not_lost() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    let concurrent = theirs.save(&pw()).unwrap();
    let folder = fx.folder.clone();
    fx.after_write(move |n| {
        if n == 1 {
            folder.write(&concurrent).unwrap();
        }
    });
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
    assert!(m.into_parts().1.checkpoint().describes(&fx.remote_bytes()));
}

/// C2: a version our write displaced in the last instant (reported by the storage) is merged
/// back onto the remote by the next attempt.
#[test]
fn displaced_version_is_merged_back_onto_the_remote() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    *fx.probe.displace_next.lock().unwrap() = Some(theirs.save(&pw()).unwrap());
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
}

#[test]
fn displaced_version_that_cannot_be_merged_is_reported() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    *fx.probe.displace_next.lock().unwrap() = Some(b"not a vault".to_vec());
    let mut local = base.clone();
    add(&mut local, "Mine");
    assert_eq!(
        fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap_err(),
        SyncError::ConcurrentCopyKept
    );
}

/// A competing copy that survives only under a hidden name and cannot be merged must reach the
/// user with that file name (a name only), not as a bare "kept".
#[test]
fn hidden_displaced_copy_that_cannot_be_merged_names_its_file() {
    for bytes in [b"not a vault".to_vec(), {
        let mut other = fast_db("other");
        add(&mut other, "Theirs");
        other.save(&pw()).unwrap()
    }] {
        let fx = Fx::new();
        let base = fast_db("v");
        run(&fx.engine, &base, false).unwrap();
        *fx.probe.displace_hidden_next.lock().unwrap() =
            Some((bytes, ".vault.kdbx.1.2.3.tmp".into()));
        let mut local = base.clone();
        add(&mut local, "Mine");
        assert_eq!(
            fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap_err(),
            SyncError::Storage(StorageError::VaultRetained {
                path: ".vault.kdbx.1.2.3.tmp".into()
            })
        );
    }
}

#[test]
fn over_budget_hidden_displaced_copy_reports_limit_and_file_name() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    let bytes = mypass_kdbx::test_util::vault_header_only(&KdfParams::AesKdf {
        seed: vec![0; 32],
        rounds: 300_000_001,
    });
    let path = PathBuf::from(".vault.kdbx.1.2.3.tmp");
    *fx.probe.displace_hidden_next.lock().unwrap() = Some((bytes, path.clone()));
    let mut local = base.clone();
    add(&mut local, "Mine");

    assert_eq!(
        fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap_err(),
        SyncError::SyncKdfLimitExceeded {
            recovery_file: Some(path),
        }
    );
}

/// A hidden copy that merges is merged back like any displaced one.
#[test]
fn hidden_displaced_copy_that_can_be_merged_is_merged_back() {
    let fx = Fx::new();
    let base = fast_db("v");
    run(&fx.engine, &base, false).unwrap();
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    *fx.probe.displace_hidden_next.lock().unwrap() =
        Some((theirs.save(&pw()).unwrap(), ".vault.kdbx.1.2.3.tmp".into()));
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(fx.engine.sync(&local, &pw(), SyncMode::Push).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
}

#[test]
fn withdrawal_after_a_hidden_displacement_keeps_its_recovery_path() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    run(&fx.engine, &local, false).unwrap();
    add(&mut local, "local edit");
    let kept = PathBuf::from("/resolved/.vault.kdbx.1.2.3.aside");
    *fx.probe.displace_hidden_next.lock().unwrap() = Some((vec![], kept.clone()));

    assert_eq!(
        fx.engine
            .sync_checked(&local, &pw(), SyncMode::Push, &|| {
                fx.probe.write_attempts.load(Ordering::SeqCst) == 1
            })
            .unwrap_err(),
        SyncError::Storage(StorageError::VaultRetained { path: kept })
    );
}

#[test]
fn withdrawn_sync_writes_nothing() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    assert_eq!(
        fx.engine
            .sync_checked(&local, &pw(), SyncMode::Push, &|| false)
            .unwrap_err(),
        SyncError::Cancelled
    );
    assert_eq!(fx.probe.write_attempts.load(Ordering::SeqCst), 0);
    assert!(!fx.file.exists());
}

#[test]
fn reconciling_push_skips_an_unchanged_vault_but_not_a_changed_one() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    run(&fx.engine, &local, true).unwrap();
    let before = fx.remote_bytes();
    let writes = fx.probe.writes();
    // An unlock with nothing new: no fresh encoding written.
    let r = fx
        .engine
        .sync(&local, &pw(), SyncMode::PushIfChanged)
        .unwrap();
    assert!(matches!(r, SyncResult::Unchanged(None)), "{r:?}");
    assert_eq!(fx.remote_bytes(), before);
    assert_eq!(fx.probe.writes(), writes);
    // New content: written.
    add(&mut local, "B");
    let r = fx
        .engine
        .sync(&local, &pw(), SyncMode::PushIfChanged)
        .unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal(_)), "{r:?}");
    fx.engine.commit(r.pending().unwrap().clone()).unwrap();
    assert_eq!(titles(&fx.remote_db()), ["A", "B"]);
    // A new password (the remote no longer opens with it): written.
    let new_pw = SecretString::from("a brand new password");
    let r = fx
        .engine
        .sync(&local, &new_pw, SyncMode::PushIfChanged)
        .unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal(_)), "{r:?}");
    assert!(Database::open(&fx.remote_bytes(), &new_pw).is_ok());
}

#[test]
fn reconciling_push_writes_a_changed_file_format() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    run(&fx.engine, &local, true).unwrap();
    local.settings.cipher = Cipher::ChaCha20;
    let r = fx
        .engine
        .sync(&local, &pw(), SyncMode::PushIfChanged)
        .unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal(_)), "{r:?}");
    assert_eq!(fx.remote_db().settings.cipher, Cipher::ChaCha20);
}

#[test]
fn withdrawal_after_the_read_stops_before_decrypting_the_remote() {
    let fx = Fx::new();
    // A remote under another password: decrypting it would end in `RemotePasswordMismatch`, so
    // `Cancelled` proves the sync stopped before the KDF ran.
    let other = fast_db("v");
    fx.folder
        .write(&other.save(&SecretString::from("other password")).unwrap())
        .unwrap();
    let before = fx.remote_bytes();
    let asked = AtomicUsize::new(0);
    // Current for the first question (attempt start), withdrawn from then on (a lock).
    let still = || asked.fetch_add(1, Ordering::SeqCst) == 0;
    assert_eq!(
        fx.engine
            .sync_checked(&fast_db("v"), &pw(), SyncMode::Push, &still)
            .unwrap_err(),
        SyncError::Cancelled
    );
    assert_eq!(fx.probe.reads.load(Ordering::SeqCst), 1);
    assert_eq!(fx.probe.write_attempts.load(Ordering::SeqCst), 0);
    assert_eq!(fx.remote_bytes(), before);
}

#[test]
fn checkpoint_after_primary_replacement_never_authorizes_a_blind_push() {
    for mode in [SyncMode::Pull, SyncMode::Push, SyncMode::PushIfChanged] {
        let fx = Fx::new();
        let mut a = fast_db("v");
        add(&mut a, "A");
        let mut ab = a.clone();
        add(&mut ab, "B");
        run(&fx.engine, &ab, true).unwrap();
        let mut ac = a;
        add(&mut ac, "C");
        let result = fx.engine.sync(&ac, &pw(), mode).unwrap();
        assert_eq!(
            titles(merged(result).database()),
            ["A", "B", "C"],
            "mode={mode:?}"
        );
        assert_eq!(titles(&fx.remote_db()), ["A", "B", "C"]);
    }
}

#[test]
fn password_rotation_merges_verified_secondary_content_after_primary_replacement() {
    let fx = Fx::new();
    let a = fast_db("v");
    let mut ab = a.clone();
    add(&mut ab, "B");
    run(&fx.engine, &ab, true).unwrap();
    let mut ac = a;
    add(&mut ac, "C");
    let new_password = SecretString::from("new password");
    let m = merged(
        fx.engine
            .sync(&ac, &new_password, SyncMode::PushIfChanged)
            .unwrap(),
    );
    assert_eq!(titles(m.database()), ["B", "C"]);
    assert_eq!(
        titles(&Database::open(&fx.remote_bytes(), &new_password).unwrap()),
        ["B", "C"]
    );
}

#[test]
fn persisted_checkpoint_cannot_authorize_overwriting_an_unreadable_secondary() {
    let fx = Fx::new();
    let local = fast_db("v");
    run(&fx.engine, &local, true).unwrap();
    let before = fx.remote_bytes();
    let restarted = SyncEngine::new(fx.probe.clone(), fx.store.clone());
    assert_eq!(
        restarted
            .sync(&local, &SecretString::from("new password"), SyncMode::Push)
            .unwrap_err(),
        SyncError::RemotePasswordMismatch
    );
    assert_eq!(fx.remote_bytes(), before);
}

// --- Recorded verified KDF cost (a lowered KDF) ----------------------------------------------

const MIB: u64 = 1024 * 1024;

/// `db` with its Argon2 memory set to `memory` (one pass).
fn with_memory(mut db: Database, memory: u64) -> Database {
    if let KdfParams::Argon2 { memory_bytes, .. } = &mut db.settings.kdf {
        *memory_bytes = memory;
    }
    db
}

/// An automatic ceiling of 8 MiB Argon2 (one pass), so the budget is exercised with fast KDFs.
fn tight_ceiling() -> KdfCost {
    KdfCost::of(&fast_db("ceiling").settings.kdf)
}

fn recorded_memory(fx: &Fx, local: &Database) -> u64 {
    fx.checkpoint()
        .unwrap()
        .verified_kdf_for(&local.root.uuid.to_string())
        .argon2_memory
}

#[test]
fn a_lowered_kdf_still_syncs_the_older_costlier_copy_it_verified() {
    let mut fx = Fx::new();
    fx.engine.ceiling = tight_ceiling();
    let mut local = with_memory(fast_db("v"), 16 * MIB);
    add(&mut local, "A");
    // The costlier vault is uploaded; the cost it was verified at is recorded.
    run(&fx.engine, &local, true).unwrap();
    assert_eq!(recorded_memory(&fx, &local), 16 * MIB);
    let costly = fx.remote_bytes();

    // The user lowers the KDF. Production builds a fresh engine for every sync run, over the
    // same persisted checkpoint.
    let mut lowered = with_memory(local.clone(), 8 * MIB);
    add(&mut lowered, "B");
    let mut engine = SyncEngine::new(fx.probe.clone(), fx.store.clone());
    engine.ceiling = tight_ceiling();
    let r = run(&engine, &lowered, true).unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal(_)), "{r:?}");
    let remote = fx.remote_db();
    assert_eq!(titles(&remote), ["A", "B"]);
    assert_eq!(KdfCost::of(&remote.settings.kdf).argon2_memory, 8 * MIB);
    // The record is kept across the lowered write: never ratcheted down.
    assert_eq!(recorded_memory(&fx, &local), 16 * MIB);

    // Without the record the same costly copy is refused (and left as it was).
    let mut fresh = Fx::new();
    fresh.engine.ceiling = tight_ceiling();
    fs::write(&fresh.file, &costly).unwrap();
    for push in [false, true] {
        assert!(matches!(
            run(&fresh.engine, &lowered, push),
            Err(SyncError::SyncKdfLimitExceeded {
                recovery_file: None
            })
        ));
    }
    assert_eq!(fresh.remote_bytes(), costly);
    assert_eq!(fresh.checkpoint(), None);
}

#[test]
fn an_unverified_costly_file_is_refused_and_never_recorded() {
    let mut fx = Fx::new();
    fx.engine.ceiling = tight_ceiling();
    let mut local = fast_db("v");
    add(&mut local, "A");
    run(&fx.engine, &local, true).unwrap();
    assert_eq!(recorded_memory(&fx, &local), 8 * MIB);

    // A costlier copy of the same vault that does not decrypt with the user's key.
    let hostile = with_memory(fx.remote_db(), 16 * MIB)
        .save(&SecretString::from("someone else"))
        .unwrap();
    fs::write(&fx.file, &hostile).unwrap();
    for push in [false, true] {
        assert!(matches!(
            run(&fx.engine, &local, push),
            Err(SyncError::SyncKdfLimitExceeded {
                recovery_file: None
            })
        ));
    }
    // A bare header asking for more is refused before any key derivation.
    let header = mypass_kdbx::test_util::vault_header_only(
        &with_memory(fast_db("v"), 32 * MIB).settings.kdf,
    );
    fs::write(&fx.file, &header).unwrap();
    assert!(matches!(
        run(&fx.engine, &local, true),
        Err(SyncError::SyncKdfLimitExceeded { .. })
    ));
    assert_eq!(fx.remote_bytes(), header);
    assert_eq!(recorded_memory(&fx, &local), 8 * MIB, "nothing ratcheted");

    // Even inside a raised ceiling, a file that does not decrypt lends nothing.
    fs::write(&fx.file, &hostile).unwrap();
    fx.engine.ceiling = KdfCost::of(&with_memory(fast_db("x"), 16 * MIB).settings.kdf);
    assert_eq!(
        run(&fx.engine, &local, true).unwrap_err(),
        SyncError::RemotePasswordMismatch
    );
    assert_eq!(fx.remote_bytes(), hostile);
    assert_eq!(recorded_memory(&fx, &local), 8 * MIB, "nothing ratcheted");
}

#[test]
fn a_checkpoint_of_another_vault_lends_no_cost() {
    let mut fx = Fx::new();
    fx.engine.ceiling = tight_ceiling();
    let mut other = with_memory(fast_db("other"), 16 * MIB);
    add(&mut other, "X");
    run(&fx.engine, &other, true).unwrap();
    // Same remote, another vault: its record never raises this vault's budget.
    let mut local = fast_db("v");
    add(&mut local, "A");
    assert_eq!(
        fx.checkpoint()
            .unwrap()
            .verified_kdf_for(&local.root.uuid.to_string()),
        KdfCost::default()
    );
    assert!(matches!(
        run(&fx.engine, &local, true),
        Err(SyncError::SyncKdfLimitExceeded { .. })
    ));
}
