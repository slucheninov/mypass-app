//! Retained vault copies and move-aside working files next to a vault.
//!
//! A replacement that loses a last-instant race against another writer keeps the other writer's
//! file next to the vault as `.<name>.<pid>.<nanos>.<n>.displaced` (see
//! `mypass_kdbx::ReplaceOutcome::Displaced`). MyPass never deletes one: it may be the only copy
//! of someone's edit. This module only lists them, read-only, for the Settings vault pane; the
//! user reviews them in the vault folder. A move-aside working file, or the file an atomic
//! exchange swapped out (`.exchanged`), can also remain after a failed replacement and is
//! labelled separately. Temp names are never listed: they can hold an uninstalled candidate.
//!
//! The status carries file names only (no folder), a location marker and, for a folder that
//! could not be read, an `unavailable` marker: a failed scan is never reported as "no copies".

use mypass_kdbx::{displaced_copies, DisplacedCopyKind};
use serde::Serialize;
use std::path::{Path, PathBuf};

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

#[allow(clippy::cast_precision_loss)] // a vault file is far below 2^53 bytes
pub(crate) fn list(targets: &[(DisplacedLocation, PathBuf)]) -> DisplacedCopies {
    let mut copies = Vec::new();
    let mut unavailable = Vec::new();
    for (location, target) in targets {
        match displaced_copies(target) {
            Ok(found) => copies.extend(found.into_iter().map(|copy| {
                DisplacedCopyInfo {
                    location: *location,
                    kind: match copy.kind {
                        DisplacedCopyKind::Displaced => RetainedCopyKind::Displaced,
                        DisplacedCopyKind::Aside => RetainedCopyKind::Aside,
                        DisplacedCopyKind::Exchanged => RetainedCopyKind::Exchanged,
                    },
                    name: copy
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    bytes: copy.len as f64,
                }
            })),
            Err(_) => unavailable.push(*location),
        }
    }
    DisplacedCopies {
        count: u32::try_from(copies.len()).unwrap_or(u32::MAX),
        copies,
        unavailable,
    }
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
        let status = list(&all);
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
        let status = list(&targets(Some(primary), None));
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
        let status = list(&all);
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
        let status = list(&all);
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
        let status = list(&all);
        assert_eq!(status.count, 0);
        assert_eq!(status.unavailable, vec![DisplacedLocation::Secondary]);
    }
}
