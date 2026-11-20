//! The sync engine: reconcile the local vault with one remote copy.
//!
//! Data safety rules, in order of priority:
//! - A remote copy is never overwritten unless its current content is either already merged into
//!   what is written. A checkpoint alone never proves what a later primary still contains.
//! - A remote must decrypt with the session password or match an exact copy this active engine
//!   already verified (password rotation). Unreadable, damaged, oversized, non-regular or
//!   different-vault content is left untouched.
//! - Every write is conditional ([`StorageBackend::write_if`]): it replaces the remote only while
//!   the remote still holds the exact content this attempt read (or is still missing), checked
//!   by the storage right before the replacement. A changed remote makes the attempt start over
//!   and merge it. A version another writer slipped in after that final check is handed back by
//!   the storage ([`WriteOutcome::Displaced`]) and merged into the next attempt instead of being
//!   lost; after a successful write the remote is read back, and a writer that replaced it in
//!   the meantime is merged too, never described by our checkpoint.
//! - A checkpoint records the secondary ciphertext, not the provenance of a later primary.
//!   Even on a checkpoint hit the engine merges the verified remote content: an external reload
//!   or a restart may have replaced the primary with a delayed branch. Checkpoints are published
//!   only after local durability, but never authorize a blind push.
//! - The engine never mutates `local`.
//! - A remote is opened within the automatic KDF budget: per dimension the larger of the default
//!   ceiling, the local vault's cost and the highest cost already verified for this remote
//!   (recorded in the checkpoint). Only a cost that decrypted with the user's key is recorded,
//!   so a vault whose KDF was lowered still syncs with the older, costlier copy, while an
//!   unverified costly file stays refused before any KDF work.
use crate::{
    backend::{StorageBackend, StorageError, WriteOutcome},
    checkpoint::{SyncCheckpoint, SyncCheckpointStore},
    watcher::EchoFilter,
};
use mypass_core::merge::merge;
use mypass_kdbx::{Database, KdbxError, KdfCost, KdfLimits, KdfParams, SaveSettings};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError},
};

/// How often one `sync` starts over when the remote changes under it, before giving up with
/// [`SyncError::RemoteChangedDuringSync`].
pub const MAX_SYNC_ATTEMPTS: usize = 3;

/// Why a sync failed. In every case the remote was left as it was, except where a write had
/// already committed (`Storage` after a write, `RemoteChangedDuringSync` after a write that was
/// replaced again, a displaced copy kept for recovery, or `Cancelled` after a write). A subsequent sync
/// reconciles the remote content.
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
    /// The remote vault uses an unsupported format or feature.
    #[error("the remote vault is unsupported: {0}")]
    RemoteUnsupported(String),
    /// A copy exceeds the automatic-open budget. An already displaced copy is kept.
    #[error("vault KDF cost above the sync limit")]
    SyncKdfLimitExceeded {
        /// The retained working copy's path, when one exists.
        recovery_file: Option<std::path::PathBuf>,
    },
    /// The remote vault is damaged.
    #[error("the remote vault is damaged: {0}")]
    RemoteCorrupt(String),
    /// The remote file is a different vault (another root group UUID); it is never overwritten
    /// and never merged into the local vault.
    #[error("the remote file is a different vault")]
    RemoteIsDifferentVault,
    /// The remote kept changing during the sync for [`MAX_SYNC_ATTEMPTS`] attempts.
    #[error("the remote file kept changing during the sync")]
    RemoteChangedDuringSync,
    /// Another writer's version was displaced by our write in the last instant and cannot be
    /// merged (it does not open with this password or is another vault). It was kept next to the
    /// remote file by the storage.
    #[error("a concurrent version of the remote was kept but cannot be merged")]
    ConcurrentCopyKept,
    /// The caller withdrew the sync (the vault was locked, switched or the app is quitting)
    /// and the engine stopped subsequent work when it noticed. A write may already have
    /// committed; cancellation does not roll it back.
    #[error("the sync was cancelled")]
    Cancelled,
    /// The checkpoint store failed. Nothing is lost: the next sync merges.
    #[error("sync checkpoint: {0}")]
    Checkpoint(String),
    /// The local or merged vault could not be encoded.
    #[error("could not encode the vault: {0}")]
    Encode(String),
}

/// When the local vault is written over a remote that holds nothing new for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// A watcher or timer pull: never.
    Pull,
    /// After a local save or on request: always.
    Push,
    /// Reconciling (unlock, retry): only when the local content or the file format (a new
    /// password, cipher or KDF) differs. A remote that is exactly this device's last write is
    /// decrypted once to compare, so an unchanged vault is not re-uploaded on every unlock.
    PushIfChanged,
}

