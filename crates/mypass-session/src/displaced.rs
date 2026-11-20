//! Retained vault copies and move-aside working files next to a vault.
//!
//! A replacement that loses a last-instant race against another writer keeps the other writer's
//! file next to the vault as `.<name>.<pid>.<nanos>.<n>.displaced` (see
//! `mypass_kdbx::ReplaceOutcome::Displaced`). MyPass never deletes one on its own: it may be the
//! only copy of someone's edit. This module lists them for the Settings vault pane and deletes
//! one only when the user confirms that exact copy. A move-aside working file, or the file an
//! atomic exchange swapped out (`.exchanged`), can also remain after a failed replacement and is
//! labelled separately. Temp names are never listed: they can hold an uninstalled candidate.
//!
//! The status carries file names only (no folder), a location marker and, for a folder that
//! could not be read, an `unavailable` marker: a failed scan is never reported as "no copies".
//!
//! Deletion is bound to what the user saw. Every replacement records the exact identity of each
//! copy it keeps (`mypass_kdbx::RetainedIdentity`: the vault, the copy path, length and full
//! SHA-256) at write time, and [`RetainedRegistry`] persists those records under the config
//! folder. A listing made while the vault is unlocked hashes every copy and gives each deletable
//! one an opaque id, `HMAC(salt, location, path, sha256, length)`, with a salt drawn once per app
//! run, and remembers it together with the vault id and the unlock incarnation. A delete takes
//! only that id: the id must come from the current listing of the same unlocked session, the copy
//! is listed and hashed again and must give the same id. The file layer authenticates the exact
//! copy with the unlocked vault's key and compares its root UUID. It then re-verifies the name,
//! type, link count, bytes, provenance and the live vault before it unlinks anything
//! (`mypass_kdbx::remove_retained_copy`). An id from an earlier listing of another vault, another
//! unlock or another app run matches nothing and deletes nothing.

use crate::session::{SessionError, VaultSession};
use hmac::{Hmac, KeyInit, Mac};
use mypass_kdbx::{
    inspect_retained_copies, recorded_retained_copies, DisplacedCopyKind, FileIo, KdbxError,
    PlainFileIo, RetainedIdentity, RetainedRemoval,
};
use rand::{rand_core::UnwrapErr, rngs::SysRng, Rng};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Which copy of the vault the displaced file sits next to.
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum DisplacedLocation {
    /// Next to the open vault file.
    Primary,
    /// Next to the copy in the secondary sync folder.
    Secondary,
}

/// How the replacement named the copy.
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum RetainedCopyKind {
    Displaced,
    Aside,
    /// The previous file an atomic exchange swapped out, left under its working name.
    Exchanged,
}

/// One retained copy. `name` is the file name only, for display.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DisplacedCopyInfo {
    pub location: DisplacedLocation,
    pub kind: RetainedCopyKind,
    pub name: String,
    /// Length in bytes.
    pub bytes: f64,
    /// The opaque id `sync_displaced_delete` takes. `None` when this copy may not be deleted
    /// (the vault is locked, it could not be verified, it may belong to another vault or to a
    /// replacement still in progress).
    pub id: Option<String>,
}

/// The retained copies of the open vault.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DisplacedCopies {
    pub count: u32,
    pub copies: Vec<DisplacedCopyInfo>,
    /// Locations whose folder could not be read: their copies are unknown, not absent.
    pub unavailable: Vec<DisplacedLocation>,
}

/// The vault files whose neighbours are searched: the open vault and, when a secondary folder is
/// set, the copy of it in that folder (the same file name).
pub(crate) fn targets(
    primary: Option<PathBuf>,
    secondary_folder: Option<&str>,
) -> Vec<(DisplacedLocation, PathBuf)> {
    let Some(primary) = primary else {
        return Vec::new();
    };
    let mut all = Vec::new();
    if let (Some(folder), Some(name)) = (secondary_folder, primary.file_name()) {
        let secondary = Path::new(folder).join(name);
        if secondary.parent() != primary.parent() {
            all.push((DisplacedLocation::Secondary, secondary));
        }
    }
    all.insert(0, (DisplacedLocation::Primary, primary));
    all
}

