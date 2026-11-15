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

#[test]
fn migration_accepts_unsorted_children_for_every_strategy() {
    let mut db = fast_database("unsorted");
    for id in [2, 1] {
        let mut entry = mypass_kdbx::Entry::new(mypass_kdbx::Timestamp(1));
        entry.uuid = uuid::Uuid::from_u128(id);
        db.root.entries.push(entry);
        let mut group = mypass_kdbx::Group::new("group", mypass_kdbx::Timestamp(1));
        group.uuid = uuid::Uuid::from_u128(id + 10);
        db.root.groups.push(group);
    }
    let pw = SecretString::from("pw");
    for strategy in [
        MigrationStrategy::SeedRemote,
        MigrationStrategy::Merge,
        MigrationStrategy::AdoptRemote,
        MigrationStrategy::OverwriteRemote,
    ] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("vault.kdbx");
        if strategy != MigrationStrategy::SeedRemote {
            fs::write(&path, db.save(&pw).unwrap()).unwrap();
        }
        let target = Arc::new(FileIoBackend::new(Arc::new(PlainFileIo), &path));
        let result = VaultMigrationService::migrate(&db, &pw, target, strategy).unwrap();
        assert!(crate::saving::same_content(
            &mypass_core::merge::merge(&db, &db),
            &result.adopted.unwrap()
        ));
    }
}

#[test]
fn withdrawn_migration_never_writes_after_target_read() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Withdraw {
        inner: FileIoBackend,
        current: Arc<AtomicBool>,
    }
    impl StorageBackend for Withdraw {
        fn exists(&self) -> Result<bool, mypass_sync::StorageError> {
            self.inner.exists()
        }
        fn write(&self, _: &[u8]) -> Result<(), mypass_sync::StorageError> {
            panic!("unguarded write")
        }

        fn read(&self) -> Result<Option<Vec<u8>>, mypass_sync::StorageError> {
            let result = self.inner.read();
            self.current.store(false, Ordering::SeqCst);
            result
        }
        fn write_if(
            &self,
            _: Option<&[u8; 32]>,
            _: &[u8],
        ) -> Result<WriteOutcome, mypass_sync::StorageError> {
            panic!("withdrawn migration wrote target")
        }
        fn modification_date(
            &self,
        ) -> Result<Option<std::time::SystemTime>, mypass_sync::StorageError> {
            self.inner.modification_date()
        }
    }
    for strategy in [
        MigrationStrategy::SeedRemote,
        MigrationStrategy::Merge,
        MigrationStrategy::OverwriteRemote,
        MigrationStrategy::AdoptRemote,
    ] {
        let d = tempfile::tempdir().unwrap();
        let current = Arc::new(AtomicBool::new(true));
        let backend = Arc::new(Withdraw {
            inner: FileIoBackend::new(Arc::new(PlainFileIo), d.path().join("v.kdbx")),
            current: current.clone(),
        });
        assert!(VaultMigrationService::migrate_checked(
            &fast_database("x"),
            &SecretString::from("pw"),
            backend,
            strategy,
            &|| current.load(Ordering::SeqCst)
        )
        .is_err());
        assert!(!d.path().join("v.kdbx").exists());
    }
}

#[test]
fn suspended_secondary_cannot_undo_adopt_and_resumes_after_failure() {
    let (d, core) = core_fixture();
    let a = d.path().join("a");
    let b = d.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    core.sync_connect_secondary(&a, MigrationStrategy::SeedRemote)
        .unwrap();
    let empty = core.session.with_db(Clone::clone).unwrap();
    fs::write(
        b.join("old.kdbx"),
        empty.save(&SecretString::from("pw")).unwrap(),
    )
    .unwrap();
    let mut remote = empty.clone();
    remote
        .root
        .entries
        .push(mypass_kdbx::Entry::new(mypass_kdbx::Timestamp(1)));
    fs::write(
        a.join("old.kdbx"),
        remote.save(&SecretString::from("pw")).unwrap(),
    )
    .unwrap();
    let worker = core.secondary.clone();
    let once = std::sync::atomic::AtomicBool::new(false);
    core.session.add_observer(Arc::new(move |event| {
        if matches!(event, crate::session::SessionEvent::VaultChanged)
            && !once.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let _ = worker.sync_now();
        }
    }));
    core.sync_connect_secondary(&b, MigrationStrategy::AdoptRemote)
        .unwrap();
    assert_eq!(core.session.with_db(|db| db.root.entries.len()).unwrap(), 0);
    assert!(core
        .sync_connect_secondary(&a, MigrationStrategy::SeedRemote)
        .is_err());
    assert_eq!(
        core.settings.get().secondary_folder,
        Some(fs::canonicalize(&b).unwrap().to_string_lossy().into_owned())
    );
    assert!(core.sync_now().is_ok());
}

