use super::*;
use crate::{
    backend::StorageError,
    checkpoint::{InMemoryCheckpointStore, SyncCheckpoint},
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

/// A backend wrapper that counts calls and can act on the n-th read (a concurrent writer).
struct Probe {
    inner: Arc<LocalFolderBackend>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    on_read: Mutex<Box<dyn FnMut(usize) + Send>>,
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
        let n = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        (self.on_read.lock().unwrap())(n);
        self.inner.read()
    }
    fn write(&self, bytes: &[u8]) -> Result<(), StorageError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.write(bytes)
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
            on_read: Mutex::new(Box::new(|_| {})),
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
    fn on_read(&self, f: impl FnMut(usize) + Send + 'static) {
        *self.probe.on_read.lock().unwrap() = Box::new(f);
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

/// The caller's side of a merge: merge into the live vault, then commit the checkpoint.
fn adopt(engine: &SyncEngine, live: &mut Database, m: MergedRemote) {
    let (db, pending) = m.into_parts();
    *live = merge(live, &db);
    engine.commit_merged(pending).unwrap();
}

#[test]
fn first_sync_uploads_initial() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    let r = fx.engine.sync(&local, &pw(), false).unwrap();
    assert!(matches!(r, SyncResult::UploadedInitial), "{r:?}");
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
    fx.engine.sync(&local, &pw(), false).unwrap();
    let before = fx.remote_bytes();
    let writes = fx.probe.writes();
    add(&mut local, "B");
    let r = fx.engine.sync(&local, &pw(), false).unwrap();
    assert!(matches!(r, SyncResult::Unchanged), "{r:?}");
    assert_eq!(fx.remote_bytes(), before);
    assert_eq!(fx.probe.writes(), writes);
}

#[test]
fn unchanged_remote_with_push_uploads_local() {
    let fx = Fx::new();
    let mut local = fast_db("v");
    add(&mut local, "A");
    fx.engine.sync(&local, &pw(), false).unwrap();
    add(&mut local, "B");
    let r = fx.engine.sync(&local, &pw(), true).unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal), "{r:?}");
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
    let m = merged(fx.engine.sync(&local, &pw(), false).unwrap());
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
    let m = merged(fx.engine.sync(&local, &pw(), false).unwrap());
    let (db, pending) = m.into_parts();
    assert!(pending.checkpoint().describes(&fx.remote_bytes()));
    // Not stored before the caller commits.
    assert_eq!(fx.checkpoint(), None);
    local = merge(&local, &db);
    fx.engine.commit_merged(pending).unwrap();
    assert!(fx.checkpoint().unwrap().describes(&fx.remote_bytes()));
    // The anti-ping-pong base: the next sync without changes is not another merge.
    let writes = fx.probe.writes();
    let r = fx.engine.sync(&local, &pw(), false).unwrap();
    assert!(matches!(r, SyncResult::Unchanged), "{r:?}");
    assert_eq!(fx.probe.writes(), writes);
}

#[test]
fn uncommitted_merge_is_merged_again_not_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    fx.engine.sync(&base, &pw(), false).unwrap();
    let mut remote = fx.remote_db();
    add(&mut remote, "B");
    fx.external_write(&remote);
    let mut local = base.clone();
    add(&mut local, "A");
    // The caller fails to adopt the merge and never commits it.
    let _ = merged(fx.engine.sync(&local, &pw(), false).unwrap());
    // A push from the stale local vault must not drop the remote's "B".
    let m = merged(fx.engine.sync(&local, &pw(), true).unwrap());
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
            fx.engine.sync(&local, &pw(), push).unwrap_err(),
            SyncError::RemotePasswordMismatch
        );
    }
    assert_eq!(fx.remote_bytes(), before);
    assert_eq!(fx.probe.writes(), 0);
    assert_eq!(fx.checkpoint(), None);
}

#[test]
fn non_kdbx_remote_is_never_overwritten() {
    let fx = Fx::new();
    fs::write(&fx.file, b"just some notes, not a vault").unwrap();
    let local = fast_db("v");
    assert_eq!(
        fx.engine.sync(&local, &pw(), true).unwrap_err(),
        SyncError::RemoteNotAVault
    );
    assert_eq!(fx.remote_bytes(), b"just some notes, not a vault");
    assert_eq!(fx.probe.writes(), 0);
}

#[test]
fn damaged_remote_is_never_overwritten() {
    let fx = Fx::new();
    let local = fast_db("v");
    fx.engine.sync(&local, &pw(), false).unwrap();
    let mut bytes = fx.remote_bytes();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    fs::write(&fx.file, &bytes).unwrap();
    let err = fx.engine.sync(&local, &pw(), true).unwrap_err();
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
    fx.engine.sync(&base, &pw(), false).unwrap();
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
    let r = fx.engine.sync(&base, &pw(), true).unwrap();
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
    fx.engine.sync(&local, &pw(), false).unwrap();
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
    let _ = merged(fx.engine.sync(&local, &pw(), false).unwrap());
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
        fx.engine.sync(&local, &pw(), true).unwrap_err(),
        SyncError::RemoteIsDifferentVault
    );
    assert_eq!(fx.remote_bytes(), before);
}

