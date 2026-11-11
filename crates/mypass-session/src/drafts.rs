//! Encrypted note drafts (format `MPD1`), a port of the Swift `NoteDraftStore`.
//!
//! While the user edits an entry the notes text is kept, up to [`DRAFT_CAPACITY`] versions for
//! [`DRAFT_TTL`], so an accidental discard or a crash does not lose a long note. One file per
//! entry: `<dir>/<entry_uuid_simple>.mpd`, laid out as `b"MPD1"` + a 12-byte random nonce +
//! ChaCha20-Poly1305 ciphertext and tag. The authenticated data is
//! `"<vault_id>|<entry_uuid_simple>"`, so a file copied onto another entry, or into another
//! vault's folder, fails authentication and is ignored. The plaintext is a JSON array of
//! `{id, savedAt, notes}` with `savedAt` in Unix seconds.
//!
//! Why there is no plaintext fallback: notes are user content that the vault itself protects. If
//! the key is unavailable (the vault is locked, or the key could not be created) nothing is
//! written and the editor simply works without drafts.
//!
//! Why the key lives inside the vault (`Meta.custom_data`, [`DRAFT_KEY_CUSTOM_DATA`]): it
//! survives lock, travels with the vault file, needs neither the Keychain nor a signed build and
//! works the same on Windows. The hostile case this does not cover is documented: whoever can
//! read the key from the vault already has the vault password, and then the notes themselves.
//!
//! Drafts are keyed by vault identity (the root group UUID), never by file path, so moving the
//! vault keeps them. After a backup restore all drafts of the vault are discarded by the caller.
//!
//! Read-only use never dirties the vault: every read path (`versions`, `clear`, purge, restore)
//! goes through [`existing_key`] and never mutates. Only the first successful draft save calls
//! [`load_or_create_key`], which adds the key through `VaultSession::mutate`.
//!
//! A draft file is attacker-controllable (it sits in the user's data directory), so reading is
//! bounded by [`MAX_DRAFT_FILE_BYTES`] and every malformed shape is treated as "no versions".

use crate::dto::timestamp_ms;
use crate::session::{SessionError, VaultSession};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use mypass_kdbx::{CustomDataItem, Database};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;
use zeroize::Zeroizing;

/// The vault `Meta.custom_data` key holding the base64 of the 32 random key bytes.
pub const DRAFT_KEY_CUSTOM_DATA: &str = "MyPass.DraftKey";
/// Versions kept per entry.
pub const DRAFT_CAPACITY: usize = 10;
/// A version older than this is dropped.
pub const DRAFT_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// The largest draft file that is read or written. Ten versions of note text fit comfortably;
/// anything bigger is hostile or corrupt.
pub const MAX_DRAFT_FILE_BYTES: usize = 4 * 1024 * 1024;

const MAGIC: &[u8; 4] = b"MPD1";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const EXTENSION: &str = "mpd";

/// One saved version of an entry's notes, as the webview sees it.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DraftVersionDto {
    pub id: String,
    /// Unix milliseconds.
    pub saved_at: f64,
    pub notes: String,
}

impl fmt::Debug for DraftVersionDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DraftVersionDto")
            .field("id", &self.id)
            .field("saved_at", &self.saved_at)
            .finish_non_exhaustive()
    }
}

/// The on-disk shape inside the encrypted file: `savedAt` is integer Unix seconds.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StoredVersion {
    id: String,
    saved_at: i64,
    notes: String,
}

/// Reads and writes the draft files of one vault.
pub struct DraftStore {
    dir: PathBuf,
    vault_id: String,
    key: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for DraftStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DraftStore")
            .field("dir", &self.dir)
            .field("vault_id", &self.vault_id)
            .finish_non_exhaustive()
    }
}

impl DraftStore {
    /// `dir` is `AppPaths::drafts_dir(vault)`; the store only appends `<entry_uuid_simple>.mpd`.
    pub fn new(dir: PathBuf, vault_id: &str, key: [u8; 32]) -> Self {
        Self {
            dir,
            vault_id: vault_id.to_owned(),
            key: Zeroizing::new(key),
        }
    }

