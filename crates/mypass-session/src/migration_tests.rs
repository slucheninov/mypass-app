use super::*;
use crate::testing::fast_database;
use mypass_kdbx::{test_util::FailingFileIo, PlainFileIo};
use mypass_sync::FileIoBackend;
use std::{fs, sync::Arc};

#[test]
fn probe_reports_empty_target_without_decrypting() {
    let d = tempfile::tempdir().unwrap();
    let target = FileIoBackend::new(Arc::new(PlainFileIo), d.path().join("v.kdbx"));
    let p = VaultMigrationService::probe(&target, &d.path().join("old.kdbx")).unwrap();
    assert!(!p.exists);
    assert!(!p.looks_like_vault);
}

#[test]
fn probe_flags_non_vault_bytes() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("v.kdbx");
    fs::write(&path, b"not a vault").unwrap();
    let target = FileIoBackend::new(Arc::new(PlainFileIo), path);
    assert!(
        !VaultMigrationService::probe(&target, &d.path().join("old"))
            .unwrap()
            .looks_like_vault
    );
}

#[test]
fn seed_remote_uploads_and_leaves_local_untouched() {
    let d = tempfile::tempdir().unwrap();
    let db = fast_database("migration");
    let pw = SecretString::from("pw");
    let source = db.save(&pw).unwrap();
    let target = Arc::new(FileIoBackend::new(
        Arc::new(PlainFileIo),
        d.path().join("v.kdbx"),
    ));
    VaultMigrationService::migrate(&db, &pw, target.clone(), MigrationStrategy::SeedRemote)
        .unwrap();
    assert_eq!(
        Database::open(&target.read().unwrap().unwrap(), &pw)
            .unwrap()
            .root_id(),
        db.root_id()
    );
    assert_eq!(
        Database::open(&source, &pw).unwrap().root_id(),
        db.root_id()
    );
    assert!(
        VaultMigrationService::migrate(&db, &pw, target, MigrationStrategy::SeedRemote).is_err()
    );
}

#[test]
fn adopt_remote_wrong_password_leaves_both_sides_untouched() {
    let d = tempfile::tempdir().unwrap();
    let db = fast_database("migration");
    let path = d.path().join("v.kdbx");
    let before = db.save(&SecretString::from("other")).unwrap();
    fs::write(&path, &before).unwrap();
    let target = Arc::new(FileIoBackend::new(Arc::new(PlainFileIo), &path));
    let e = VaultMigrationService::migrate(
        &db,
        &SecretString::from("pw"),
        target,
        MigrationStrategy::AdoptRemote,
    )
    .err()
    .unwrap();
    assert_eq!(e, MigrationError::TargetPasswordMismatch);
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn adopt_remote_non_vault_leaves_both_sides_untouched() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("v.kdbx");
    fs::write(&path, b"not a vault").unwrap();
    let target = Arc::new(FileIoBackend::new(Arc::new(PlainFileIo), &path));
    assert_eq!(
        VaultMigrationService::migrate(
            &fast_database("migration"),
            &SecretString::from("pw"),
            target,
            MigrationStrategy::AdoptRemote
        )
        .err(),
        Some(MigrationError::TargetNotAVault)
    );
    assert_eq!(fs::read(path).unwrap(), b"not a vault");
}

