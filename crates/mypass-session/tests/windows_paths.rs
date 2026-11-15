//! Real temporary-file Windows cases. UNC is opt-in; native identity/contention is gated.
#![cfg(windows)]
// Test-only fixtures, matching the journey integration suite.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use mypass_kdbx::{Database, FileIo, PlainFileIo, SaveOutcome, VaultFile};
use mypass_platform::{testing::fake_platform, AppPaths};
use mypass_session::{
    backups::BackupManager, settings::SettingsStore, storage::StorageStatus,
    testing::fast_database, AppCore,
};
use secrecy::SecretString;
use std::{fs, path::Path};

fn pw() -> SecretString {
    SecretString::from("synthetic-windows-path-test")
}

fn round_trip(dir: &Path, backups: &Path) {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join("vault.kdbx");
    let mut db = fast_database("Windows paths");
    fs::write(&path, db.save(&pw()).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let (mut file, _) = VaultFile::open(&path, &pw()).unwrap();
    let manager = BackupManager::new(backups.into(), SettingsStore::in_memory());
    let id = db.root_id().to_string();
    manager.snapshot_before_save(&id, &path, 1_700_000_000, true);
    let items = manager.list(&id);
    assert_eq!(items.len(), 1);
    assert_eq!(manager.read(&id, &items[0].id).unwrap(), before);
    db.meta.database_name = "Saved on Windows".into();
    assert_eq!(file.save(&db, &pw()).unwrap(), SaveOutcome::Saved);
    assert_eq!(
        VaultFile::open(&path, &pw()).unwrap().1.meta.database_name,
        "Saved on Windows"
    );
}

#[test]
fn long_path_over_260_chars_saves_and_reopens() {
    let tmp = tempfile::tempdir().unwrap();
    let mut dir = fs::canonicalize(tmp.path()).unwrap();
    while dir.as_os_str().len() < 310 {
        dir.push("x".repeat(40));
    }
    round_trip(&dir, &tmp.path().join("data/backups"));
}

#[test]
fn non_ascii_directory_saves_and_reopens() {
    let tmp = tempfile::tempdir().unwrap();
    round_trip(
        &tmp.path().join("Паролі 🔑"),
        &tmp.path().join("data/backups"),
    );
}

#[test]
fn file_stamp_survives_ntfs_mtime_granularity() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("vault.kdbx");
    fs::write(&path, fast_database("Stamp").save(&pw()).unwrap()).unwrap();
    let (mut file, db) = VaultFile::open(&path, &pw()).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut competitor = fs::read(&path).unwrap();
    competitor[0] ^= 1;
    fs::write(&path, &competitor).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    assert!(file.has_changed_on_disk().unwrap());
    assert_eq!(file.save(&db, &pw()).unwrap(), SaveOutcome::ChangedOnDisk);
    assert_eq!(fs::read(&path).unwrap(), competitor);
}

#[test]
fn bounded_read_refuses_directory_and_oversized_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("large");
    fs::write(&path, b"oversized").unwrap();
    assert!(PlainFileIo.read_limited(&path, 2).is_err());
    assert!(PlainFileIo.read_limited(tmp.path(), 100).is_err());
}

#[test]
fn windows_app_uses_local_storage_for_a_macos_cloud_lookalike() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::windows_data_root(tmp.path());
    let core = AppCore::new(paths.clone(), fake_platform().platform).unwrap();
    let dir = mypass_sync::icloud::ICloudDriveLocation::root(paths.home());
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault.kdbx");
    core.session
        .create_vault_with(&path, pw(), fast_database("Local"))
        .unwrap();
    core.session.flush().unwrap();
    assert_eq!(core.session.storage_status(), StorageStatus::default());
    assert!(!paths
        .data_dir
        .join(mypass_sync::icloud::ICLOUD_CACHE_DIR)
        .exists());
    assert!(Database::open(&fs::read(path).unwrap(), &pw()).is_ok());
}

#[test]
#[ignore = "requires explicitly supplied UNC share and Windows integration opt-in"]
fn windows_integration_unc_path_vault_opens_when_available() {
    assert_eq!(
        std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
        Ok("1")
    );
    let Some(share) = std::env::var_os("MYPASS_WINDOWS_UNC_TEST") else {
        eprintln!("UNC test NOT RUN: MYPASS_WINDOWS_UNC_TEST absent");
        return;
    };
    assert!(share.to_string_lossy().starts_with(r"\\"));
    let tmp = tempfile::tempdir_in(share).unwrap();
    round_trip(tmp.path(), &tmp.path().join("backups"));
}

#[test]
fn reserved_paths_are_rejected_at_local_folder_boundaries() {
    use mypass_sync::{LocalFolderBackend, StorageError};
    let tmp = tempfile::tempdir().unwrap();
    for name in ["CON", "NUL", "file.kdbx.", "file.kdbx ", "file:stream"] {
        assert!(matches!(
            LocalFolderBackend::new(tmp.path(), name),
            Err(StorageError::InvalidPath(_))
        ));
        assert!(matches!(
            LocalFolderBackend::new(tmp.path().join(name), "vault.kdbx"),
            Err(StorageError::InvalidPath(_))
        ));
    }
}