    /// Stores `notes` as the newest version of `entry`. `now` is Unix seconds. Returns `false`
    /// when nothing was stored (I/O failure, or the text is too large); a text identical to the
    /// newest version is a no-op that returns `true`.
    pub fn save(&self, entry: Uuid, notes: &str, now: i64) -> bool {
        let mut versions = self.read_fresh(entry, now);
        if versions.first().is_some_and(|v| v.notes == notes) {
            return true;
        }
        versions.insert(
            0,
            StoredVersion {
                id: Uuid::new_v4().simple().to_string(),
                saved_at: now,
                notes: notes.to_owned(),
            },
        );
        versions.truncate(DRAFT_CAPACITY);
        // Drop the oldest versions until the file fits the cap; the newest must fit alone.
        while !versions.is_empty() {
            match self.encode(entry, &versions) {
                Some(bytes) if bytes.len() <= MAX_DRAFT_FILE_BYTES => {
                    return self.write(entry, &bytes);
                }
                _ => {
                    if versions.len() == 1 {
                        return false;
                    }
                    versions.pop();
                }
            }
        }
        false
    }

    /// The live versions of `entry`, newest first. Expired, undecryptable and foreign files
    /// yield nothing.
    pub fn versions(&self, entry: Uuid, now: i64) -> Vec<DraftVersionDto> {
        self.read_fresh(entry, now)
            .into_iter()
            .map(|v| DraftVersionDto {
                id: v.id,
                saved_at: timestamp_ms(mypass_kdbx::Timestamp(v.saved_at)),
                notes: v.notes,
            })
            .collect()
    }

    /// Removes the drafts of one entry. Missing files are fine.
    pub fn clear(&self, entry: Uuid) {
        let _ = fs::remove_file(self.path(entry));
    }

    /// Removes every draft of this vault (the folder is dedicated to it).
    pub fn clear_all(&self) {
        let Ok(read) = fs::read_dir(&self.dir) else {
            return;
        };
        for item in read.flatten() {
            // `file_type` does not follow symlinks, so a link is removed, never its target.
            if item.file_type().is_ok_and(|t| !t.is_dir()) {
                let _ = fs::remove_file(item.path());
            }
        }
        // Best effort: succeeds only when the folder is now empty.
        let _ = fs::remove_dir(&self.dir);
    }

    /// Drops expired versions everywhere: files with no live version, and files that no longer
    /// decrypt, are removed; files with some expired versions are rewritten.
    pub fn purge_expired(&self, now: i64) {
        let Ok(read) = fs::read_dir(&self.dir) else {
            return;
        };
        for item in read.flatten() {
            let path = item.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                continue;
            }
            let Some(entry) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| Uuid::parse_str(s).ok())
            else {
                let _ = fs::remove_file(&path);
                continue;
            };
            let all = self.read_all(entry);
            let live: Vec<StoredVersion> = all
                .iter()
                .filter(|v| !expired(v.saved_at, now))
                .cloned()
                .collect();
            if live.is_empty() {
                let _ = fs::remove_file(&path);
            } else if live.len() != all.len() {
                match self.encode(entry, &live) {
                    Some(bytes) if self.write(entry, &bytes) => {}
                    _ => {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
    }

    fn path(&self, entry: Uuid) -> PathBuf {
        self.dir.join(format!("{}.{EXTENSION}", entry.simple()))
    }

    fn aad(&self, entry: Uuid) -> Vec<u8> {
        format!("{}|{}", self.vault_id, entry.simple()).into_bytes()
    }

    fn read_fresh(&self, entry: Uuid, now: i64) -> Vec<StoredVersion> {
        let mut all = self.read_all(entry);
        all.retain(|v| !expired(v.saved_at, now));
        all
    }

    /// Every version in the file, or nothing when it is absent, oversized, malformed, foreign or
    /// tampered with.
    fn read_all(&self, entry: Uuid) -> Vec<StoredVersion> {
        let Some(bytes) = read_bounded(&self.path(entry)) else {
            return Vec::new();
        };
        self.decode(entry, &bytes).unwrap_or_default()
    }

    fn decode(&self, entry: Uuid, bytes: &[u8]) -> Option<Vec<StoredVersion>> {
        let rest = bytes.strip_prefix(MAGIC.as_slice())?;
        let (nonce, ciphertext) = rest.split_at_checked(NONCE_LEN)?;
        if ciphertext.len() < TAG_LEN {
            return None;
        }
        let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
        let cipher = ChaCha20Poly1305::new(&(*self.key).into());
        let plain = cipher
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad: &self.aad(entry),
                },
            )
            .ok()
            .map(Zeroizing::new)?;
        serde_json::from_slice(&plain).ok()
    }

