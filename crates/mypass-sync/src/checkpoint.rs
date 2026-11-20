//! The sync checkpoint: what the remote file held right after this device last synced with it.
//!
//! Hash-based (length plus SHA-256), an improvement on the old app's date-only checkpoint, which
//! missed changes within the file system's timestamp granularity and broke on missing dates. The
//! modification time is kept for display only and never decides anything.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

/// Largest checkpoint file read back, in bytes. A checkpoint is under 300 bytes of JSON; a larger
/// file is damaged and reads as "no checkpoint".
const MAX_CHECKPOINT_FILE_BYTES: u64 = 64 * 1024;

/// The remote file's content identity at the last sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCheckpoint {
    /// Length of the remote file in bytes.
    pub len: u64,
    /// The remote file's modification time in Unix seconds, when the storage reports one.
    /// Informational only.
    pub modified_secs: Option<i64>,
    /// Lower-case hex SHA-256 of the remote file.
    pub sha256_hex: String,
    /// Root group UUID of the vault this checkpoint was taken for; `None` for a bare content
    /// stamp. This associates the stamp with its vault; it does not prove that a primary
    /// reloaded or replaced since publication still contains the secondary's content.
    pub vault_uuid: Option<String>,
    /// The highest KDF cost that already authenticated with the user's key for this (vault,
    /// remote) pair, carried from checkpoint to checkpoint. It raises the automatic KDF budget
    /// of later syncs, so a vault whose KDF was lowered still syncs with the older, costlier
    /// copy it verified before. Only a successful decryption records a cost: a hostile file
    /// cannot ratchet it. Absent in checkpoints written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_kdf: Option<VerifiedKdfCost>,
}

/// [`mypass_kdbx::KdfCost`] as stored in a checkpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedKdfCost {
    /// Argon2 memory, in bytes.
    pub argon2_memory: u64,
    /// Argon2 memory times iterations, in bytes.
    pub argon2_work: u64,
    /// AES-KDF rounds.
    pub aes_rounds: u64,
}

impl From<mypass_kdbx::KdfCost> for VerifiedKdfCost {
    fn from(c: mypass_kdbx::KdfCost) -> Self {
        Self {
            argon2_memory: c.argon2_memory,
            argon2_work: c.argon2_work,
            aes_rounds: c.aes_rounds,
        }
    }
}

impl From<VerifiedKdfCost> for mypass_kdbx::KdfCost {
    fn from(c: VerifiedKdfCost) -> Self {
        Self {
            argon2_memory: c.argon2_memory,
            argon2_work: c.argon2_work,
            aes_rounds: c.aes_rounds,
        }
    }
}

impl SyncCheckpoint {
    /// The checkpoint of `bytes` (content identity only, no vault).
    pub fn of_bytes(bytes: &[u8], modified: Option<SystemTime>) -> Self {
        Self {
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            modified_secs: modified.map(unix_secs),
            sha256_hex: sha256_hex(bytes),
            vault_uuid: None,
            verified_kdf: None,
        }
    }

    /// Record which vault (root group UUID) this checkpoint was taken for.
    pub fn for_vault(mut self, vault_uuid: impl Into<String>) -> Self {
        self.vault_uuid = Some(vault_uuid.into());
        self
    }

    /// Record the highest KDF cost verified with the user's key (see [`Self::verified_kdf`]).
    pub fn with_verified_kdf(mut self, cost: mypass_kdbx::KdfCost) -> Self {
        self.verified_kdf = Some(cost.into());
        self
    }

    /// The recorded verified cost when this checkpoint was taken for `vault` (zero otherwise).
    pub fn verified_kdf_for(&self, vault: &str) -> mypass_kdbx::KdfCost {
        self.verified_kdf
            .filter(|_| self.vault_uuid.as_deref() == Some(vault))
            .map(Into::into)
            .unwrap_or_default()
    }

