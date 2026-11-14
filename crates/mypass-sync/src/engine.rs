//! The sync engine: reconcile the local vault with one remote copy.
//!
//! Data safety rules, in order of priority:
//! - A remote copy is never overwritten unless its current content is either already merged into
//!   what is written, or is exactly the content this device wrote last time (the checkpoint).
//! - A remote that does not open with the session password, is not a vault, is damaged, is too
//!   large, is not a regular file or belongs to another vault is left untouched.
//! - Right before every write the remote is read again; if it changed since this attempt read it,
//!   the attempt starts over, so a concurrent writer is merged, not overwritten.
//! - The engine never mutates `local`. A merge result is written to the remote, but its checkpoint
//!   is stored only when the caller commits it after adopting the merge, so an adoption that fails
//!   costs one more merge, never a blind push over remote-only changes.
use crate::{
    backend::{StorageBackend, StorageError},
    checkpoint::{SyncCheckpoint, SyncCheckpointStore},
    watcher::EchoFilter,
};
use mypass_core::merge::merge;
use mypass_kdbx::{Database, KdbxError};
use secrecy::SecretString;
use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError},
};

/// How often one `sync` re-reads and retries when the remote changes between its read and its
/// write, before giving up with [`SyncError::RemoteChangedDuringSync`] (nothing written).
pub const MAX_SYNC_ATTEMPTS: usize = 3;

/// Why a sync failed. In every case the remote was left as it was, except where a write had
/// already committed (`Storage` after a write, `Checkpoint`), which the next sync reconciles.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    /// The remote storage failed (unavailable folder, timeout, too large, not a regular file...).
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The remote vault does not open with the session password; it is never overwritten.
    #[error("the remote vault does not open with this password")]
    RemotePasswordMismatch,
    /// The remote file is not a KDBX vault; it is never overwritten.
    #[error("the remote file is not a KDBX vault")]
    RemoteNotAVault,
    /// The remote vault uses a format this engine does not read (KDBX 3, an unknown feature).
    #[error("the remote vault is unsupported: {0}")]
    RemoteUnsupported(String),
    /// The remote vault is damaged.
    #[error("the remote vault is damaged: {0}")]
    RemoteCorrupt(String),
    /// The remote file is a different vault (another root group UUID); it is never overwritten
    /// and never merged into the local vault.
    #[error("the remote file is a different vault")]
    RemoteIsDifferentVault,
    /// The remote kept changing between read and write for [`MAX_SYNC_ATTEMPTS`] attempts.
    #[error("the remote file kept changing during the sync")]
    RemoteChangedDuringSync,
    /// The checkpoint store failed. When this follows a write, the next sync merges.
    #[error("sync checkpoint: {0}")]
    Checkpoint(String),
    /// The local or merged vault could not be encoded.
    #[error("could not encode the vault: {0}")]
    Encode(String),
}

/// What a successful sync did.
#[derive(Debug)]
pub enum SyncResult {
    /// The remote was missing; the local vault was written there.
    UploadedInitial,
    /// Nothing to do: the remote is what this device last wrote (and no push was asked for), or
    /// local and remote already hold the same vault content.
    Unchanged,
    /// The local vault was written over a remote that held nothing new for it.
    UploadedLocal,
    /// The remote held changes the local vault lacks. The merge is already on the remote (unless
    /// the remote already held it all). The caller adopts the merged database, then calls
    /// [`SyncEngine::commit_merged`].
    Merged(Box<MergedRemote>),
}

/// A merge result waiting for the caller to adopt it.
#[derive(Debug)]
pub struct MergedRemote {
    database: Database,
    checkpoint: PendingCheckpoint,
}

impl MergedRemote {
    /// The merged database (local plus remote).
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Split into the merged database and the checkpoint to commit after adopting it.
    ///
    /// Adopt by merging into the live database (`mypass_core::merge::merge(&live, &merged)`), not
    /// by replacing it: the live vault may have changed since `sync` took its snapshot, and a
    /// replacement would drop those edits.
    pub fn into_parts(self) -> (Database, PendingCheckpoint) {
        (self.database, self.checkpoint)
    }
}

