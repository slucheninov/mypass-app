//! Per-vault backup snapshots taken before the vault file is overwritten.
//!
//! Rules ported from the old app's `VaultBackupManager`:
//! - a backup is a byte copy of the on-disk vault BEFORE it is overwritten (still encrypted with
//!   the vault's own key; nothing is decrypted here);
//! - the first write of a session always snapshots (the vault may have changed outside the app);
//!   afterwards the configured interval must have elapsed since the newest backup, and a newest
//!   backup dated in the future (clock skew) counts as due;
//! - the count is read from the settings at the moment of the save, so a settings change applies
//!   to the very next save; after writing, backups beyond the count are deleted oldest first, and
//!   the snapshot just written is never deleted: it is named `max(now, newest + 1)` so it sorts
//!   newest even when older backups are dated in the future (the clock went backwards);
//! - a missing source file (the first save of a new vault) writes nothing and records nothing;
//! - a backup failure NEVER blocks the save: it is recorded (`last_failure`, kept in memory and
//!   in `<folder>/last-failure.json` so Settings can show it after a restart) and cleared by the
//!   next successful snapshot.
//!
//! Layout: `<backups_dir>/<vault_id>/backup-<unix_secs>.kdbx`, with a `-<n>` suffix when two
//! snapshots fall in the same second. One folder per vault identity; `vault_id` is validated as a
//! `mypass_platform::VaultId` before it touches a path. The old app's flat shared folder is not
//! migrated: the new app is a fresh install whose old sandbox container it cannot read, so there
//! is nothing to migrate.
//!
//! DTO instants are `f64` Unix milliseconds and sizes `f64` bytes: the TypeScript bindings
//! cannot carry 64-bit integers. The rule functions take `i64` Unix seconds.

use crate::scheduler::lock;
use crate::session::SessionError;
use crate::settings::{BackupInterval, SettingsStore};
use mypass_platform::VaultId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One backup file.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupItem {
    /// The file name, e.g. `backup-1700000000.kdbx`.
    pub id: String,
    /// Unix milliseconds, from the file name.
    pub created: f64,
    /// Bytes.
    pub size: f64,
}

/// The last snapshot failure of a vault.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupFailure {
    pub message: String,
    /// Unix milliseconds.
    pub at: f64,
}

/// What the backups pane shows.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    /// Newest first.
    pub items: Vec<BackupItem>,
    pub last_failure: Option<BackupFailure>,
    pub count: u32,
    pub interval: BackupInterval,
}

/// Highest `-<n>` suffix tried for snapshots within the same second.
pub(crate) const MAX_SAME_SECOND: u32 = 99;
const FAILURE_FILE: &str = "last-failure.json";

/// Parses `backup-<secs>[-<n>].kdbx` into `(secs, n)`; anything else (including any path
/// separator or `..`) is `None`.
fn parse_backup_name(name: &str) -> Option<(i64, u32)> {
    let rest = name.strip_prefix("backup-")?.strip_suffix(".kdbx")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (secs, n) = match rest.split_once('-') {
        Some((secs, n)) => (secs, Some(n)),
        None => (rest, None),
    };
    if !digits(secs) || !n.is_none_or(digits) {
        return None;
    }
    let n = match n {
        Some(n) => n.parse().ok()?,
        None => 0,
    };
    Some((secs.parse().ok()?, n))
}

fn backup_name(secs: i64, n: u32) -> String {
    if n == 0 {
        format!("backup-{secs}.kdbx")
    } else {
        format!("backup-{secs}-{n}.kdbx")
    }
}

fn millis(secs: i64) -> f64 {
    secs as f64 * 1000.0
}

/// Takes, lists and reads backups (cheap to clone; clones share the in-memory failure record).
#[derive(Clone)]
pub struct BackupManager {
    root: PathBuf,
    settings: SettingsStore,
    failures: Arc<Mutex<HashMap<String, BackupFailure>>>,
}