    /// Whether both describe the same content (length and hash; time, vault and cost ignored).
    pub fn same_content(&self, other: &SyncCheckpoint) -> bool {
        self.len == other.len && self.sha256_hex == other.sha256_hex
    }

    /// Whether this checkpoint describes exactly `bytes`.
    pub fn describes(&self, bytes: &[u8]) -> bool {
        self.len == u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            && self.sha256_hex == sha256_hex(bytes)
    }

    fn is_well_formed(&self) -> bool {
        self.sha256_hex.len() == 64
            && self
                .sha256_hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}

fn unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_secs()).map_or(i64::MIN, |s| -s),
    }
}

/// Lower-case hex SHA-256.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Where the checkpoint lives between syncs. One store per (vault, remote) pair.
pub trait SyncCheckpointStore: Send + Sync {
    /// The stored checkpoint; `None` when there is none or it is unreadable as a checkpoint.
    fn load(&self) -> io::Result<Option<SyncCheckpoint>>;
    /// Replace the stored checkpoint; `None` clears it (the next sync then merges).
    fn save(&self, cp: Option<&SyncCheckpoint>) -> io::Result<()>;
}

/// A checkpoint in a JSON file, written atomically. Clearing writes `null` (no file is deleted).
#[derive(Debug, Clone)]
pub struct FileCheckpointStore(PathBuf);

impl FileCheckpointStore {
    /// A store at `path`; the parent folder is created on the first save.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// The checkpoint file.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

fn kdbx_to_io(e: mypass_kdbx::KdbxError) -> io::Error {
    match e {
        mypass_kdbx::KdbxError::Io { kind, message } => io::Error::new(kind, message),
        other => io::Error::other(other.to_string()),
    }
}

impl SyncCheckpointStore for FileCheckpointStore {
    fn load(&self) -> io::Result<Option<SyncCheckpoint>> {
        use mypass_kdbx::FileIo as _;
        let bytes = match mypass_kdbx::PlainFileIo.read_limited(&self.0, MAX_CHECKPOINT_FILE_BYTES)
        {
            Ok(bytes) => bytes,
            Err(e) => {
                let e = kdbx_to_io(e);
                return match e.kind() {
                    io::ErrorKind::NotFound => Ok(None),
                    // Too large or not a regular file: damaged, so "no checkpoint" (a merge).
                    io::ErrorKind::FileTooLarge | io::ErrorKind::InvalidInput => Ok(None),
                    _ => Err(e),
                };
            }
        };
        // A damaged file reads as "no checkpoint": the next sync merges, which is always safe.
        let parsed = serde_json::from_slice::<Option<SyncCheckpoint>>(&bytes).unwrap_or(None);
        Ok(parsed.filter(SyncCheckpoint::is_well_formed))
    }