    fn encode(&self, entry: Uuid, versions: &[StoredVersion]) -> Option<Vec<u8>> {
        let plain = Zeroizing::new(serde_json::to_vec(versions).ok()?);
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let cipher = ChaCha20Poly1305::new(&(*self.key).into());
        let ciphertext = cipher
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: &plain,
                    aad: &self.aad(entry),
                },
            )
            .ok()?;
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ciphertext.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Some(out)
    }

    fn write(&self, entry: Uuid, bytes: &[u8]) -> bool {
        if create_private_dir(&self.dir).is_err() {
            return false;
        }
        // Atomic replace through a same-folder temp file, created owner-only on unix.
        mypass_kdbx::write_atomic(&self.path(entry), bytes).is_ok()
    }
}

fn expired(saved_at: i64, now: i64) -> bool {
    let ttl = i64::try_from(DRAFT_TTL.as_secs()).unwrap_or(i64::MAX);
    now.saturating_sub(saved_at) > ttl
}

/// Reads at most [`MAX_DRAFT_FILE_BYTES`]; a longer file is refused without reading it all.
fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_DRAFT_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= MAX_DRAFT_FILE_BYTES).then_some(bytes)
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// The draft key of an open database, or `None` when it has none (or it is malformed). Never
/// mutates: every read path uses this.
pub fn existing_key(db: &Database) -> Option<[u8; 32]> {
    let item = db
        .meta
        .custom_data
        .iter()
        .find(|i| i.key == DRAFT_KEY_CUSTOM_DATA)?;
    decode_key(&item.value)
}

fn decode_key(value: &str) -> Option<[u8; 32]> {
    let raw = Zeroizing::new(
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .ok()?,
    );
    raw.as_slice().try_into().ok()
}