/// What a listing's ids are bound to: the unlocked vault, its unlock incarnation and the
/// searched vault files and whether the primary supports deletion. A delete recomputes it;
/// any difference makes every id stale.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ListingKey {
    pub(crate) vault_id: String,
    pub(crate) incarnation: u64,
    pub(crate) targets: Vec<(DisplacedLocation, PathBuf)>,
    pub(crate) can_delete_primary: bool,
}

/// One deletable copy of the current listing, as the user saw it.
#[derive(Clone)]
struct ListedCopy {
    id: String,
    location: DisplacedLocation,
    /// The vault path as configured (the delete resolves it again).
    target: PathBuf,
    /// The copy's resolved path.
    path: PathBuf,
    name: OsString,
    len: u64,
    sha256: [u8; 32],
}

struct Listed {
    key: ListingKey,
    copies: Vec<ListedCopy>,
}

/// The ids of the latest listing and the per-run salt they are made with.
pub(crate) struct RetainedListing {
    salt: [u8; 64],
    listed: Mutex<Option<Listed>>,
}

impl RetainedListing {
    pub(crate) fn new() -> Self {
        let mut salt = [0u8; 64];
        UnwrapErr(SysRng).fill_bytes(&mut salt);
        Self {
            salt,
            listed: Mutex::new(None),
        }
    }

    fn listed(&self) -> MutexGuard<'_, Option<Listed>> {
        self.listed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn id(
        &self,
        key: &ListingKey,
        location: DisplacedLocation,
        path: &Path,
        len: u64,
        sha256: &[u8; 32],
    ) -> String {
        // A 64-byte key is exactly the SHA-256 HMAC block size, so initialization is infallible.
        let mut mac = Hmac::<Sha256>::new((&self.salt).into());
        // Bound to the vault and its unlock: the same file listed after a relock gets a new id.
        let vault = key.vault_id.as_bytes();
        Mac::update(
            &mut mac,
            &u64::try_from(vault.len()).unwrap_or(u64::MAX).to_le_bytes(),
        );
        Mac::update(&mut mac, vault);
        Mac::update(&mut mac, &key.incarnation.to_le_bytes());
        Mac::update(
            &mut mac,
            match location {
                DisplacedLocation::Primary => &[0u8],
                DisplacedLocation::Secondary => &[1u8],
            },
        );
        let path = path.as_os_str().as_encoded_bytes();
        Mac::update(
            &mut mac,
            &u64::try_from(path.len()).unwrap_or(u64::MAX).to_le_bytes(),
        );
        Mac::update(&mut mac, path);
        Mac::update(&mut mac, sha256);
        Mac::update(&mut mac, &len.to_le_bytes());
        hex(&mac.finalize().into_bytes())
    }
}

/// Lists the retained copies of every target. With `ids` (an unlocked vault), every copy that
/// may be deleted gets an id and the listing replaces the remembered one.
#[allow(clippy::cast_precision_loss)] // a vault file is far below 2^53 bytes
pub(crate) fn list(
    targets: &[(DisplacedLocation, PathBuf)],
    known: &[RetainedIdentity],
    ids: Option<(&RetainedListing, ListingKey)>,
) -> DisplacedCopies {
    let mut copies = Vec::new();
    let mut listed = Vec::new();
    let mut unavailable = Vec::new();
    for (location, target) in targets {
        let inspection = match inspect_retained_copies(target, known) {
            Ok(inspection) => inspection,
            Err(_) => {
                unavailable.push(*location);
                continue;
            }
        };
        for check in inspection.copies {
            let id = match (&ids, check.verdict, check.sha256, check.path.file_name()) {
                (Some((listing, key)), Ok(_), Some(sha256), Some(name))
                    if *location != DisplacedLocation::Primary || key.can_delete_primary =>
                {
                    let id = listing.id(key, *location, &check.path, check.len, &sha256);
                    listed.push(ListedCopy {
                        id: id.clone(),
                        location: *location,
                        target: target.clone(),
                        path: check.path.clone(),
                        name: name.to_os_string(),
                        len: check.len,
                        sha256,
                    });
                    Some(id)
                }
                _ => None,
            };
            copies.push(DisplacedCopyInfo {
                location: *location,
                kind: match check.kind {
                    DisplacedCopyKind::Displaced => RetainedCopyKind::Displaced,
                    DisplacedCopyKind::Aside => RetainedCopyKind::Aside,
                    DisplacedCopyKind::Exchanged => RetainedCopyKind::Exchanged,
                },
                name: check
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                bytes: check.len as f64,
                id,
            });
        }
    }
    if let Some((listing, key)) = ids {
        *listing.listed() = Some(Listed {
            key,
            copies: listed,
        });
    }
    DisplacedCopies {
        count: u32::try_from(copies.len()).unwrap_or(u32::MAX),
        copies,
        unavailable,
    }
}

