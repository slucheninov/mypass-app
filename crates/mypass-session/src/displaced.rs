//! Competing vault copies kept next to a vault file ("displaced" copies).
//!
//! A replacement that loses a last-instant race against another writer keeps the other writer's
//! file next to the vault as `.<name>.<pid>.<nanos>.<n>.displaced` (see
//! `mypass_kdbx::ReplaceOutcome::Displaced`). The engine never deletes one: it may be the only
//! copy of someone's edit. This module lists them for the Settings sync pane and removes them
//! only on the explicit user command `sync_displaced_delete_all`.
//!
//! The webview never names a file. Paths in the status are display-only; deletion takes no
//! argument at all and re-finds the copies next to the open vault and next to its secondary copy
//! by their exact generated names (`mypass_kdbx::remove_displaced_copies`).

use mypass_kdbx::{displaced_copies, remove_displaced_copies};
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

/// One kept competing copy. `path` is for display only.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DisplacedCopyInfo {
    pub location: DisplacedLocation,
    pub path: String,
    /// Length in bytes.
    pub bytes: f64,
}

/// The kept competing copies of the open vault.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DisplacedCopies {
    pub count: u32,
    pub copies: Vec<DisplacedCopyInfo>,
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
    let copies: Vec<DisplacedCopyInfo> = targets
        .iter()
        .flat_map(|(location, target)| {
            displaced_copies(target)
                .into_iter()
                .map(move |copy| DisplacedCopyInfo {
                    location: *location,
                    path: copy.path.display().to_string(),
                    bytes: copy.len as f64,
                })
        })
        .collect();
    DisplacedCopies {
        count: u32::try_from(copies.len()).unwrap_or(u32::MAX),
        copies,
    }
}

/// Deletes every displaced copy next to `targets`; the number that could not be removed.
pub(crate) fn delete_all(targets: &[(DisplacedLocation, PathBuf)]) -> usize {
    targets
        .iter()
        .map(|(_, target)| remove_displaced_copies(target).failed)
        .sum()
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
    fn list_reports_count_location_and_size_and_delete_all_removes_only_those() {
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
        let bystander = primary_dir.join("notes.txt");
        fs::write(&bystander, b"mine").unwrap();
        let all = targets(Some(primary.clone()), secondary_dir.to_str());
        let before = list(&all);
        assert_eq!(before.count, 2);
        assert_eq!(before.copies[0].location, DisplacedLocation::Primary);
        assert_eq!(before.copies[1].location, DisplacedLocation::Secondary);
        assert!((before.copies[0].bytes - 9.0).abs() < f64::EPSILON);
        // Listing never deletes.
        assert!(one.exists() && two.exists());
        assert_eq!(delete_all(&all), 0);
        assert_eq!(list(&all), DisplacedCopies::default());
        assert!(!one.exists() && !two.exists());
        assert!(primary.exists() && bystander.exists());
        assert!(secondary_dir.join("v.kdbx").exists());
    }
}
