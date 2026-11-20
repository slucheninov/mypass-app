//! One-off, guarded migrations. Source copies are never removed. A failure after a durable
//! target write leaves that verified copy available for recovery; rollback never overwrites a
//! concurrent writer. Merge deliberately has no persisted sync checkpoint.
use mypass_kdbx::{Database, KdbxError};
use mypass_sync::{
    InMemoryCheckpointStore, StorageBackend, SyncEngine, SyncError, SyncMode, WriteOutcome,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::UNIX_EPOCH};

#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum MigrationStrategy {
    SeedRemote,
    Merge,
    AdoptRemote,
    OverwriteRemote,
}

#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TargetProbe {
    pub exists: bool,
    /// Unix seconds; f64 at the IPC boundary, as for the other session DTOs.
    pub modified: Option<f64>,
    pub byte_count: Option<f64>,
    pub looks_like_vault: bool,
    pub inside_icloud: bool,
    pub same_as_current: bool,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LocationChangeRequest {
    pub folder_token: String,
    pub strategy: MigrationStrategy,
}

pub struct MigrationOutcome {
    pub strategy: MigrationStrategy,
    pub adopted: Option<Database>,
    pub target_written: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    #[error("vault locked")]
    Locked,
    #[error("target unreadable")]
    TargetUnreadable,
    #[error("target is not a vault")]
    TargetNotAVault,
    #[error("target password mismatch")]
    TargetPasswordMismatch,
    #[error("target write failed or concurrent writer detected")]
    TargetWriteFailed,
    #[error("source invalid")]
    SourceInvalid,
    #[error("target is the current file")]
    SameFile,
    #[error("target validation failed")]
    ValidationFailed,
}

pub struct VaultMigrationService;
impl VaultMigrationService {
    pub fn probe(
        target: &dyn StorageBackend,
        current: &Path,
    ) -> Result<TargetProbe, MigrationError> {
        let bytes = target
            .read()
            .map_err(|_| MigrationError::TargetUnreadable)?;
        let modified = target
            .modification_date()
            .map_err(|_| MigrationError::TargetUnreadable)?
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64());
        Ok(TargetProbe {
            exists: bytes.is_some(),
            modified,
            byte_count: bytes.as_ref().map(|b| b.len() as f64),
            looks_like_vault: bytes
                .as_ref()
                .is_some_and(|b| b.starts_with(&[3, 217, 162, 154, 103, 251, 75, 181])),
            inside_icloud: false, // AppCore knows the user's home and resolves this flag.
            same_as_current: target.vault_path().is_some_and(|p| same_file(p, current)),
        })
    }

    pub fn migrate(
        local: &Database,
        password: &SecretString,
        target: Arc<dyn StorageBackend>,
        strategy: MigrationStrategy,
    ) -> Result<MigrationOutcome, MigrationError> {
        Self::migrate_checked(local, password, target, strategy, &|| true)
    }

    pub fn migrate_checked(
        local: &Database,
        password: &SecretString,
        target: Arc<dyn StorageBackend>,
        strategy: MigrationStrategy,
        current: &dyn Fn() -> bool,
    ) -> Result<MigrationOutcome, MigrationError> {
        let check = || {
            if current() {
                Ok(())
            } else {
                Err(MigrationError::Locked)
            }
        };
        check()?;
        if !unique_ids(&local.root, &mut std::collections::HashSet::new()) {
            return Err(MigrationError::SourceInvalid);
        }
        check()?;
        let before = target
            .read()
            .map_err(|_| MigrationError::TargetUnreadable)?;
        check()?;
        if strategy == MigrationStrategy::AdoptRemote {
            let bytes = before.ok_or(MigrationError::TargetUnreadable)?;
            let remote = open_target(&bytes, password)?;
            return Ok(MigrationOutcome {
                strategy,
                adopted: Some(remote),
                target_written: false,
            });
        }
        let (expected, written) = if strategy == MigrationStrategy::Merge {
            let engine = SyncEngine::new(target.clone(), Arc::new(InMemoryCheckpointStore::new()));
            let result = engine
                .sync_checked(local, password, SyncMode::Push, current)
                .map_err(|e| match e {
                    SyncError::Cancelled => MigrationError::Locked,
                    SyncError::RemotePasswordMismatch => MigrationError::TargetPasswordMismatch,
                    SyncError::RemoteNotAVault => MigrationError::TargetNotAVault,
                    _ => MigrationError::TargetWriteFailed,
                })?;
            let (adopt, _) = result.into_parts();
            (adopt.unwrap_or_else(|| local.clone()), true)
        } else {
            if strategy == MigrationStrategy::SeedRemote && before.is_some() {
                return Err(MigrationError::TargetWriteFailed);
            }
            // Even explicit overwrite refuses corrupt/unreadable/differently encrypted targets.
            if let Some(b) = &before {
                check()?;
                open_target(b, password)?;
            }
            check()?;
            let bytes = local
                .save(password)
                .map_err(|_| MigrationError::SourceInvalid)?;
            check()?;
            let verified =
                Database::open(&bytes, password).map_err(|_| MigrationError::SourceInvalid)?;
            if !crate::saving::equivalent_content(local, &verified) {
                return Err(MigrationError::SourceInvalid);
            }
            let hash: Option<[u8; 32]> = before.as_ref().map(|b| Sha256::digest(b).into());
            check()?;
            match target
                .write_if(hash.as_ref(), &bytes)
                .map_err(|_| MigrationError::TargetWriteFailed)?
            {
                WriteOutcome::Written => {}
                WriteOutcome::Conflict
                | WriteOutcome::Displaced(_)
                | WriteOutcome::DisplacedKept { .. } => {
                    return Err(MigrationError::TargetWriteFailed)
                }
            }
            (local.clone(), true)
        };
        check()?;
        let bytes = target
            .read()
            .map_err(|_| MigrationError::ValidationFailed)?
            .ok_or(MigrationError::ValidationFailed)?;
        check()?;
        let verified =
            Database::open(&bytes, password).map_err(|_| MigrationError::ValidationFailed)?;
        if !crate::saving::equivalent_content(&expected, &verified) {
            return Err(MigrationError::ValidationFailed);
        }
        Ok(MigrationOutcome {
            strategy,
            adopted: Some(verified),
            target_written: written && before.as_ref() != Some(&bytes),
        })
    }
}