#[test]
fn recovery_reservation_rejects_checkpoint_publication_until_install() {
    let (_d, session) = session_fixture();
    let incarnation = session.unlocked_incarnation().unwrap();
    let reservation = {
        let mut inner = crate::scheduler::lock(&session.0.inner);
        crate::saving::InFlight::recovery(&session, &mut inner)
    };
    assert_eq!(
        session.publish_if_current(
            incarnation,
            || true,
            || panic!("stale publication admitted")
        ),
        None::<()>
    );
    drop(reservation.finish_unpublished());
    assert_eq!(
        session.publish_if_current(incarnation, || true, || 7),
        Some(7)
    );
}

#[test]
fn app_migration_withdraws_on_lock_quit_and_vault_switch_during_read() {
    struct WithdrawIo {
        session: crate::session::VaultSession,
        action: u8,
        next: std::path::PathBuf,
    }
    impl mypass_kdbx::FileIo for WithdrawIo {
        fn read(&self, _: &Path) -> mypass_kdbx::Result<Vec<u8>> {
            panic!("unbounded")
        }
        fn read_limited(&self, p: &Path, max: u64) -> mypass_kdbx::Result<Vec<u8>> {
            match self.action {
                0 => {
                    self.session.lock(crate::session::LockReason::Manual);
                }
                1 => {
                    self.session.begin_quit();
                }
                _ => {
                    self.session
                        .open_vault(&self.next, SecretString::from("pw"))
                        .unwrap();
                }
            }
            mypass_kdbx::FileIo::read_limited(&PlainFileIo, p, max)
        }
        fn write_atomic(&self, _: &Path, _: &[u8]) -> mypass_kdbx::Result<()> {
            panic!("write after withdrawal")
        }
        fn replace_if(
            &self,
            _: &Path,
            _: Option<&[u8; 32]>,
            _: &[u8],
            _: u64,
        ) -> mypass_kdbx::Result<mypass_kdbx::ReplaceOutcome> {
            panic!("guarded write after withdrawal")
        }

        fn metadata(&self, p: &Path) -> mypass_kdbx::Result<mypass_kdbx::FileMeta> {
            mypass_kdbx::FileIo::metadata(&PlainFileIo, p)
        }
    }
    for action in 0..3 {
        for strategy in [
            MigrationStrategy::SeedRemote,
            MigrationStrategy::Merge,
            MigrationStrategy::OverwriteRemote,
        ] {
            let (d, core) = core_fixture();
            let folder = d.path().join("target");
            fs::create_dir(&folder).unwrap();
            let next = d.path().join("next.kdbx");
            fs::write(
                &next,
                fast_database("next")
                    .save(&SecretString::from("pw"))
                    .unwrap(),
            )
            .unwrap();
            let target = folder.join("old.kdbx");
            let selected = fs::canonicalize(&folder).unwrap().join("old.kdbx");
            let io = Arc::new(WithdrawIo {
                session: core.session.clone(),
                action,
                next,
            });
            core.session.set_io_resolver(Some(Arc::new(move |path| {
                if path == selected {
                    Some(crate::ResolvedStorage::plain(io.clone()))
                } else {
                    None
                }
            })));
            assert!(core.vault_location_change(&folder, strategy).is_err());
            assert!(!target.exists());
        }
    }
}

#[test]
fn queued_folder_migration_cannot_apply_to_another_incarnation() {
    let (d, core) = core_fixture();
    let incarnation = core.session.unlocked_incarnation().unwrap();
    let next = d.path().join("next.kdbx");
    fs::write(
        &next,
        fast_database("next")
            .save(&SecretString::from("pw"))
            .unwrap(),
    )
    .unwrap();
    core.session
        .open_vault(&next, SecretString::from("pw"))
        .unwrap();
    let folder = d.path().join("target");
    fs::create_dir(&folder).unwrap();
    assert!(core
        .migrate_folder_for(&folder, MigrationStrategy::SeedRemote, true, incarnation)
        .is_err());
    assert!(!folder.join("next.kdbx").exists());
}

#[test]
fn unsorted_vaults_migrate_through_primary_and_secondary_for_all_strategies() {
    for relocate in [true, false] {
        for strategy in [
            MigrationStrategy::SeedRemote,
            MigrationStrategy::Merge,
            MigrationStrategy::AdoptRemote,
            MigrationStrategy::OverwriteRemote,
        ] {
            let (d, core) = core_fixture();
            core.session
                .mutate(|db, _| {
                    for id in [2, 1] {
                        let mut e = mypass_kdbx::Entry::new(mypass_kdbx::Timestamp(1));
                        e.uuid = uuid::Uuid::from_u128(id);
                        db.root.entries.push(e);
                        let mut g = mypass_kdbx::Group::new("g", mypass_kdbx::Timestamp(1));
                        g.uuid = uuid::Uuid::from_u128(id + 10);
                        db.root.groups.push(g);
                    }
                    Ok(())
                })
                .unwrap();
            core.session.flush().unwrap();
            let target = d.path().join("target");
            fs::create_dir(&target).unwrap();
            if strategy != MigrationStrategy::SeedRemote {
                let bytes = core
                    .session
                    .with_db(|db| db.save(&SecretString::from("pw")).unwrap())
                    .unwrap();
                fs::write(target.join("old.kdbx"), bytes).unwrap();
            }
            if relocate {
                core.vault_location_change(&target, strategy).unwrap();
            } else {
                core.sync_connect_secondary(&target, strategy).unwrap();
            }
            core.session
                .with_db(|db| {
                    assert_eq!(db.root.entries.len(), 2);
                    assert_eq!(db.root.groups.len(), 2);
                })
                .unwrap();
            let remote = Database::open(
                &fs::read(target.join("old.kdbx")).unwrap(),
                &SecretString::from("pw"),
            )
            .unwrap();
            assert_eq!(remote.root.entries.len(), 2);
            assert_eq!(remote.root.groups.len(), 2);
            assert!(d.path().join("old.kdbx").exists());
        }
    }
}