/// What a successful sync did. Every variant that carries a [`PendingCheckpoint`] wants it
/// committed once the local state is durable (see the module docs).
#[derive(Debug)]
pub enum SyncResult {
    /// The remote was missing; the local vault was written there.
    UploadedInitial(PendingCheckpoint),
    /// Nothing written: the remote is what this device last wrote (no checkpoint to record), or
    /// local and remote already hold the same vault content.
    Unchanged(Option<PendingCheckpoint>),
    /// The local vault was written over a remote that held nothing new for it.
    UploadedLocal(PendingCheckpoint),
    /// The remote held changes the local vault lacks. The merge is already on the remote (unless
    /// the remote already held it all). The caller adopts the merged database, makes it durable,
    /// then commits the checkpoint.
    Merged(Box<MergedRemote>),
}

impl SyncResult {
    /// The checkpoint to commit once the local state is durable, if any.
    pub fn pending(&self) -> Option<&PendingCheckpoint> {
        match self {
            Self::UploadedInitial(p) | Self::UploadedLocal(p) => Some(p),
            Self::Unchanged(p) => p.as_ref(),
            Self::Merged(m) => Some(&m.checkpoint),
        }
    }

    /// The database to adopt (only for [`SyncResult::Merged`]) and the checkpoint to commit
    /// after the local state, including that adoption, is durable.
    pub fn into_parts(self) -> (Option<Database>, Option<PendingCheckpoint>) {
        match self {
            Self::UploadedInitial(p) | Self::UploadedLocal(p) => (None, Some(p)),
            Self::Unchanged(p) => (None, p),
            Self::Merged(m) => {
                let (db, p) = m.into_parts();
                (Some(db), Some(p))
            }
        }
    }
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

    /// Split into the merged database and the checkpoint to commit after adopting it AND making
    /// the adoption durable locally.
    ///
    /// Adopt by merging into the live database (`mypass_core::merge::merge(&live, &merged)`), not
    /// by replacing it: the live vault may have changed since `sync` took its snapshot, and a
    /// replacement would drop those edits.
    pub fn into_parts(self) -> (Database, PendingCheckpoint) {
        (self.database, self.checkpoint)
    }
}

/// A checkpoint of the remote content; stored only by [`SyncEngine::commit`], after the caller
/// made the local state it describes durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCheckpoint(SyncCheckpoint);

impl PendingCheckpoint {
    /// The checkpoint that `commit` will store.
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
    /// One verified secondary copy, keyed by its exact ciphertext digest. Allows an active
    /// reused engine to merge a previously verified old-password copy without retaining a
    /// password. Dropped with the engine. Production SecondarySync creates a fresh engine
    /// per run, so this cache does not carry verified content between production sync runs.
    verified: Mutex<Option<([u8; 32], Database)>>,
    /// The default automatic KDF ceilings, raised per dimension by verified costs. Always
    /// [`KdfCost::AUTOMATIC`] in production; the engine's own tests lower it so the budget can
    /// be exercised without gigabytes of Argon2.
    ceiling: KdfCost,
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
        KdbxError::RemoteKdfLimitExceeded(_) => SyncError::SyncKdfLimitExceeded {
            recovery_file: None,
        },
        KdbxError::Unsupported(m) => SyncError::RemoteUnsupported(m),
        KdbxError::Corrupt(m) | KdbxError::InvalidKdfParameters(m) => SyncError::RemoteCorrupt(m),
        e @ (KdbxError::Io { .. }
        | KdbxError::VaultRetained { .. }
        | KdbxError::Random
        | KdbxError::RetainedCopyRefused(_)
        | KdbxError::InvalidOperation(_)) => SyncError::RemoteCorrupt(e.to_string()),
    }
}

fn encode(db: &Database, password: &SecretString) -> Result<Vec<u8>, SyncError> {
    db.save(password)
        .map_err(|e| SyncError::Encode(e.to_string()))
}