/// The draft key, created on first use. The vault is modified (through `mutate`, so the session
/// turns pending once) only when no valid key exists yet; call this from the first draft save
/// only, never from a read path.
pub fn load_or_create_key(session: &VaultSession) -> Result<[u8; 32], SessionError> {
    if let Some(key) = session.with_db(existing_key)? {
        return Ok(key);
    }
    session.mutate(|db, now| {
        // Checked again under the session lock: a concurrent creator wins.
        if let Some(key) = existing_key(db) {
            return Ok(key);
        }
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        let encoded = base64::engine::general_purpose::STANDARD.encode(key);
        match db
            .meta
            .custom_data
            .iter_mut()
            .find(|i| i.key == DRAFT_KEY_CUSTOM_DATA)
        {
            // A malformed item is repaired in place rather than duplicated.
            Some(item) => {
                item.value = encoded;
                item.last_modified = Some(now);
            }
            None => db.meta.custom_data.push(CustomDataItem {
                key: DRAFT_KEY_CUSTOM_DATA.to_owned(),
                value: encoded,
                last_modified: Some(now),
            }),
        }
        Ok(key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saving::AUTOSAVE_DEBOUNCE;
    use crate::scheduler::ManualScheduler;
    use crate::session::{
        LockReason, SaveState, SessionDeps, SessionEvent, SessionState, VaultSession,
    };
    use crate::settings::SettingsStore;
    use crate::testing::fast_database;
    use mypass_kdbx::{Database, Entry, PlainFileIo, Timestamp};
    use secrecy::SecretString;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;

    const VAULT: &str = "0123456789abcdef0123456789abcdef";
    const KEY: [u8; 32] = [7; 32];

    fn store(dir: &TempDir) -> DraftStore {
        DraftStore::new(dir.path().join("drafts"), VAULT, KEY)
    }

    fn texts(v: &[DraftVersionDto]) -> Vec<&str> {
        v.iter().map(|d| d.notes.as_str()).collect()
    }

    fn file_of(dir: &TempDir, entry: Uuid) -> PathBuf {
        dir.path()
            .join("drafts")
            .join(format!("{}.mpd", entry.simple()))
    }

    #[test]
    fn save_then_versions_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "a", 1));
        assert!(s.save(e, "b", 2));
        let v = s.versions(e, 3);
        assert_eq!(texts(&v), ["b", "a"]);
        assert_eq!(v[0].saved_at, 2000.0);
        assert_eq!(v[1].saved_at, 1000.0);
        assert_ne!(v[0].id, v[1].id);
    }

    #[test]
    fn identical_to_newest_is_not_duplicated() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "a", 1));
        assert!(s.save(e, "a", 2));
        let v = s.versions(e, 3);
        assert_eq!(texts(&v), ["a"]);
        assert_eq!(v[0].saved_at, 1000.0);
    }

    #[test]
    fn capacity_keeps_newest_ten() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        for i in 0..12 {
            assert!(s.save(e, &format!("n{i}"), i));
        }
        let v = s.versions(e, 20);
        assert_eq!(v.len(), DRAFT_CAPACITY);
        assert_eq!(v[0].notes, "n11");
        assert_eq!(v[9].notes, "n2");
        assert!(!texts(&v).contains(&"n0") && !texts(&v).contains(&"n1"));
    }

    #[test]
    fn versions_older_than_seven_days_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "a", 0));
        assert_eq!(s.versions(e, 7 * 86_400).len(), 1);
        assert!(s.versions(e, 7 * 86_400 + 1).is_empty());
    }

    #[test]
    fn file_starts_with_mpd1_magic_and_is_not_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "super secret note text", 1));
        let bytes = fs::read(file_of(&dir, e)).unwrap();
        assert!(bytes.starts_with(b"MPD1"));
        let needle = b"super secret note text";
        assert!(!bytes.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn draft_copied_to_other_entry_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(s.save(a, "a note", 1));
        fs::copy(file_of(&dir, a), file_of(&dir, b)).unwrap();
        assert_eq!(s.versions(a, 2).len(), 1);
        assert!(s.versions(b, 2).is_empty());
    }

    #[test]
    fn draft_with_other_vault_id_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "a note", 1));
        let other = DraftStore::new(
            dir.path().join("drafts"),
            "fedcba9876543210fedcba9876543210",
            KEY,
        );
        assert!(other.versions(e, 2).is_empty());
    }

    #[test]
    fn wrong_key_yields_no_versions() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "a note", 1));
        let other = DraftStore::new(dir.path().join("drafts"), VAULT, [8; 32]);
        assert!(other.versions(e, 2).is_empty());
    }

    #[test]
    fn tampered_ciphertext_yields_no_versions() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "a note", 1));
        let path = file_of(&dir, e);
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(s.versions(e, 2).is_empty());
    }

    #[test]
    fn short_garbled_and_oversized_files_yield_no_versions_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "a note", 1));
        let path = file_of(&dir, e);
        for bytes in [
            Vec::new(),
            b"MPD".to_vec(),
            b"MPD1".to_vec(),
            b"MPD1".iter().copied().chain([0u8; 20]).collect(),
            vec![0xAB; 64],
            vec![0u8; MAX_DRAFT_FILE_BYTES + 1],
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(s.versions(e, 2).is_empty());
        }
        // A new save recovers over the garbage.
        assert!(s.save(e, "fresh", 3));
        assert_eq!(texts(&s.versions(e, 4)), ["fresh"]);
    }

    #[test]
    fn oversized_notes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        let huge = "x".repeat(MAX_DRAFT_FILE_BYTES);
        assert!(!s.save(e, &huge, 1));
        assert!(s.versions(e, 2).is_empty());
    }

    #[test]
    fn clear_removes_file_and_clear_all_removes_directory_contents() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(s.save(a, "a", 1));
        assert!(s.save(b, "b", 1));
        s.clear(a);
        assert!(!file_of(&dir, a).exists());
        assert!(file_of(&dir, b).exists());
        s.clear_all();
        let left = fs::read_dir(dir.path().join("drafts"))
            .map(|d| d.count())
            .unwrap_or(0);
        assert_eq!(left, 0);
        // Clearing what does not exist is fine.
        s.clear(a);
        s.clear_all();
    }

    #[test]
    fn purge_expired_removes_only_expired_and_unreadable_files() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let (old, fresh, junk) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        assert!(s.save(old, "old", 0));
        assert!(s.save(fresh, "fresh", 8 * 86_400));
        fs::write(file_of(&dir, junk), b"garbage").unwrap();
        s.purge_expired(8 * 86_400 + 1);
        assert!(!file_of(&dir, old).exists());
        assert!(!file_of(&dir, junk).exists());
        assert_eq!(s.versions(fresh, 8 * 86_400 + 1).len(), 1);
    }

    #[test]
    fn purge_expired_trims_expired_versions_of_a_mixed_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "old", 0));
        assert!(s.save(e, "new", 6 * 86_400));
        s.purge_expired(7 * 86_400 + 1);
        assert_eq!(texts(&s.versions(e, 7 * 86_400 + 1)), ["new"]);
    }

    #[test]
    fn unwritable_directory_returns_false_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"a file, not a directory").unwrap();
        let s = DraftStore::new(blocker.join("drafts"), VAULT, KEY);
        assert!(!s.save(Uuid::new_v4(), "a", 1));
    }

    #[test]
    fn debug_output_redacts_notes_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        let e = Uuid::new_v4();
        assert!(s.save(e, "very private words", 1));
        let v = s.versions(e, 2);
        assert!(!format!("{v:?}").contains("very private words"));
        assert!(!format!("{s:?}").contains("[7"));
    }

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    struct Fx {
        dir: TempDir,
        session: VaultSession,
        scheduler: Arc<ManualScheduler>,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let session = VaultSession::new(SessionDeps {
            settings: SettingsStore::in_memory(),
            scheduler: scheduler.clone(),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        Fx {
            dir,
            session,
            scheduler,
        }
    }

    impl Fx {
        fn created(&self) -> PathBuf {
            let path = self.dir.path().join("v.kdbx");
            let mut db = fast_database("Test Vault");
            let now = Timestamp::now();
            let mut e = Entry::new(now);
            e.set_field("Title", "first", false);
            let root = db.root_id();
            db.add_entry(root, e, now).unwrap();
            self.session.create_vault_with(&path, pw("pw"), db).unwrap();
            fs::canonicalize(path).unwrap()
        }
    }

    fn key_items(db: &Database) -> usize {
        db.meta
            .custom_data
            .iter()
            .filter(|i| i.key == DRAFT_KEY_CUSTOM_DATA)
            .count()
    }

    #[test]
    fn existing_key_absent_returns_none() {
        let db = fast_database("Empty");
        assert!(existing_key(&db).is_none());
    }

    #[test]
    fn existing_key_rejects_malformed_values() {
        for value in ["", "not base64 !!", "AAAA"] {
            let mut db = fast_database("Bad");
            db.meta.custom_data.push(mypass_kdbx::CustomDataItem {
                key: DRAFT_KEY_CUSTOM_DATA.to_owned(),
                value: value.to_owned(),
                last_modified: None,
            });
            assert!(existing_key(&db).is_none(), "{value:?}");
        }
    }

    #[test]
    fn load_or_create_key_creates_once_and_marks_dirty_once() {
        let f = fx();
        f.created();
        let pending = Arc::new(AtomicUsize::new(0));
        let counter = pending.clone();
        f.session.add_observer(Arc::new(move |ev| {
            if let SessionEvent::SaveChanged(s) = ev {
                if s.state == SaveState::Pending {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            }
        }));
        let first = load_or_create_key(&f.session).unwrap();
        let second = load_or_create_key(&f.session).unwrap();
        assert_eq!(first, second);
        assert_eq!(pending.load(Ordering::SeqCst), 1);
        let items = f.session.with_db(key_items).unwrap();
        assert_eq!(items, 1);
        assert_eq!(f.session.with_db(existing_key).unwrap(), Some(first));
    }

    #[test]
    fn load_or_create_key_repairs_a_malformed_item_without_duplicating_it() {
        let f = fx();
        f.created();
        f.session
            .mutate(|db, _| {
                db.meta.custom_data.push(mypass_kdbx::CustomDataItem {
                    key: DRAFT_KEY_CUSTOM_DATA.to_owned(),
                    value: "junk".to_owned(),
                    last_modified: None,
                });
                Ok(())
            })
            .unwrap();
        let key = load_or_create_key(&f.session).unwrap();
        assert_eq!(f.session.with_db(key_items).unwrap(), 1);
        assert_eq!(f.session.with_db(existing_key).unwrap(), Some(key));
    }

    #[test]
    fn key_is_32_random_bytes_base64() {
        use base64::Engine as _;
        let (a, b) = (fx(), fx());
        a.created();
        b.created();
        let ka = load_or_create_key(&a.session).unwrap();
        let kb = load_or_create_key(&b.session).unwrap();
        assert_ne!(ka, kb);
        let value = a
            .session
            .with_db(|db| {
                db.meta
                    .custom_data
                    .iter()
                    .find(|i| i.key == DRAFT_KEY_CUSTOM_DATA)
                    .map(|i| i.value.clone())
            })
            .unwrap()
            .unwrap();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(value)
            .unwrap();
        assert_eq!(raw.len(), 32);
        assert_eq!(raw, ka);
    }

    #[test]
    fn draft_key_added_lazily_and_not_modifying_vault_when_drafts_unused() {
        let f = fx();
        let path = f.created();
        let drafts = f.dir.path().join("drafts");
        let entry = Uuid::new_v4();
        let before_bytes = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        // Every read path: no key exists, so nothing is read, cleared or created.
        assert!(f.session.with_db(existing_key).unwrap().is_none());
        let vault_id = f.session.vault_id().unwrap();
        let store = DraftStore::new(drafts.clone(), &vault_id, KEY);
        assert!(store.versions(entry, 1).is_empty());
        store.clear(entry);
        store.purge_expired(1);
        f.session.check_external_change().unwrap();
        f.session.lock(LockReason::Manual);
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.session.status().state, SessionState::Unlocked);
        f.scheduler.advance(AUTOSAVE_DEBOUNCE * 2);

        assert_eq!(f.session.status().save.state, SaveState::Idle);
        assert_eq!(f.session.with_db(key_items).unwrap(), 0);
        assert_eq!(fs::read(&path).unwrap(), before_bytes);
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime
        );

        // The first draft save creates the key, once, and the next save writes it to disk.
        let key = load_or_create_key(&f.session).unwrap();
        assert_eq!(f.session.status().save.state, SaveState::Pending);
        assert_eq!(f.session.with_db(key_items).unwrap(), 1);
        let store = DraftStore::new(drafts, &vault_id, key);
        assert!(store.save(entry, "note", 5));
        assert_eq!(texts(&store.versions(entry, 6)), ["note"]);
        f.scheduler.advance(AUTOSAVE_DEBOUNCE);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        let (_, on_disk) = mypass_kdbx::VaultFile::open(&path, &pw("pw")).unwrap();
        assert_eq!(existing_key(&on_disk), Some(key));
        assert_ne!(fs::read(&path).unwrap(), before_bytes);
    }
}