#[test]
fn checkpoint_of_another_vault_does_not_allow_a_blind_push() {
    let fx = Fx::new();
    let mut a = fast_db("a");
    add(&mut a, "A");
    fx.engine.sync(&a, &pw(), false).unwrap();
    let before = fx.remote_bytes();
    // Same store and remote, but now the local vault is another one.
    let b = fast_db("b");
    assert_eq!(
        fx.engine.sync(&b, &pw(), true).unwrap_err(),
        SyncError::RemoteIsDifferentVault
    );
    assert_eq!(fx.remote_bytes(), before);
}

#[test]
fn remote_changed_between_read_and_write_is_merged_not_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    fx.engine.sync(&base, &pw(), false).unwrap();
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    let concurrent = theirs.save(&pw()).unwrap();
    let folder = fx.folder.clone();
    let start = fx.probe.reads.load(Ordering::SeqCst);
    // The re-read right before the push sees another device's write.
    fx.on_read(move |n| {
        if n == start + 2 {
            folder.write(&concurrent).unwrap();
        }
    });
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(fx.engine.sync(&local, &pw(), true).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
}

#[test]
fn remote_that_keeps_changing_is_never_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    fx.engine.sync(&base, &pw(), false).unwrap();
    let writes = fx.probe.writes();
    let folder = fx.folder.clone();
    let seed = fx.remote_db();
    // Every read gets a fresh external encoding, so every pre-write check fails.
    fx.on_read(move |_| {
        folder.write(&seed.save(&pw()).unwrap()).unwrap();
    });
    let mut local = base.clone();
    add(&mut local, "Mine");
    assert_eq!(
        fx.engine.sync(&local, &pw(), true).unwrap_err(),
        SyncError::RemoteChangedDuringSync
    );
    assert_eq!(fx.probe.writes(), writes);
}

#[test]
fn remote_appearing_during_initial_upload_is_not_overwritten() {
    let fx = Fx::new();
    let base = fast_db("v");
    let mut theirs = base.clone();
    add(&mut theirs, "Theirs");
    let appearing = theirs.save(&pw()).unwrap();
    let folder = fx.folder.clone();
    fx.on_read(move |n| {
        if n == 2 {
            folder.write(&appearing).unwrap();
        }
    });
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(fx.engine.sync(&local, &pw(), false).unwrap());
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
    let r = fx.engine.sync(&local, &pw(), true).unwrap();
    assert!(matches!(r, SyncResult::Unchanged), "{r:?}");
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
    let r = fx.engine.sync(&local, &pw(), false).unwrap();
    assert!(matches!(r, SyncResult::UploadedLocal), "{r:?}");
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
    let m = merged(fx.engine.sync(&local, &pw(), true).unwrap());
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
    fx.engine.sync(&x, &pw(), true).unwrap();
    let m = merged(y_engine.sync(&y, &pw(), true).unwrap());
    adopt(&y_engine, &mut y, m);
    let m = merged(fx.engine.sync(&x, &pw(), false).unwrap());
    adopt(&fx.engine, &mut x, m);
    assert_eq!(titles(&x), ["X", "Y"]);
    assert_eq!(titles(&y), ["X", "Y"]);
    let writes = fx.probe.writes();
    for _ in 0..3 {
        for (engine, db) in [(&fx.engine, &x), (&y_engine, &y)] {
            let r = engine.sync(db, &pw(), false).unwrap();
            assert!(matches!(r, SyncResult::Unchanged), "{r:?}");
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
        engine.sync(&fast_db("v"), &pw(), true).unwrap_err(),
        SyncError::Storage(StorageError::TooLarge { max_bytes: 16 })
    );
    assert_eq!(fs::read(dir.path().join("vault.kdbx")).unwrap(), [1u8; 32]);
}

#[test]
fn non_regular_remote_is_refused_and_left_alone() {
    let fx = Fx::new();
    fs::create_dir(&fx.file).unwrap();
    assert_eq!(
        fx.engine.sync(&fast_db("v"), &pw(), true).unwrap_err(),
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
        engine.sync(&fast_db("v"), &pw(), true).unwrap_err(),
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
    assert!(matches!(
        engine.sync(&base, &pw(), false).unwrap_err(),
        SyncError::Checkpoint(_)
    ));
    store.failing.store(false, Ordering::SeqCst);
    let mut theirs = fx.remote_db();
    add(&mut theirs, "Theirs");
    fx.external_write(&theirs);
    let mut local = base.clone();
    add(&mut local, "Mine");
    let m = merged(engine.sync(&local, &pw(), true).unwrap());
    assert_eq!(titles(m.database()), ["Mine", "Theirs"]);
    assert_eq!(titles(&fx.remote_db()), ["Mine", "Theirs"]);
}