    fn save(&self, cp: Option<&SyncCheckpoint>) -> io::Result<()> {
        let json = serde_json::to_vec(&cp).map_err(io::Error::other)?;
        if let Some(parent) = self.0.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        mypass_kdbx::write_atomic(&self.0, &json).map_err(kdbx_to_io)
    }
}

/// A checkpoint held in memory (tests, and a remote whose checkpoint need not survive a restart).
#[derive(Debug, Default)]
pub struct InMemoryCheckpointStore {
    inner: Mutex<Option<SyncCheckpoint>>,
}

impl InMemoryCheckpointStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SyncCheckpointStore for InMemoryCheckpointStore {
    fn load(&self) -> io::Result<Option<SyncCheckpoint>> {
        Ok(self
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    fn save(&self, cp: Option<&SyncCheckpoint>) -> io::Result<()> {
        *self.inner.lock().unwrap_or_else(PoisonError::into_inner) = cp.cloned();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_round_trip_and_corrupt_file_reads_as_none() {
        let d = tempfile::tempdir().unwrap();
        let store = FileCheckpointStore::new(d.path().join("state").join("sync.json"));
        assert_eq!(store.load().unwrap(), None);
        let cp = SyncCheckpoint::of_bytes(b"remote bytes", Some(SystemTime::now()))
            .for_vault("0b6c4b1e-4f3c-4a43-9d38-1f6ad1d1c0de");
        store.save(Some(&cp)).unwrap();
        assert_eq!(store.load().unwrap(), Some(cp.clone()));
        // Clearing writes `null` and reads back as none.
        store.save(None).unwrap();
        assert!(store.path().exists());
        assert_eq!(store.load().unwrap(), None);
        // Damaged files read as none, never as an error that would block syncing.
        for damaged in [
            &b"{not json"[..],
            b"",
            b"{\"len\":1}",
            b"{\"len\":1,\"modified_secs\":null,\"sha256_hex\":\"XYZ\",\"vault_uuid\":null}",
        ] {
            fs::write(store.path(), damaged).unwrap();
            assert_eq!(store.load().unwrap(), None, "{damaged:?}");
        }
        fs::write(store.path(), vec![b' '; 70 * 1024]).unwrap();
        assert_eq!(store.load().unwrap(), None);
        // A save after damage works again.
        store.save(Some(&cp)).unwrap();
        assert_eq!(store.load().unwrap(), Some(cp));
    }

    #[test]
    fn verified_kdf_round_trips_and_older_checkpoints_still_load() {
        let d = tempfile::tempdir().unwrap();
        let store = FileCheckpointStore::new(d.path().join("sync.json"));
        let cost = mypass_kdbx::KdfCost {
            argon2_memory: 2 << 30,
            argon2_work: 6 << 30,
            aes_rounds: 0,
        };
        let cp = SyncCheckpoint::of_bytes(b"remote", None)
            .for_vault("v1")
            .with_verified_kdf(cost);
        store.save(Some(&cp)).unwrap();
        let back = store.load().unwrap().unwrap();
        assert_eq!(back, cp);
        assert_eq!(back.verified_kdf_for("v1"), cost);
        // Another vault's checkpoint never lends its cost.
        assert_eq!(back.verified_kdf_for("v2"), mypass_kdbx::KdfCost::default());
        assert!(back.same_content(&SyncCheckpoint::of_bytes(b"remote", None)));
        // A checkpoint written before the field existed loads without a recorded cost.
        let old = format!(
            "{{\"len\":6,\"modified_secs\":null,\"sha256_hex\":\"{}\",\"vault_uuid\":\"v1\"}}",
            sha256_hex(b"remote")
        );
        fs::write(store.path(), old).unwrap();
        let back = store.load().unwrap().unwrap();
        assert_eq!(back.verified_kdf, None);
        assert_eq!(back.verified_kdf_for("v1"), mypass_kdbx::KdfCost::default());
        // Without a recorded cost nothing new is serialised.
        let json = serde_json::to_string(&SyncCheckpoint::of_bytes(b"x", None)).unwrap();
        assert!(!json.contains("verified_kdf"), "{json}");
    }

    #[test]
    fn checkpoint_compares_content_not_time_or_vault() {
        let a = SyncCheckpoint::of_bytes(b"abc", Some(UNIX_EPOCH)).for_vault("v1");
        let b = SyncCheckpoint::of_bytes(b"abc", None);
        assert!(a.same_content(&b));
        assert!(a.describes(b"abc"));
        assert!(!a.describes(b"abd"));
        assert_eq!(a.modified_secs, Some(0));
        assert_eq!(
            a.sha256_hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn in_memory_store_round_trip() {
        let store = InMemoryCheckpointStore::new();
        assert_eq!(store.load().unwrap(), None);
        let cp = SyncCheckpoint::of_bytes(b"x", None);
        store.save(Some(&cp)).unwrap();
        assert_eq!(store.load().unwrap(), Some(cp));
        store.save(None).unwrap();
        assert_eq!(store.load().unwrap(), None);
    }
}