fn unique_ids(group: &mypass_kdbx::Group, ids: &mut std::collections::HashSet<uuid::Uuid>) -> bool {
    ids.insert(group.uuid)
        && group.entries.iter().all(|entry| ids.insert(entry.uuid))
        && group.groups.iter().all(|group| unique_ids(group, ids))
}

fn open_target(bytes: &[u8], password: &SecretString) -> Result<Database, MigrationError> {
    Database::open(bytes, password).map_err(|e| match e {
        KdbxError::WrongPassword => MigrationError::TargetPasswordMismatch,
        _ => MigrationError::TargetNotAVault,
    })
}

pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        if a == b {
            return true;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return a.dev() == b.dev() && a.ino() == b.ino();
        }
    }
    false
}

/// A migrated target remains untrusted on every reopen and save conflict check.
/// `VaultFile` uses `read`, so that entry point must also apply the remote-file bound.
pub(crate) struct BoundedIo(pub Arc<dyn mypass_kdbx::FileIo>);
impl mypass_kdbx::FileIo for BoundedIo {
    fn read(&self, path: &Path) -> mypass_kdbx::Result<Vec<u8>> {
        self.0.read_limited(path, mypass_kdbx::MAX_VAULT_FILE_BYTES)
    }
    fn read_limited(&self, path: &Path, max: u64) -> mypass_kdbx::Result<Vec<u8>> {
        self.0
            .read_limited(path, max.min(mypass_kdbx::MAX_VAULT_FILE_BYTES))
    }
    fn open_read(&self, path: &Path) -> mypass_kdbx::Result<mypass_kdbx::OpenedFile> {
        self.0.open_read(path)
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> mypass_kdbx::Result<()> {
        self.0.write_atomic(path, bytes)
    }
    fn write_checked(
        &self,
        path: &Path,
        expected: &[u8; 32],
        bytes: &[u8],
    ) -> mypass_kdbx::Result<mypass_kdbx::SaveOutcome> {
        self.0.write_checked(path, expected, bytes)
    }
    fn metadata(&self, path: &Path) -> mypass_kdbx::Result<mypass_kdbx::FileMeta> {
        self.0.metadata(path)
    }
    fn replace_if(
        &self,
        path: &Path,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
        max: u64,
    ) -> mypass_kdbx::Result<mypass_kdbx::ReplaceOutcome> {
        self.0.replace_if(
            path,
            expected,
            bytes,
            max.min(mypass_kdbx::MAX_VAULT_FILE_BYTES),
        )
    }
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