#[test]
fn write_failure_preserves_existing_target() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("v.kdbx");
    let db = fast_database("migration");
    let pw = SecretString::from("pw");
    let before = db.save(&pw).unwrap();
    fs::write(&path, &before).unwrap();
    let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
    io.fail_write_atomic(std::io::ErrorKind::Other);
    let target = Arc::new(FileIoBackend::new(io, &path));
    assert!(
        VaultMigrationService::migrate(&db, &pw, target, MigrationStrategy::OverwriteRemote)
            .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

fn session_fixture() -> (tempfile::TempDir, crate::session::VaultSession) {
    let d = tempfile::tempdir().unwrap();
    let session = crate::session::VaultSession::new(crate::session::SessionDeps {
        settings: crate::settings::SettingsStore::in_memory(),
        scheduler: crate::scheduler::ManualScheduler::new(std::time::UNIX_EPOCH),
        backups_root: d.path().join("backups"),
        file_io: Arc::new(PlainFileIo),
    });
    let path = d.path().join("old.kdbx");
    fs::write(
        &path,
        fast_database("migration")
            .save(&SecretString::from("pw"))
            .unwrap(),
    )
    .unwrap();
    session.open_vault(&path, SecretString::from("pw")).unwrap();
    (d, session)
}

#[test]
fn relocate_keeps_session_unlocked_and_saves_to_new_path() {
    let (d, s) = session_fixture();
    let target = d.path().join("new.kdbx");
    fs::copy(d.path().join("old.kdbx"), &target).unwrap();
    s.switch_to(&target, Arc::new(PlainFileIo), None).unwrap();
    assert!(s.is_unlocked());
    s.mutate(|db, _| {
        db.meta.database_name = "changed".into();
        Ok(())
    })
    .unwrap();
    s.flush().unwrap();
    assert_eq!(
        Database::open(&fs::read(&target).unwrap(), &SecretString::from("pw"))
            .unwrap()
            .meta
            .database_name,
        "changed"
    );
    assert_ne!(
        fs::read(&target).unwrap(),
        fs::read(d.path().join("old.kdbx")).unwrap()
    );
}

#[test]
fn failed_validation_keeps_session_on_old_path() {
    let (d, s) = session_fixture();
    let old = s.vault_path();
    let target = d.path().join("new.kdbx");
    fs::write(&target, b"truncated").unwrap();
    assert!(s.switch_to(&target, Arc::new(PlainFileIo), None).is_err());
    assert_eq!(s.vault_path(), old);
    assert!(s.is_unlocked());
}

#[test]
fn adopt_remote_replaces_working_copy_after_validation() {
    let (d, s) = session_fixture();
    let remote = fast_database("remote");
    let target = d.path().join("new.kdbx");
    fs::write(&target, remote.save(&SecretString::from("pw")).unwrap()).unwrap();
    s.switch_to(&target, Arc::new(PlainFileIo), Some(remote))
        .unwrap();
    assert_eq!(
        s.with_db(|db| db.meta.database_name.clone()).unwrap(),
        "remote"
    );
}

#[test]
fn change_while_locked_is_refused() {
    let (d, s) = session_fixture();
    s.lock(crate::session::LockReason::Manual);
    assert!(s
        .switch_to(&d.path().join("new.kdbx"), Arc::new(PlainFileIo), None)
        .is_err());
}

#[test]
fn merge_keeps_entries_from_both_sides_and_updates_both() {
    let d = tempfile::tempdir().unwrap();
    let mut local = fast_database("migration");
    let mut remote = local.clone();
    local
        .root
        .entries
        .push(mypass_kdbx::Entry::new(mypass_kdbx::Timestamp::now()));
    remote
        .root
        .entries
        .push(mypass_kdbx::Entry::new(mypass_kdbx::Timestamp::now()));
    let pw = SecretString::from("pw");
    let path = d.path().join("v.kdbx");
    fs::write(&path, remote.save(&pw).unwrap()).unwrap();
    let target = Arc::new(FileIoBackend::new(Arc::new(PlainFileIo), &path));
    let result =
        VaultMigrationService::migrate(&local, &pw, target, MigrationStrategy::Merge).unwrap();
    assert_eq!(result.adopted.unwrap().root.entries.len(), 2);
    assert_eq!(
        Database::open(&fs::read(path).unwrap(), &pw)
            .unwrap()
            .root
            .entries
            .len(),
        2
    );
}

fn core_fixture() -> (tempfile::TempDir, crate::AppCore) {
    let d = tempfile::tempdir().unwrap();
    let core = crate::AppCore::with_parts(
        mypass_platform::AppPaths::macos(d.path()),
        mypass_platform::testing::fake_platform().platform,
        crate::scheduler::ManualScheduler::new(std::time::UNIX_EPOCH),
        crate::settings::SettingsStore::in_memory(),
    );
    let path = d.path().join("old.kdbx");
    fs::write(
        &path,
        fast_database("migration")
            .save(&SecretString::from("pw"))
            .unwrap(),
    )
    .unwrap();
    core.vault_open(path.to_str().unwrap(), SecretString::from("pw"))
        .unwrap();
    (d, core)
}

#[test]
fn old_copy_moves_to_backups_not_deleted() {
    let (d, core) = core_fixture();
    let folder = d.path().join("target");
    fs::create_dir(&folder).unwrap();
    let before = fs::read(d.path().join("old.kdbx")).unwrap();
    core.vault_location_change(&folder, MigrationStrategy::SeedRemote)
        .unwrap();
    assert_eq!(
        core.session.vault_path().unwrap(),
        fs::canonicalize(folder.join("old.kdbx")).unwrap()
    );
    assert_eq!(fs::read(d.path().join("old.kdbx")).unwrap(), before);
    let backups = core
        .backups
        .folder(&core.session.vault_id().unwrap())
        .unwrap();
    assert!(fs::read_dir(backups)
        .unwrap()
        .any(|p| fs::read(p.unwrap().path()).ok().as_ref() == Some(&before)));
}

#[test]
fn connect_secondary_with_existing_vault_offers_same_strategies() {
    let (d, core) = core_fixture();
    let folder = d.path().join("secondary");
    fs::create_dir(&folder).unwrap();
    let remote = core.session.with_db(Clone::clone).unwrap();
    fs::write(
        folder.join("old.kdbx"),
        remote.save(&SecretString::from("pw")).unwrap(),
    )
    .unwrap();
    let old = core.session.vault_path();
    core.sync_connect_secondary(&folder, MigrationStrategy::Merge)
        .unwrap();
    assert_eq!(core.session.vault_path(), old);
    assert_eq!(
        core.settings.get().secondary_folder.as_deref(),
        fs::canonicalize(&folder).unwrap().to_str()
    );
    core.secondary.shutdown();
}

#[cfg(unix)]
#[test]
fn move_to_same_file_is_refused() {
    let (d, core) = core_fixture();
    let folder = d.path().join("alias");
    std::os::unix::fs::symlink(d.path(), &folder).unwrap();
    assert!(core.vault_location_probe(&folder).unwrap().same_as_current);
    assert!(core
        .vault_location_change(&folder, MigrationStrategy::OverwriteRemote)
        .is_err());
}

/// A committed write followed by a truncated verification read: no source mutation is allowed.
struct TruncateVerification {
    inner: FileIoBackend,
    wrote: std::sync::atomic::AtomicBool,
}
impl mypass_sync::StorageBackend for TruncateVerification {
    fn exists(&self) -> Result<bool, mypass_sync::StorageError> {
        self.inner.exists()
    }
    fn modification_date(
        &self,
    ) -> Result<Option<std::time::SystemTime>, mypass_sync::StorageError> {
        self.inner.modification_date()
    }
    fn read(&self) -> Result<Option<Vec<u8>>, mypass_sync::StorageError> {
        if self.wrote.load(std::sync::atomic::Ordering::SeqCst) {
            Ok(Some(vec![0]))
        } else {
            self.inner.read()
        }
    }
    fn write(&self, _: &[u8]) -> Result<(), mypass_sync::StorageError> {
        panic!("unguarded write")
    }
    fn write_if(
        &self,
        hash: Option<&[u8; 32]>,
        bytes: &[u8],
    ) -> Result<WriteOutcome, mypass_sync::StorageError> {
        let r = self.inner.write_if(hash, bytes)?;
        self.wrote.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(r)
    }
}

#[test]
fn failed_verification_preserves_source_and_keeps_written_target_recoverable() {
    let d = tempfile::tempdir().unwrap();
    let db = fast_database("migration");
    let target = d.path().join("v.kdbx");
    let pw = SecretString::from("pw");
    let backend = Arc::new(TruncateVerification {
        inner: FileIoBackend::new(Arc::new(PlainFileIo), &target),
        wrote: false.into(),
    });
    assert_eq!(
        VaultMigrationService::migrate(&db, &pw, backend, MigrationStrategy::SeedRemote).err(),
        Some(MigrationError::ValidationFailed)
    );
    assert_eq!(
        Database::open(&fs::read(target).unwrap(), &pw)
            .unwrap()
            .root_id(),
        db.root_id()
    );
}

#[test]
fn overwrite_remote_writes_validated_bytes_only() {
    let d = tempfile::tempdir().unwrap();
    let mut db = fast_database("migration");
    let path = d.path().join("v.kdbx");
    let pw = SecretString::from("pw");
    let before = db.save(&pw).unwrap();
    fs::write(&path, &before).unwrap();
    // Invalid model: duplicate UUIDs cannot be encoded.
    let entry = mypass_kdbx::Entry::new(mypass_kdbx::Timestamp::now());
    db.root.entries.push(entry.clone());
    db.root.entries.push(entry);
    let backend = Arc::new(FileIoBackend::new(Arc::new(PlainFileIo), &path));
    assert_eq!(
        VaultMigrationService::migrate(&db, &pw, backend, MigrationStrategy::OverwriteRemote).err(),
        Some(MigrationError::SourceInvalid)
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn secondary_connect_merge_does_not_use_persisted_checkpoint() {
    let (d, core) = core_fixture();
    let folder = d.path().join("secondary");
    fs::create_dir(&folder).unwrap();
    let mut remote = core.session.with_db(Clone::clone).unwrap();
    remote
        .root
        .entries
        .push(mypass_kdbx::Entry::new(mypass_kdbx::Timestamp::now()));
    fs::write(
        folder.join("old.kdbx"),
        remote.save(&SecretString::from("pw")).unwrap(),
    )
    .unwrap();
    core.sync_connect_secondary(&folder, MigrationStrategy::Merge)
        .unwrap();
    assert_eq!(core.session.with_db(|db| db.root.entries.len()).unwrap(), 1);
    assert_eq!(
        Database::open(
            &fs::read(folder.join("old.kdbx")).unwrap(),
            &SecretString::from("pw")
        )
        .unwrap()
        .root
        .entries
        .len(),
        1
    );
    core.secondary.shutdown();
}

#[test]
fn move_to_icloud_then_back_to_local_round_trip_preserves_all_entries() {
    let (d, core) = core_fixture();
    let cloud = mypass_sync::icloud::ICloudDriveLocation::root(d.path());
    fs::create_dir_all(&cloud).unwrap();
    core.session
        .set_io_resolver(Some(crate::storage::icloud_resolver(
            Arc::new(mypass_platform::testing::FakeCoordinatedFiles::new()),
            d.path().to_path_buf(),
            d.path().join("cache"),
        )));
    core.vault_location_change(&cloud, MigrationStrategy::SeedRemote)
        .unwrap();
    assert_eq!(
        core.session.storage_status().location,
        mypass_sync::icloud::VaultLocation::ICloudDrive
    );
    core.session
        .mutate(|db, _| {
            db.root
                .entries
                .push(mypass_kdbx::Entry::new(mypass_kdbx::Timestamp::now()));
            Ok(())
        })
        .unwrap();
    let local = d.path().join("local");
    fs::create_dir(&local).unwrap();
    core.vault_location_change(&local, MigrationStrategy::SeedRemote)
        .unwrap();
    assert_eq!(
        core.session.storage_status().location,
        mypass_sync::icloud::VaultLocation::Local
    );
    assert_eq!(core.session.with_db(|db| db.root.entries.len()).unwrap(), 1);
}

struct BoundedOnly;
impl mypass_kdbx::FileIo for BoundedOnly {
    fn read(&self, _: &std::path::Path) -> mypass_kdbx::Result<Vec<u8>> {
        panic!("migration used an unbounded target read")
    }
    fn read_limited(&self, p: &std::path::Path, max: u64) -> mypass_kdbx::Result<Vec<u8>> {
        mypass_kdbx::FileIo::read_limited(&PlainFileIo, p, max)
    }
    fn write_atomic(&self, p: &std::path::Path, b: &[u8]) -> mypass_kdbx::Result<()> {
        mypass_kdbx::FileIo::write_atomic(&PlainFileIo, p, b)
    }
    fn metadata(&self, p: &std::path::Path) -> mypass_kdbx::Result<mypass_kdbx::FileMeta> {
        mypass_kdbx::FileIo::metadata(&PlainFileIo, p)
    }
}

#[test]
fn switch_verification_uses_bounded_reads() {
    let (d, s) = session_fixture();
    let target = d.path().join("target.kdbx");
    fs::copy(d.path().join("old.kdbx"), &target).unwrap();
    s.switch_to(&target, Arc::new(BoundedOnly), None).unwrap();
}

#[test]
fn edits_flushed_during_migration_still_fence_the_switch() {
    let (d, s) = session_fixture();
    let snapshot = s.sync_snapshot().unwrap();
    let target = d.path().join("target.kdbx");
    fs::copy(d.path().join("old.kdbx"), &target).unwrap();
    s.mutate(|db, _| {
        db.meta.database_name = "new edit".into();
        Ok(())
    })
    .unwrap();
    s.flush().unwrap();
    assert!(s
        .switch_snapshot(
            &target,
            Arc::new(PlainFileIo),
            Some(snapshot.db.clone()),
            &snapshot
        )
        .is_err());
    assert_eq!(
        s.vault_path().unwrap(),
        fs::canonicalize(d.path().join("old.kdbx")).unwrap()
    );
    assert_eq!(
        s.with_db(|db| db.meta.database_name.clone()).unwrap(),
        "new edit"
    );
}

#[test]
fn lock_during_migration_fences_the_switch() {
    let (d, s) = session_fixture();
    let snapshot = s.sync_snapshot().unwrap();
    let target = d.path().join("target.kdbx");
    fs::copy(d.path().join("old.kdbx"), &target).unwrap();
    s.lock(crate::session::LockReason::Manual);
    assert!(s
        .switch_snapshot(&target, Arc::new(PlainFileIo), None, &snapshot)
        .is_err());
    assert!(!s.is_unlocked());
    assert_eq!(
        s.vault_path().unwrap(),
        fs::canonicalize(d.path().join("old.kdbx")).unwrap()
    );
}

#[test]
fn secondary_adopt_of_another_vault_is_refused_without_changing_the_primary() {
    let (d, core) = core_fixture();
    let folder = d.path().join("secondary");
    fs::create_dir(&folder).unwrap();
    fs::write(
        folder.join("old.kdbx"),
        fast_database("different vault")
            .save(&SecretString::from("pw"))
            .unwrap(),
    )
    .unwrap();
    let before = fs::read(d.path().join("old.kdbx")).unwrap();
    assert!(core
        .sync_connect_secondary(&folder, MigrationStrategy::AdoptRemote)
        .is_err());
    assert_eq!(fs::read(d.path().join("old.kdbx")).unwrap(), before);
    assert!(core.settings.get().secondary_folder.is_none());
}

#[test]
fn unchanged_merge_reports_no_target_write() {
    let d = tempfile::tempdir().unwrap();
    let db = fast_database("migration");
    let pw = SecretString::from("pw");
    let path = d.path().join("v.kdbx");
    fs::write(&path, db.save(&pw).unwrap()).unwrap();
    let target = Arc::new(FileIoBackend::new(Arc::new(PlainFileIo), &path));
    let outcome =
        VaultMigrationService::migrate(&db, &pw, target, MigrationStrategy::Merge).unwrap();
    assert!(!outcome.target_written);
}