/// The checkpoint of a merge written to the remote; stored only by [`SyncEngine::commit_merged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCheckpoint(SyncCheckpoint);

impl PendingCheckpoint {
    /// The checkpoint that `commit_merged` will store.
    pub fn checkpoint(&self) -> &SyncCheckpoint {
        &self.0
    }
}

/// Reconciles a local vault with one remote copy. Calls are serialised internally.
pub struct SyncEngine {
    remote: Arc<dyn StorageBackend>,
    checkpoints: Arc<dyn SyncCheckpointStore>,
    echo: Option<Arc<EchoFilter>>,
    gate: Mutex<()>,
}

impl fmt::Debug for SyncEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncEngine").finish_non_exhaustive()
    }
}

fn remote_open_error(e: KdbxError) -> SyncError {
    match e {
        KdbxError::WrongPassword => SyncError::RemotePasswordMismatch,
        KdbxError::NotKdbx => SyncError::RemoteNotAVault,
        e @ KdbxError::UnsupportedVersion { .. } => SyncError::RemoteUnsupported(e.to_string()),
        KdbxError::Unsupported(m) => SyncError::RemoteUnsupported(m),
        KdbxError::Corrupt(m) | KdbxError::InvalidKdfParameters(m) => SyncError::RemoteCorrupt(m),
        e @ (KdbxError::Io { .. } | KdbxError::Random | KdbxError::InvalidOperation(_)) => {
            SyncError::RemoteCorrupt(e.to_string())
        }
    }
}

fn encode(db: &Database, password: &SecretString) -> Result<Vec<u8>, SyncError> {
    db.save(password)
        .map_err(|e| SyncError::Encode(e.to_string()))
}

fn checkpoint_error(e: std::io::Error) -> SyncError {
    SyncError::Checkpoint(format!("{:?}: {e}", e.kind()))
}

/// One attempt's outcome: done, or the remote changed under it and the sync starts over.
enum Attempt {
    Done(SyncResult),
    Raced,
}

impl SyncEngine {
    /// An engine for `remote` with its checkpoint in `checkpoints`.
    pub fn new(remote: Arc<dyn StorageBackend>, checkpoints: Arc<dyn SyncCheckpointStore>) -> Self {
        Self {
            remote,
            checkpoints,
            echo: None,
            gate: Mutex::new(()),
        }
    }

    /// Note every remote write in `echo` just before it happens, so the folder watcher's event
    /// for this device's own upload is recognised and skipped.
    pub fn with_echo_filter(mut self, echo: Arc<EchoFilter>) -> Self {
        self.echo = Some(echo);
        self
    }

    /// Reconcile `local` with the remote copy; `push_local` writes the local vault over a remote
    /// that holds nothing new (after a local save). `local` is never mutated.
    ///
    /// - Remote missing: write `local`, [`SyncResult::UploadedInitial`].
    /// - Remote equals the checkpoint of this vault: [`SyncResult::UploadedLocal`] with
    ///   `push_local`, otherwise [`SyncResult::Unchanged`].
    /// - Otherwise open the remote with `password` and merge. Equal content is
    ///   [`SyncResult::Unchanged`] with no write (so two devices never ping-pong fresh encodings
    ///   of the same vault); a remote with nothing new gets the merge written and is
    ///   [`SyncResult::UploadedLocal`]; else [`SyncResult::Merged`].
    pub fn sync(
        &self,
        local: &Database,
        password: &SecretString,
        push_local: bool,
    ) -> Result<SyncResult, SyncError> {
        let _serial = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let vault = local.root.uuid.to_string();
        for _ in 0..MAX_SYNC_ATTEMPTS {
            if let Attempt::Done(result) = self.attempt(local, password, push_local, &vault)? {
                return Ok(result);
            }
        }
        Err(SyncError::RemoteChangedDuringSync)
    }

    /// Store the checkpoint of a merge after the caller adopted [`MergedRemote::database`] into
    /// its live vault. Until then the old checkpoint stays, and the next sync merges again.
    pub fn commit_merged(&self, pending: PendingCheckpoint) -> Result<(), SyncError> {
        let _serial = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.checkpoints
            .save(Some(&pending.0))
            .map_err(checkpoint_error)
    }