#[test]
fn migrated_icloud_saves_preserve_a_writer_at_coordinator_entry() {
    for reopen in [false, true] {
        let (d, core) = core_fixture();
        let fake = Arc::new(mypass_platform::testing::FakeCoordinatedFiles::new());
        let cloud = mypass_sync::icloud::ICloudDriveLocation::root(d.path());
        fs::create_dir_all(&cloud).unwrap();
        core.session
            .set_io_resolver(Some(crate::storage::icloud_resolver(
                fake.clone(),
                d.path().to_path_buf(),
                d.path().join("cache"),
            )));
        core.vault_location_change(&cloud, MigrationStrategy::SeedRemote)
            .unwrap();
        if reopen {
            core.session.lock(crate::session::LockReason::Manual);
            core.session.unlock(SecretString::from("pw")).unwrap();
        }
        let mut remote = core.session.with_db(Clone::clone).unwrap();
        let mut entry = mypass_kdbx::Entry::new(mypass_kdbx::Timestamp(100));
        entry.set_field("Title", "remote-only", false);
        remote.root.entries.push(entry);
        core.session
            .mutate(|db, _| {
                let mut entry = mypass_kdbx::Entry::new(mypass_kdbx::Timestamp(101));
                entry.set_field("Title", "local-only", false);
                db.root.entries.push(entry);
                Ok(())
            })
            .unwrap();
        let target = core.session.vault_path().unwrap();
        let path = target.clone();
        let bytes = remote.save(&SecretString::from("pw")).unwrap();
        fake.before_next_write(move || fs::write(&path, &bytes).unwrap());
        core.session.flush().unwrap();
        let saved = Database::open(&fs::read(target).unwrap(), &SecretString::from("pw")).unwrap();
        let mut titles: Vec<_> = saved
            .root
            .entries
            .iter()
            .filter_map(|e| e.field("Title"))
            .collect();
        titles.sort();
        assert_eq!(titles, ["local-only", "remote-only"], "reopen={reopen}");
        assert!(crate::saving::equivalent_content(
            &saved,
            &core.session.with_db(Clone::clone).unwrap()
        ));
    }
}

#[test]
fn migrated_icloud_pending_retry_preserves_a_competing_writer() {
    use mypass_platform::testing::CoordinatedOp;
    let (d, core) = core_fixture();
    let fake = Arc::new(mypass_platform::testing::FakeCoordinatedFiles::new());
    let cloud = mypass_sync::icloud::ICloudDriveLocation::root(d.path());
    fs::create_dir_all(&cloud).unwrap();
    core.session
        .set_io_resolver(Some(crate::storage::icloud_resolver(
            fake.clone(),
            d.path().to_path_buf(),
            d.path().join("cache"),
        )));
    core.vault_location_change(&cloud, MigrationStrategy::SeedRemote)
        .unwrap();
    let target = core.session.vault_path().unwrap();
    let mut remote = core.session.with_db(Clone::clone).unwrap();
    remote
        .root
        .entries
        .push(mypass_kdbx::Entry::new(mypass_kdbx::Timestamp(100)));
    core.session
        .mutate(|db, _| {
            db.root
                .entries
                .push(mypass_kdbx::Entry::new(mypass_kdbx::Timestamp(101)));
            Ok(())
        })
        .unwrap();
    fake.fail_next(CoordinatedOp::Write, std::io::ErrorKind::PermissionDenied);
    core.session.lock(crate::session::LockReason::Manual);
    assert!(!core.session.is_unlocked());
    let bytes = remote.save(&SecretString::from("pw")).unwrap();
    let path = target.clone();
    let competing = bytes.clone();
    fake.before_next_write(move || fs::write(&path, &competing).unwrap());
    assert!(
        core.session.flush().is_err(),
        "pending retry must refuse the racing write"
    );
    assert_eq!(fs::read(&target).unwrap(), bytes);
    core.session.unlock(SecretString::from("pw")).unwrap();
    core.session.flush().unwrap();
    let saved = Database::open(&fs::read(target).unwrap(), &SecretString::from("pw")).unwrap();
    assert_eq!(saved.root.entries.len(), 2);
}