fn checkpoint_error(e: std::io::Error) -> SyncError {
    SyncError::Checkpoint(format!("{:?}: {e}", e.kind()))
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The KDF parameters without the salt or seed (redrawn on every save).
fn kdf_shape(kdf: &KdfParams) -> KdfParams {
    let mut kdf = kdf.clone();
    match &mut kdf {
        KdfParams::Argon2 { salt, .. } => salt.clear(),
        KdfParams::AesKdf { seed, .. } => seed.clear(),
    }
    kdf
}

/// Whether two vaults are written in the same file format (salt and seed ignored).
fn same_format(a: &SaveSettings, b: &SaveSettings) -> bool {
    a.minor_version == b.minor_version
        && a.cipher == b.cipher
        && a.compression == b.compression
        && kdf_shape(&a.kdf) == kdf_shape(&b.kdf)
}

/// One attempt's outcome.
enum Attempt {
    Done(SyncResult),
    /// The remote changed before our write; start over with the same base.
    Raced,
    /// Our write landed, but the remote changed right around it (a displaced version, or one
    /// written right after us). Start over from this base, which holds everything we wrote plus
    /// the displaced version, so nothing seen so far can be lost.
    Rebase(Box<Database>),
}

/// Which plain result a write that needs no adoption reports.
#[derive(Clone, Copy)]
enum Upload {
    Initial,
    Local,
}

impl SyncEngine {
    /// An engine for `remote` with its checkpoint in `checkpoints`.
    pub fn new(remote: Arc<dyn StorageBackend>, checkpoints: Arc<dyn SyncCheckpointStore>) -> Self {
        Self {
            remote,
            checkpoints,
            echo: None,
            gate: Mutex::new(()),
            verified: Mutex::new(None),
            ceiling: KdfCost::AUTOMATIC,
        }
    }

    /// Note every remote write in `echo` just before it happens, so the folder watcher's event
    /// for this device's own upload is recognised and skipped.
    pub fn with_echo_filter(mut self, echo: Arc<EchoFilter>) -> Self {
        self.echo = Some(echo);
        self
    }

    /// [`SyncEngine::sync_checked`] that is never cancelled.
    pub fn sync(
        &self,
        local: &Database,
        password: &SecretString,
        mode: SyncMode,
    ) -> Result<SyncResult, SyncError> {
        self.sync_checked(local, password, mode, &|| true)
    }

    /// Reconcile `local` with the remote copy. `local` is never mutated and no checkpoint is
    /// stored (see [`SyncResult::pending`]). `still_current` is asked at the start of every
    /// attempt, before every decryption of the remote and right before every remote write;
    /// `false` ends the sync with [`SyncError::Cancelled`] and writes nothing more, so a caller
    /// that withdrew (a locked vault) gets its key material back after at most one more storage
    /// read or KDF run.
    ///
    /// - Remote missing: write `local` (never over a file that appeared meanwhile),
    ///   [`SyncResult::UploadedInitial`].
    /// - Remote present: verify and merge its content, including on checkpoint hits. Equal content is
    ///   [`SyncResult::Unchanged`] with no write (so two devices never ping-pong fresh encodings
    ///   of the same vault); a remote with nothing new gets the merge written and is
    ///   [`SyncResult::UploadedLocal`]; else [`SyncResult::Merged`].
    pub fn sync_checked(
        &self,
        local: &Database,
        password: &SecretString,
        mode: SyncMode,
        still_current: &dyn Fn() -> bool,
    ) -> Result<SyncResult, SyncError> {
        let _serial = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let vault = local.root.uuid.to_string();
        // The local vault's own cost, plus the highest cost recorded for this remote. A store
        // that cannot be read lends nothing (the narrower budget fails closed); the attempt
        // below reports a checkpoint failure where it needs the checkpoint itself.
        let recorded = self
            .checkpoints
            .load()
            .ok()
            .flatten()
            .map(|cp| cp.verified_kdf_for(&vault))
            .unwrap_or_default();
        let mut verified = KdfCost::of(&local.settings.kdf).max(recorded);
        let mut rebased: Option<Database> = None;
        for _ in 0..MAX_SYNC_ATTEMPTS {
            let step = self.attempt(
                local,
                rebased.as_ref(),
                password,
                mode,
                &vault,
                &mut verified,
                still_current,
            )?;
            match step {
                Attempt::Done(result) => return Ok(result),
                Attempt::Raced => {}
                Attempt::Rebase(base) => rebased = Some(*base),
            }
        }
        Err(SyncError::RemoteChangedDuringSync)
    }

    /// Store `pending` after the local state it describes is durable on the local disk (see the
    /// module docs). Until then the old checkpoint stays and the next sync merges again.
    pub fn commit(&self, pending: PendingCheckpoint) -> Result<(), SyncError> {
        let _serial = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        self.checkpoints
            .save(Some(&pending.0))
            .map_err(checkpoint_error)
    }

    /// The automatic budget for a remote open: the ceiling raised by what was verified.
    fn limits(&self, verified: KdfCost) -> KdfLimits {
        KdfLimits::within(self.ceiling.max(verified))
    }

    #[allow(clippy::too_many_arguments)] // one attempt's whole context, as `install` takes it
    fn attempt(
        &self,
        local: &Database,
        rebased: Option<&Database>,
        password: &SecretString,
        mode: SyncMode,
        vault: &str,
        verified: &mut KdfCost,
        still_current: &dyn Fn() -> bool,
    ) -> Result<Attempt, SyncError> {
        if !still_current() {
            return Err(SyncError::Cancelled);
        }
        let base = rebased.unwrap_or(local);
        let remote_bytes = self.remote.read()?;
        if !still_current() {
            return Err(SyncError::Cancelled);
        }
        let Some(remote_bytes) = remote_bytes else {
            let bytes = encode(base, password)?;
            let adopt = rebased.is_some();
            let write = Write::new(None, &bytes, base, adopt, Upload::Initial);
            return self.install(write, password, local, vault, verified, still_current);
        };

        let checkpoint = self.checkpoints.load().map_err(checkpoint_error)?;
        let ours = rebased.is_none()
            && checkpoint.as_ref().is_some_and(|cp| {
                cp.vault_uuid.as_deref() == Some(vault) && cp.describes(&remote_bytes)
            });
        if !still_current() {
            return Err(SyncError::Cancelled);
        }
        let (remote_db, needs_rekey) =
            match Database::open_with_limits(&remote_bytes, password, self.limits(*verified)) {
                Ok(db) => (db, false),
                Err(KdbxError::WrongPassword) => {
                    // Only exact, previously verified ciphertext can use the retained content.
                    // A persisted checkpoint is insufficient after a restart: fail closed then.
                    let cached = self.verified.lock().unwrap_or_else(PoisonError::into_inner);
                    let db = cached
                        .as_ref()
                        .filter(|(hash, _)| *hash == digest(&remote_bytes))
                        .map(|(_, db)| db.clone())
                        .ok_or(SyncError::RemotePasswordMismatch)?;
                    (db, true)
                }
                Err(e) => return Err(remote_open_error(e)),
            };
        if remote_db.root.uuid != local.root.uuid {
            return Err(SyncError::RemoteIsDifferentVault);
        }
        // Decrypted with the user's key (or exactly a copy this engine decrypted): its cost is
        // verified for this vault and remote.
        *verified = verified.max(KdfCost::of(&remote_db.settings.kdf));
        self.remember_verified(&remote_bytes, &remote_db);
        let merged = merge(base, &remote_db);
        // `Database` equality depends on child order and `merge` sorts by UUID, so each side is
        // compared in its canonical (self-merged) form: equal exactly when it lacks nothing.
        let remote_has_all = merged == merge(&remote_db, &remote_db);
        let local_has_all = merged == merge(local, local);
        if ours && local_has_all && mode == SyncMode::Pull {
            return Ok(Attempt::Done(SyncResult::Unchanged(None)));
        }
        if ours && remote_has_all && local_has_all {
            if mode == SyncMode::PushIfChanged
                && !needs_rekey
                && same_format(&remote_db.settings, &local.settings)
            {
                return Ok(Attempt::Done(SyncResult::Unchanged(None)));
            }
            if !still_current() {
                return Err(SyncError::Cancelled);
            }
            let bytes = encode(local, password)?;
            let expected = digest(&remote_bytes);
            let write = Write::new(Some(&expected), &bytes, local, false, Upload::Local);
            return self.install(write, password, local, vault, verified, still_current);
        }
        if remote_has_all && !needs_rekey {
            // Nothing to write: the remote already holds every change.
            let cp = PendingCheckpoint(self.checkpoint_of(&remote_bytes, vault, *verified));
            if local_has_all {
                return Ok(Attempt::Done(SyncResult::Unchanged(Some(cp))));
            }
            return Ok(Attempt::Done(SyncResult::Merged(Box::new(MergedRemote {
                database: merged,
                checkpoint: cp,
            }))));
        }
        if !still_current() {
            return Err(SyncError::Cancelled);
        }
        let bytes = encode(&merged, password)?;
        let expected = digest(&remote_bytes);
        let write = Write::new(
            Some(&expected),
            &bytes,
            &merged,
            !local_has_all,
            Upload::Local,
        );
        self.install(write, password, local, vault, verified, still_current)
    }

    fn remember_verified(&self, bytes: &[u8], db: &Database) {
        *self.verified.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((digest(bytes), db.clone()));
    }

    /// One conditional write and what it means for the attempt.
    fn install(
        &self,
        write: Write<'_>,
        password: &SecretString,
        local: &Database,
        vault: &str,
        verified: &mut KdfCost,
        still_current: &dyn Fn() -> bool,
    ) -> Result<Attempt, SyncError> {
        if !still_current() {
            return Err(SyncError::Cancelled);
        }
        if let Some(echo) = &self.echo {
            echo.note_own_write(SyncCheckpoint::of_bytes(write.bytes, None));
        }
        match self.remote.write_if(write.expected, write.bytes)? {
            WriteOutcome::Conflict => Ok(Attempt::Raced),
            outcome @ (WriteOutcome::Displaced(_) | WriteOutcome::DisplacedKept { .. }) => {
                let (theirs, hidden) = match outcome {
                    WriteOutcome::DisplacedKept { bytes, path } => (bytes, Some(path)),
                    WriteOutcome::Displaced(bytes) => (bytes, None),
                    _ => return Err(SyncError::Cancelled),
                };
                let kept = |path| SyncError::Storage(StorageError::VaultRetained { path });
                if !still_current() {
                    return Err(hidden.map_or(SyncError::Cancelled, kept));
                }
                // Carry the working path when the displaced copy cannot be merged.
                let kept_or_displaced =
                    |path: Option<_>| path.map_or(SyncError::ConcurrentCopyKept, kept);
                // Our bytes are on the remote, but they displaced a version another writer
                // committed in the last instant: merge it back on the next attempt.
                let theirs = Database::open_with_limits(&theirs, password, self.limits(*verified))
                    .map_err(|e| match e {
                        KdbxError::RemoteKdfLimitExceeded(_) => SyncError::SyncKdfLimitExceeded {
                            recovery_file: hidden.clone(),
                        },
                        _ => kept_or_displaced(hidden.clone()),
                    })?;
                if theirs.root.uuid != local.root.uuid {
                    return Err(kept_or_displaced(hidden));
                }
                *verified = verified.max(KdfCost::of(&theirs.settings.kdf));
                Ok(Attempt::Rebase(Box::new(merge(write.content, &theirs))))
            }
            WriteOutcome::Written => {
                if !still_current() {
                    return Err(SyncError::Cancelled);
                }
                // Read back: a writer that replaced the file right after us is merged by the next
                // attempt and never described by our checkpoint.
                if self.remote.read()?.as_deref() != Some(write.bytes) {
                    return Ok(Attempt::Rebase(Box::new(write.content.clone())));
                }
                self.remember_verified(write.bytes, write.content);
                // Our own encoding: its cost is the one we just derived a key for.
                *verified = verified.max(KdfCost::of(&write.content.settings.kdf));
                let cp = PendingCheckpoint(self.checkpoint_of(write.bytes, vault, *verified));
                Ok(Attempt::Done(if write.adopt {
                    SyncResult::Merged(Box::new(MergedRemote {
                        database: write.content.clone(),
                        checkpoint: cp,
                    }))
                } else {
                    match write.upload {
                        Upload::Initial => SyncResult::UploadedInitial(cp),
                        Upload::Local => SyncResult::UploadedLocal(cp),
                    }
                }))
            }
        }
    }

    fn checkpoint_of(&self, bytes: &[u8], vault: &str, verified: KdfCost) -> SyncCheckpoint {
        // The time is informational: a failure to read it never fails the sync.
        let modified = self.remote.modification_date().ok().flatten();
        SyncCheckpoint::of_bytes(bytes, modified)
            .for_vault(vault)
            .with_verified_kdf(verified)
    }
}

/// One conditional write of an attempt.
struct Write<'a> {
    /// The SHA-256 the remote must still hold (`None`: it must still be missing).
    expected: Option<&'a [u8; 32]>,
    bytes: &'a [u8],
    /// The database `bytes` encode.
    content: &'a Database,
    /// `content` holds something the local vault lacks: the result is `Merged`.
    adopt: bool,
    upload: Upload,
}

impl<'a> Write<'a> {
    fn new(
        expected: Option<&'a [u8; 32]>,
        bytes: &'a [u8],
        content: &'a Database,
        adopt: bool,
        upload: Upload,
    ) -> Self {
        Self {
            expected,
            bytes,
            content,
            adopt,
            upload,
        }
    }
}

#[cfg(test)]
mod tests;