impl std::fmt::Debug for BackupManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupManager")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl BackupManager {
    /// `root` is `AppPaths.backups_dir`.
    pub fn new(root: PathBuf, settings: SettingsStore) -> Self {
        Self {
            root,
            settings,
            failures: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The pure due rule (all times Unix seconds).
    pub fn is_snapshot_due(
        interval: BackupInterval,
        newest: Option<i64>,
        now: i64,
        first_write_of_session: bool,
    ) -> bool {
        if first_write_of_session || interval == BackupInterval::EverySave {
            return true;
        }
        let Some(newest) = newest else {
            return true;
        };
        // A negative gap (newest backup in the future: clock skew) fails the conversion: due.
        u64::try_from(now.saturating_sub(newest)).map_or(true, |gap| gap >= interval.seconds())
    }

    /// `<root>/<vault_id>`, the same path as `AppPaths::vault_backups_dir`. A `vault_id` that is
    /// not a plain path segment is `InvalidInput`.
    pub fn folder(&self, vault_id: &str) -> Result<PathBuf, SessionError> {
        let id = VaultId::new(vault_id).map_err(|e| SessionError::InvalidInput(e.to_string()))?;
        Ok(self.root.join(id.as_str()))
    }

    /// Creates the vault's folder when missing and returns it (backs `app_reveal_backups`).
    pub fn ensure_folder(&self, vault_id: &str) -> Result<PathBuf, SessionError> {
        let folder = self.folder(vault_id)?;
        fs::create_dir_all(&folder).map_err(|e| SessionError::Io(e.to_string()))?;
        Ok(folder)
    }

    /// Copies `vault_file` into the vault's folder when a snapshot is due. Never returns an
    /// error and never panics: a failure is recorded and the save goes ahead.
    pub fn snapshot_before_save(
        &self,
        vault_id: &str,
        vault_file: &Path,
        now: i64,
        first_write_of_session: bool,
    ) {
        let Ok(folder) = self.folder(vault_id) else {
            // Not a valid id: there is no folder to record a failure in, and no snapshot.
            return;
        };
        match self.try_snapshot(&folder, vault_file, now, first_write_of_session) {
            Ok(true) => self.clear_failure(vault_id),
            Ok(false) => {}
            Err(message) => self.record_failure(vault_id, &folder, message, now),
        }
    }

    /// `Ok(true)` when a backup was written, `Ok(false)` when none was due or the source is
    /// missing.
    fn try_snapshot(
        &self,
        folder: &Path,
        vault_file: &Path,
        now: i64,
        first_write_of_session: bool,
    ) -> Result<bool, String> {
        let settings = self.settings.get();
        let now = now.max(0);
        let newest = self.list_folder(folder).first().map(|(secs, _, _)| *secs);
        if !Self::is_snapshot_due(
            settings.backup_interval,
            newest,
            now,
            first_write_of_session,
        ) {
            return Ok(false);
        }
        let bytes = match fs::read(vault_file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("reading the vault failed: {e}")),
        };
        fs::create_dir_all(folder).map_err(|e| format!("creating the folder failed: {e}"))?;
        // Rotation sorts by the time in the file name, so the fresh snapshot must sort newest even
        // when existing backups are dated in the future (the clock went backwards): it is then
        // named one second after the newest one. Equal seconds are told apart by the `-<n>` suffix.
        let secs = match newest {
            Some(newest) if newest > now => newest.saturating_add(1),
            _ => now,
        };
        let (target_name, target) = (0..=MAX_SAME_SECOND)
            .map(|n| {
                let name = backup_name(secs, n);
                let path = folder.join(&name);
                (name, path)
            })
            .find(|(_, p)| {
                matches!(fs::symlink_metadata(p), Err(e) if e.kind() == ErrorKind::NotFound)
            })
            .ok_or_else(|| "no free backup file name".to_owned())?;
        mypass_kdbx::write_atomic(&target, &bytes)
            .map_err(|e| format!("writing the backup failed: {e}"))?;
        // Rotation is best effort: an undeletable old backup must not mark the snapshot failed.
        // The snapshot just written is never a rotation victim, whatever the names say.
        let keep = usize::try_from(settings.backup_count).unwrap_or(usize::MAX);
        let older = self
            .list_folder(folder)
            .into_iter()
            .filter(|(_, _, item)| item.id != target_name);
        for (_, _, item) in older.skip(keep.saturating_sub(1)) {
            let _ = fs::remove_file(folder.join(item.id));
        }
        Ok(true)
    }

    fn record_failure(&self, vault_id: &str, folder: &Path, message: String, now: i64) {
        let failure = BackupFailure {
            message,
            at: millis(now),
        };
        // Persisting is best effort: the in-memory record already serves this run.
        if let Ok(text) = serde_json::to_vec(&failure) {
            if fs::create_dir_all(folder).is_ok() {
                let _ = mypass_kdbx::write_atomic(&folder.join(FAILURE_FILE), &text);
            }
        }
        lock(&self.failures).insert(vault_id.to_owned(), failure);
    }

    /// Forgets the last failure (memory and file).
    pub fn clear_failure(&self, vault_id: &str) {
        lock(&self.failures).remove(vault_id);
        if let Ok(folder) = self.folder(vault_id) {
            let _ = fs::remove_file(folder.join(FAILURE_FILE));
        }
    }

    fn last_failure(&self, vault_id: &str) -> Option<BackupFailure> {
        if let Some(f) = lock(&self.failures).get(vault_id) {
            return Some(f.clone());
        }
        let folder = self.folder(vault_id).ok()?;
        let bytes = fs::read(folder.join(FAILURE_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Backup files of `folder`, newest first, with their sort key.
    fn list_folder(&self, folder: &Path) -> Vec<(i64, u32, BackupItem)> {
        let Ok(entries) = fs::read_dir(folder) else {
            return Vec::new();
        };
        let mut items: Vec<_> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let (secs, n) = parse_backup_name(&name)?;
                let meta = entry.metadata().ok().filter(|m| m.is_file())?;
                Some((
                    secs,
                    n,
                    BackupItem {
                        id: name,
                        created: millis(secs),
                        size: meta.len() as f64,
                    },
                ))
            })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse((item.0, item.1)));
        items
    }

    /// Backups of a vault, newest first (empty for an invalid id or a missing folder).
    pub fn list(&self, vault_id: &str) -> Vec<BackupItem> {
        match self.folder(vault_id) {
            Ok(folder) => self
                .list_folder(&folder)
                .into_iter()
                .map(|(_, _, item)| item)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Items, last failure and the current count and interval settings.
    pub fn status(&self, vault_id: &str) -> BackupStatus {
        let settings = self.settings.get();
        BackupStatus {
            items: self.list(vault_id),
            last_failure: self.last_failure(vault_id),
            count: settings.backup_count,
            interval: settings.backup_interval,
        }
    }

    /// The bytes of backup `id`. Only names of the form `backup-<secs>[-<n>].kdbx` are accepted,
    /// so no path separator or `..` can reach the file system.
    pub fn read(&self, vault_id: &str, id: &str) -> Result<Vec<u8>, SessionError> {
        if parse_backup_name(id).is_none() {
            return Err(SessionError::InvalidInput("not a backup id".into()));
        }
        let path = self.folder(vault_id)?.join(id);
        fs::read(path).map_err(|e| match e.kind() {
            ErrorKind::NotFound => SessionError::NotFound,
            _ => SessionError::Io(e.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsPatch;
    use mypass_platform::{AppPaths, VaultId};
    use std::fs;
    use tempfile::TempDir;

    const VID: &str = "0123456789abcdef0123456789abcdef";

    struct Fx {
        dir: TempDir,
        settings: SettingsStore,
        backups: BackupManager,
        vault: PathBuf,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let settings = SettingsStore::in_memory();
        let backups = BackupManager::new(dir.path().join("backups"), settings.clone());
        let vault = dir.path().join("v.kdbx");
        fs::write(&vault, b"vault-v1").unwrap();
        Fx {
            dir,
            settings,
            backups,
            vault,
        }
    }

    impl Fx {
        fn count(&self, n: u32) {
            self.settings
                .update(SettingsPatch {
                    backup_count: Some(n),
                    ..SettingsPatch::default()
                })
                .unwrap();
        }
        fn ids(&self) -> Vec<String> {
            self.backups.list(VID).into_iter().map(|i| i.id).collect()
        }
    }

    fn is_backup_name(id: &str) -> bool {
        let Some(rest) = id
            .strip_prefix("backup-")
            .and_then(|r| r.strip_suffix(".kdbx"))
        else {
            return false;
        };
        let mut parts = rest.split('-');
        let secs = parts.next().unwrap_or("");
        let counter = parts.next();
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        digits(secs) && counter.is_none_or(digits) && parts.next().is_none()
    }

    #[test]
    fn due_rule_table() {
        use BackupInterval::*;
        let due = BackupManager::is_snapshot_due;
        assert!(due(EverySave, Some(100), 100, false));
        assert!(due(EverySave, None, 0, false));
        assert!(!due(TenMinutes, Some(1000), 1599, false));
        assert!(due(TenMinutes, Some(1000), 1600, false));
        assert!(!due(Hourly, Some(0), 3599, false));
        assert!(due(Hourly, Some(0), 3600, false));
        assert!(!due(Daily, Some(0), 86_399, false));
        assert!(due(Daily, Some(0), 86_400, false));
        for interval in [EverySave, TenMinutes, Hourly, Daily] {
            assert!(
                due(interval, Some(100), 100, true),
                "first write {interval:?}"
            );
            assert!(due(interval, None, 100, false), "no newest {interval:?}");
            assert!(
                due(interval, Some(5000), 100, false),
                "future newest {interval:?}"
            );
        }
    }

    #[test]
    fn snapshot_copies_existing_file_bytes() {
        let f = fx();
        f.backups
            .snapshot_before_save(VID, &f.vault, 1_700_000_000, true);
        let items = f.backups.list(VID);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert!(is_backup_name(&item.id), "{}", item.id);
        assert_eq!(item.id, "backup-1700000000.kdbx");
        assert_eq!(item.created, 1_700_000_000_000.0);
        assert_eq!(item.size, 8.0);
        assert_eq!(f.backups.read(VID, &item.id).unwrap(), b"vault-v1");
        assert_eq!(f.backups.status(VID).last_failure, None);
    }

    #[test]
    fn interval_skips_snapshot_until_due() {
        let f = fx();
        f.settings
            .update(SettingsPatch {
                backup_interval: Some(BackupInterval::TenMinutes),
                ..SettingsPatch::default()
            })
            .unwrap();
        f.backups.snapshot_before_save(VID, &f.vault, 1000, false);
        f.backups.snapshot_before_save(VID, &f.vault, 1599, false);
        assert_eq!(f.ids(), vec!["backup-1000.kdbx"]);
        f.backups.snapshot_before_save(VID, &f.vault, 1600, false);
        assert_eq!(f.ids().len(), 2);
    }

    #[test]
    fn rotation_keeps_newest_n() {
        let f = fx();
        f.count(3);
        for now in 1..=6 {
            f.backups.snapshot_before_save(VID, &f.vault, now, false);
        }
        assert_eq!(
            f.ids(),
            vec!["backup-6.kdbx", "backup-5.kdbx", "backup-4.kdbx"]
        );
    }

    #[test]
    fn rotation_never_deletes_the_fresh_snapshot_when_backups_are_future_dated() {
        let f = fx();
        f.count(3);
        let folder = f.backups.ensure_folder(VID).unwrap();
        for secs in [5000, 5001, 5002] {
            fs::write(folder.join(backup_name(secs, 0)), b"old").unwrap();
        }
        fs::write(&f.vault, b"vault-fresh").unwrap();
        f.backups.snapshot_before_save(VID, &f.vault, 100, false);
        let ids = f.ids();
        assert_eq!(ids.len(), 3, "{ids:?}");
        assert!(!ids.contains(&"backup-5000.kdbx".to_owned()), "{ids:?}");
        assert!(ids.contains(&"backup-5001.kdbx".to_owned()), "{ids:?}");
        assert!(ids.contains(&"backup-5002.kdbx".to_owned()), "{ids:?}");
        let fresh = ids.first().unwrap();
        assert_eq!(f.backups.read(VID, fresh).unwrap(), b"vault-fresh");
        assert_eq!(f.backups.status(VID).last_failure, None);
    }

    #[test]
    fn count_change_applies_to_next_save() {
        let f = fx();
        f.count(10);
        for now in 1..=5 {
            f.backups.snapshot_before_save(VID, &f.vault, now, false);
        }
        assert_eq!(f.ids().len(), 5);
        f.count(2);
        assert_eq!(f.ids().len(), 5);
        f.backups.snapshot_before_save(VID, &f.vault, 6, false);
        assert_eq!(f.ids(), vec!["backup-6.kdbx", "backup-5.kdbx"]);
        let status = f.backups.status(VID);
        assert_eq!(status.count, 2);
        assert_eq!(status.interval, BackupInterval::EverySave);
    }

    #[test]
    fn missing_source_records_nothing() {
        let f = fx();
        f.backups
            .snapshot_before_save(VID, &f.dir.path().join("new.kdbx"), 10, true);
        assert!(f.ids().is_empty());
        assert_eq!(f.backups.status(VID).last_failure, None);
    }

    #[test]
    fn unwritable_backup_dir_records_failure_and_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root-is-a-file");
        fs::write(&root, b"x").unwrap();
        let vault = dir.path().join("v.kdbx");
        fs::write(&vault, b"vault").unwrap();
        let backups = BackupManager::new(root, SettingsStore::in_memory());
        backups.snapshot_before_save(VID, &vault, 42, true);
        let failure = backups.status(VID).last_failure.expect("failure recorded");
        assert_eq!(failure.at, 42_000.0);
        assert!(!failure.message.is_empty());
        assert!(backups.list(VID).is_empty());
        backups.clear_failure(VID);
        assert_eq!(backups.status(VID).last_failure, None);
    }

    #[test]
    fn success_clears_previous_failure() {
        let f = fx();
        let folder = f.backups.folder(VID).unwrap();
        // A regular file where the vault folder belongs makes the next snapshot fail.
        fs::create_dir_all(folder.parent().unwrap()).unwrap();
        fs::write(&folder, b"blocker").unwrap();
        f.backups.snapshot_before_save(VID, &f.vault, 1, true);
        assert!(f.backups.status(VID).last_failure.is_some());
        fs::remove_file(&folder).unwrap();
        f.backups.snapshot_before_save(VID, &f.vault, 2, false);
        assert_eq!(f.backups.status(VID).last_failure, None);
        assert_eq!(f.ids(), vec!["backup-2.kdbx"]);
    }

    #[test]
    fn failure_survives_a_restart() {
        let f = fx();
        f.backups.snapshot_before_save(VID, &f.vault, 1, true);
        // The folder exists now; a directory squatting on the next name makes the copy fail.
        let folder = f.backups.folder(VID).unwrap();
        for name in std::iter::once("backup-5.kdbx".to_owned())
            .chain((1..=MAX_SAME_SECOND).map(|n| format!("backup-5-{n}.kdbx")))
        {
            fs::create_dir(folder.join(name)).unwrap();
        }
        f.backups.snapshot_before_save(VID, &f.vault, 5, false);
        let failure = f.backups.status(VID).last_failure.expect("recorded");
        let restarted = BackupManager::new(f.dir.path().join("backups"), f.settings.clone());
        assert_eq!(restarted.status(VID).last_failure, Some(failure));
        restarted.clear_failure(VID);
        let again = BackupManager::new(f.dir.path().join("backups"), f.settings.clone());
        assert_eq!(again.status(VID).last_failure, None);
    }

    #[test]
    fn separate_folders_per_vault_id() {
        let f = fx();
        let other = "fedcba9876543210fedcba9876543210";
        f.backups.snapshot_before_save(VID, &f.vault, 1, true);
        f.backups.snapshot_before_save(other, &f.vault, 2, true);
        assert_eq!(f.ids(), vec!["backup-1.kdbx"]);
        let others: Vec<_> = f.backups.list(other).into_iter().map(|i| i.id).collect();
        assert_eq!(others, vec!["backup-2.kdbx"]);
        assert_ne!(
            f.backups.folder(VID).unwrap(),
            f.backups.folder(other).unwrap()
        );
    }

    #[test]
    fn list_is_newest_first() {
        let f = fx();
        // Written out of order by hand: a snapshot is always named newest (see the rotation test).
        let folder = f.backups.ensure_folder(VID).unwrap();
        for name in [
            "backup-30.kdbx",
            "backup-10.kdbx",
            "backup-20.kdbx",
            "backup-20-1.kdbx",
        ] {
            fs::write(folder.join(name), b"x").unwrap();
        }
        fs::write(folder.join("notes.txt"), b"ignored").unwrap();
        assert_eq!(
            f.ids(),
            vec![
                "backup-30.kdbx",
                "backup-20-1.kdbx",
                "backup-20.kdbx",
                "backup-10.kdbx"
            ]
        );
        assert_eq!(f.backups.status(VID).items.len(), 4);
    }

    #[test]
    fn read_rejects_path_traversal() {
        let f = fx();
        f.backups.snapshot_before_save(VID, &f.vault, 1, true);
        for bad in [
            "../x",
            "a/b",
            "a\\b",
            "..",
            "",
            "last-failure.json",
            "/etc/passwd",
        ] {
            assert!(
                matches!(f.backups.read(VID, bad), Err(SessionError::InvalidInput(_))),
                "{bad}"
            );
        }
        assert_eq!(
            f.backups.read(VID, "backup-99.kdbx"),
            Err(SessionError::NotFound)
        );
        assert!(matches!(
            f.backups.read("../evil", "backup-1.kdbx"),
            Err(SessionError::InvalidInput(_))
        ));
    }

    #[test]
    fn ensure_folder_creates_the_vault_folder_and_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::macos(dir.path());
        let backups = BackupManager::new(paths.backups_dir.clone(), SettingsStore::in_memory());
        let folder = backups.ensure_folder(VID).unwrap();
        assert_eq!(folder, paths.vault_backups_dir(&VaultId::new(VID).unwrap()));
        assert!(folder.is_dir());
        assert_eq!(backups.ensure_folder(VID).unwrap(), folder);
    }

    #[test]
    fn folder_for_traversal_like_vault_id_is_rejected() {
        let f = fx();
        for bad in ["..", "../x", "a/b", "", "CON", "/abs"] {
            assert!(
                matches!(f.backups.folder(bad), Err(SessionError::InvalidInput(_))),
                "{bad}"
            );
            assert!(f.backups.ensure_folder(bad).is_err());
            assert!(f.backups.list(bad).is_empty());
            f.backups.snapshot_before_save(bad, &f.vault, 1, true);
        }
        assert!(!f.dir.path().join("x").exists());
    }

    #[test]
    fn two_snapshots_in_the_same_second_do_not_overwrite() {
        let f = fx();
        f.backups.snapshot_before_save(VID, &f.vault, 7, true);
        fs::write(&f.vault, b"vault-v2").unwrap();
        f.backups.snapshot_before_save(VID, &f.vault, 7, false);
        let ids = f.ids();
        assert_eq!(ids, vec!["backup-7-1.kdbx", "backup-7.kdbx"]);
        assert!(ids.iter().all(|id| is_backup_name(id)));
        assert_eq!(f.backups.read(VID, "backup-7.kdbx").unwrap(), b"vault-v1");
        assert_eq!(f.backups.read(VID, "backup-7-1.kdbx").unwrap(), b"vault-v2");
    }

    #[test]
    fn status_serialises_camel_case_without_64_bit_integers() {
        let f = fx();
        f.backups.snapshot_before_save(VID, &f.vault, 3, true);
        let v = serde_json::to_value(f.backups.status(VID)).unwrap();
        assert_eq!(v["items"][0]["id"], "backup-3.kdbx");
        assert_eq!(v["lastFailure"], serde_json::Value::Null);
        assert_eq!(v["interval"], "everySave");
        assert_eq!(v["count"], 10);
    }
}