/// Why a delete did nothing.
#[derive(Debug)]
pub(crate) enum DeleteError {
    /// The id is not one of the current listing of this unlocked session.
    StaleId,
    /// The unlocked key is no longer available.
    Session(SessionError),
    /// The copy no longer verifies (or the file layer refused); see the inner error.
    Refused(KdbxError),
}

/// Deletes the copy `id` names, after re-listing and re-hashing it (see the module docs).
/// `key` is the listing key of the current session.
pub(crate) fn delete(
    listing: &RetainedListing,
    key: &ListingKey,
    id: &str,
    known: &[RetainedIdentity],
    session: &VaultSession,
) -> Result<PathBuf, DeleteError> {
    let copy = {
        let guard = listing.listed();
        guard
            .as_ref()
            .filter(|listed| listed.key == *key)
            .and_then(|listed| listed.copies.iter().find(|copy| copy.id == id))
            .cloned()
            .ok_or(DeleteError::StaleId)?
    };
    // Listed and hashed again now: the same file name must still verify to the same id.
    let now = inspect_retained_copies(&copy.target, known).map_err(DeleteError::Refused)?;
    let check =
        now.copies
            .into_iter()
            .find(|c| c.path == copy.path)
            .ok_or(DeleteError::Refused(KdbxError::RetainedCopyRefused(
                mypass_kdbx::RetainedRefusal::NotRetainedCopy,
            )))?;
    if let Err(reason) = check.verdict {
        return Err(DeleteError::Refused(KdbxError::RetainedCopyRefused(reason)));
    }
    let same = check.sha256.is_some_and(|sha256| {
        listing.id(key, copy.location, &check.path, check.len, &sha256) == id
    });
    if !same {
        return Err(DeleteError::Refused(KdbxError::RetainedCopyRefused(
            mypass_kdbx::RetainedRefusal::Changed,
        )));
    }
    let removal = RetainedRemoval {
        target: copy.target,
        name: copy.name,
        len: copy.len,
        sha256: copy.sha256,
    };
    let (root, password) = session
        .with_db_and_password(|db, password| (db.root_id(), password.clone()))
        .map_err(DeleteError::Session)?;
    let io: Arc<dyn FileIo> = match copy.location {
        DisplacedLocation::Primary => session.file_io(),
        DisplacedLocation::Secondary => Arc::new(PlainFileIo),
    };
    let authenticated = io
        .authenticate_retained(removal, &password, root)
        .map_err(DeleteError::Refused)?;
    drop(password);
    if authenticated.target() != now.target {
        return Err(DeleteError::Refused(KdbxError::RetainedCopyRefused(
            mypass_kdbx::RetainedRefusal::Changed,
        )));
    }

    // The KDF can take time. Require both the listing and the unlocked incarnation to still
    // match, and fence session invalidation only for the final filesystem operation.
    let mut guard = listing.listed();
    let listed = guard
        .as_mut()
        .filter(|listed| listed.key == *key && listed.copies.iter().any(|copy| copy.id == id))
        .ok_or(DeleteError::StaleId)?;
    session
        .publish_if_current(
            key.incarnation,
            |db| db.root_id() == root,
            || io.remove_retained(&authenticated, known),
        )
        .ok_or(DeleteError::StaleId)?
        .map_err(DeleteError::Refused)?;
    listed.copies.retain(|copy| copy.id != id);
    Ok(copy.path)
}