    fn attempt(
        &self,
        local: &Database,
        password: &SecretString,
        push_local: bool,
        vault: &str,
    ) -> Result<Attempt, SyncError> {
        let Some(remote_bytes) = self.remote.read()? else {
            let bytes = encode(local, password)?;
            // A file that appeared since the read is someone else's: start over and merge it.
            if self.remote.read()?.is_some() {
                return Ok(Attempt::Raced);
            }
            self.write_remote(&bytes)?;
            self.store_checkpoint(&bytes, vault)?;
            return Ok(Attempt::Done(SyncResult::UploadedInitial));
        };

        let checkpoint = self.checkpoints.load().map_err(checkpoint_error)?;
        let ours = checkpoint.as_ref().is_some_and(|cp| {
            cp.vault_uuid.as_deref() == Some(vault) && cp.describes(&remote_bytes)
        });
        if ours {
            if !push_local {
                return Ok(Attempt::Done(SyncResult::Unchanged));
            }
            let bytes = encode(local, password)?;
            if !self.remote_still(&remote_bytes)? {
                return Ok(Attempt::Raced);
            }
            self.write_remote(&bytes)?;
            self.store_checkpoint(&bytes, vault)?;
            return Ok(Attempt::Done(SyncResult::UploadedLocal));
        }

        let remote_db = Database::open(&remote_bytes, password).map_err(remote_open_error)?;
        if remote_db.root.uuid != local.root.uuid {
            return Err(SyncError::RemoteIsDifferentVault);
        }
        let merged = merge(local, &remote_db);
        // `Database` equality depends on child order and `merge` sorts by UUID, so each side is
        // compared in its canonical (self-merged) form: equal exactly when it lacks nothing.
        let remote_has_all = merged == merge(&remote_db, &remote_db);
        let local_has_all = merged == merge(local, local);
        if remote_has_all {
            // Nothing to write: the remote already holds every change.
            let cp = self.checkpoint_of(&remote_bytes, vault);
            if local_has_all {
                self.checkpoints.save(Some(&cp)).map_err(checkpoint_error)?;
                return Ok(Attempt::Done(SyncResult::Unchanged));
            }
            return Ok(Attempt::Done(SyncResult::Merged(Box::new(MergedRemote {
                database: merged,
                checkpoint: PendingCheckpoint(cp),
            }))));
        }
        let bytes = encode(&merged, password)?;
        if !self.remote_still(&remote_bytes)? {
            return Ok(Attempt::Raced);
        }
        self.write_remote(&bytes)?;
        if local_has_all {
            // The local vault already held everything: nothing to adopt.
            self.store_checkpoint(&bytes, vault)?;
            return Ok(Attempt::Done(SyncResult::UploadedLocal));
        }
        let cp = self.checkpoint_of(&bytes, vault);
        Ok(Attempt::Done(SyncResult::Merged(Box::new(MergedRemote {
            database: merged,
            checkpoint: PendingCheckpoint(cp),
        }))))
    }

    fn write_remote(&self, bytes: &[u8]) -> Result<(), StorageError> {
        if let Some(echo) = &self.echo {
            echo.note_own_write(SyncCheckpoint::of_bytes(bytes, None));
        }
        self.remote.write(bytes)
    }

    /// Whether the remote still holds exactly `expected` (re-read right before a write).
    fn remote_still(&self, expected: &[u8]) -> Result<bool, StorageError> {
        Ok(self.remote.read()?.as_deref() == Some(expected))
    }

    fn checkpoint_of(&self, bytes: &[u8], vault: &str) -> SyncCheckpoint {
        // The time is informational: a failure to read it never fails the sync.
        let modified = self.remote.modification_date().ok().flatten();
        SyncCheckpoint::of_bytes(bytes, modified).for_vault(vault)
    }

    fn store_checkpoint(&self, written: &[u8], vault: &str) -> Result<(), SyncError> {
        let cp = self.checkpoint_of(written, vault);
        self.checkpoints.save(Some(&cp)).map_err(checkpoint_error)
    }
}

#[cfg(test)]
mod tests;