/// The persisted write-time identities of retained copies (`<config_dir>/retained-copies.json`).
/// Kept small and bounded; a record that cannot be written is still in the in-process journal,
/// and a copy without any record falls back to the stricter unrecorded rule (never looser).
pub(crate) struct RetainedRegistry {
    path: PathBuf,
    records: Mutex<Option<Vec<RetainedIdentity>>>,
}

/// How many records are kept (the newest win).
const REGISTRY_MAX: usize = 512;

#[derive(Serialize, Deserialize)]
struct StoredRegistry {
    version: u32,
    copies: Vec<StoredIdentity>,
}

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    target: String,
    path: String,
    len: u64,
    sha256: String,
}

impl RetainedRegistry {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            records: Mutex::new(None),
        }
    }

    fn records(&self) -> MutexGuard<'_, Option<Vec<RetainedIdentity>>> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn load(&self) -> Vec<RetainedIdentity> {
        let Ok(bytes) = std::fs::read(&self.path) else {
            return Vec::new();
        };
        let Ok(stored) = serde_json::from_slice::<StoredRegistry>(&bytes) else {
            return Vec::new();
        };
        stored
            .copies
            .into_iter()
            .filter_map(|c| {
                Some(RetainedIdentity {
                    target: PathBuf::from(c.target),
                    path: PathBuf::from(c.path),
                    len: c.len,
                    sha256: unhex(&c.sha256)?,
                })
            })
            .collect()
    }

    fn save(&self, records: &[RetainedIdentity]) {
        let stored = StoredRegistry {
            version: 1,
            copies: records
                .iter()
                .filter_map(|r| {
                    Some(StoredIdentity {
                        target: r.target.to_str()?.to_owned(),
                        path: r.path.to_str()?.to_owned(),
                        len: r.len,
                        sha256: hex(&r.sha256),
                    })
                })
                .collect(),
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&stored) {
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = mypass_kdbx::write_atomic(&self.path, &bytes);
        }
    }

    /// Adds records (replacing older ones of the same path) and persists when anything changed.
    pub(crate) fn add(&self, new: &[RetainedIdentity]) {
        let mut guard = self.records();
        let records = guard.get_or_insert_with(|| self.load());
        let mut changed = false;
        for identity in new {
            if records.last() == Some(identity) || records.contains(identity) {
                continue;
            }
            records.retain(|r| r.path != identity.path);
            records.push(identity.clone());
            changed = true;
        }
        let excess = records.len().saturating_sub(REGISTRY_MAX);
        records.drain(..excess);
        if changed {
            self.save(records);
        }
    }

    /// Every persisted record plus this process's journal (which it also persists).
    pub(crate) fn all(&self) -> Vec<RetainedIdentity> {
        self.add(&recorded_retained_copies());
        self.records().clone().unwrap_or_default()
    }

    /// Drops the record of a deleted copy.
    pub(crate) fn forget(&self, path: &Path) {
        let mut guard = self.records();
        let records = guard.get_or_insert_with(|| self.load());
        let before = records.len();
        records.retain(|r| r.path != path);
        if records.len() != before {
            self.save(records);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::fs;

    fn displaced(dir: &Path, name: &str, tag: &str) -> PathBuf {
        let path = dir.join(format!(".{name}.{tag}.displaced"));
        fs::write(&path, b"competing").unwrap();
        path
    }

    #[test]
    fn targets_cover_the_primary_and_a_distinct_secondary_copy() {
        let d = tempfile::tempdir().unwrap();
        let primary = d.path().join("a").join("v.kdbx");
        let folder = d.path().join("b");
        let all = targets(Some(primary.clone()), folder.to_str());
        assert_eq!(all[0], (DisplacedLocation::Primary, primary.clone()));
        assert_eq!(
            all[1],
            (DisplacedLocation::Secondary, folder.join("v.kdbx"))
        );
        // The same folder is searched once; no vault, nothing to search.
        assert_eq!(
            targets(Some(primary.clone()), primary.parent().unwrap().to_str()).len(),
            1
        );
        assert!(targets(None, folder.to_str()).is_empty());
        assert_eq!(targets(Some(primary), None).len(), 1);
    }

    #[test]
    fn list_reports_count_location_name_and_size_and_deletes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let primary_dir = d.path().join("a");
        let secondary_dir = d.path().join("b");
        fs::create_dir_all(&primary_dir).unwrap();
        fs::create_dir_all(&secondary_dir).unwrap();
        let primary = primary_dir.join("v.kdbx");
        fs::write(&primary, b"vault").unwrap();
        fs::write(secondary_dir.join("v.kdbx"), b"copy").unwrap();
        let one = displaced(&primary_dir, "v.kdbx", "1.2.3");
        let two = displaced(&secondary_dir, "v.kdbx", "4.5.6");
        let all = targets(Some(primary), secondary_dir.to_str());
        let status = list(&all, &[], None);
        assert_eq!(status.count, 2);
        assert_eq!(status.copies[0].location, DisplacedLocation::Primary);
        assert_eq!(status.copies[0].name, ".v.kdbx.1.2.3.displaced");
        assert_eq!(status.copies[1].location, DisplacedLocation::Secondary);
        assert!((status.copies[0].bytes - 9.0).abs() < f64::EPSILON);
        assert!(status.unavailable.is_empty());
        assert!(!status.copies.iter().any(|c| c.name.contains('/')));
        assert!(one.exists() && two.exists());
    }

    #[test]
    fn retained_exchange_copies_are_listed_and_temp_candidates_are_not() {
        let d = tempfile::tempdir().unwrap();
        let primary = d.path().join("v.kdbx");
        fs::write(&primary, b"vault").unwrap();
        fs::write(d.path().join(".v.kdbx.1.2.3.exchanged"), b"previous").unwrap();
        // A temp name can hold an uninstalled candidate: never a recovery copy.
        fs::write(d.path().join(".v.kdbx.1.2.4.tmp"), b"candidate").unwrap();
        let status = list(&targets(Some(primary), None), &[], None);
        assert_eq!(status.count, 1);
        assert_eq!(status.copies[0].kind, RetainedCopyKind::Exchanged);
        assert_eq!(status.copies[0].name, ".v.kdbx.1.2.3.exchanged");
    }

    #[test]
    fn an_unreadable_location_is_unavailable_not_empty() {
        let d = tempfile::tempdir().unwrap();
        let primary = d.path().join("v.kdbx");
        fs::write(&primary, b"vault").unwrap();
        let kept = displaced(d.path(), "v.kdbx", "1.2.3");
        // The secondary "folder" is a plain file, so its directory cannot be read.
        let blocker = d.path().join("blocker");
        fs::write(&blocker, b"x").unwrap();
        let all = targets(Some(primary), blocker.to_str());
        let status = list(&all, &[], None);
        assert_eq!(status.count, 1);
        assert_eq!(status.unavailable, vec![DisplacedLocation::Secondary]);
        assert!(kept.exists());
    }

    #[test]
    fn a_missing_or_unplugged_folder_is_unavailable_not_empty() {
        let d = tempfile::tempdir().unwrap();
        let primary = d.path().join("v.kdbx");
        fs::write(&primary, b"vault").unwrap();
        // The sync folder was removed or its volume unplugged: its directory does not exist.
        let gone = d.path().join("unplugged").join("sync.kdbx");
        let all = targets(Some(primary), gone.to_str());
        let status = list(&all, &[], None);
        assert_eq!(status.count, 0);
        assert_eq!(status.unavailable, vec![DisplacedLocation::Secondary]);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_secondary_symlink_is_unavailable_not_empty() {
        // The configured secondary vault is a symlink whose target is gone (a failed install and
        // restore left the vault under a retained name): Settings must say the location cannot
        // be checked, never show "no copies".
        let d = tempfile::tempdir().unwrap();
        let primary = d.path().join("v.kdbx");
        fs::write(&primary, b"vault").unwrap();
        let real_dir = d.path().join("real");
        let link_dir = d.path().join("link");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("sync.kdbx");
        std::os::unix::fs::symlink(real_dir.join("gone.kdbx"), &link).unwrap();
        let all = targets(Some(primary), link.to_str());
        let status = list(&all, &[], None);
        assert_eq!(status.count, 0);
        assert_eq!(status.unavailable, vec![DisplacedLocation::Secondary]);
    }
}
