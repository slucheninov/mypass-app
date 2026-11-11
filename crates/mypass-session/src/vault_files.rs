//! Facts about vault files on disk. Stage 2a Task 5 lands [`VaultFileStatus`] (needed by
//! `SessionStatus`); the `.kdbx` discovery and the open-time guards join this module later.

use serde::Serialize;
use std::io::ErrorKind;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// What the session's vault file looks like right now. Sizes and instants are `f64` (bytes and
/// Unix milliseconds) because the TypeScript bindings cannot carry 64-bit integers.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VaultFileStatus {
    /// The session has no vault path.
    NoVault,
    /// The file exists. `modified` is 0 when the file system reports no modification time.
    Connected { size: f64, modified: f64 },
    /// The path no longer exists (moved, deleted, unmounted).
    Missing,
    /// The path exists but its metadata cannot be read.
    Unreadable { message: String },
}

/// Stats `path` (one `metadata` call; never reads the content).
pub fn file_status(path: Option<&Path>) -> VaultFileStatus {
    let Some(path) = path else {
        return VaultFileStatus::NoVault;
    };
    match std::fs::metadata(path) {
        Ok(meta) => VaultFileStatus::Connected {
            size: meta.len() as f64,
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0.0, |d| d.as_millis() as f64),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => VaultFileStatus::Missing,
        Err(e) => VaultFileStatus::Unreadable {
            message: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_status_reports_each_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        assert_eq!(file_status(None), VaultFileStatus::NoVault);
        assert_eq!(file_status(Some(&path)), VaultFileStatus::Missing);
        std::fs::write(&path, b"12345").unwrap();
        match file_status(Some(&path)) {
            VaultFileStatus::Connected { size, modified } => {
                assert_eq!(size, 5.0);
                assert!(modified > 0.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn serialises_with_a_kind_tag() {
        let v = serde_json::to_value(VaultFileStatus::Connected {
            size: 1.0,
            modified: 2.0,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"kind": "connected", "size": 1.0, "modified": 2.0})
        );
        assert_eq!(
            serde_json::to_value(VaultFileStatus::NoVault).unwrap(),
            serde_json::json!({"kind": "noVault"})
        );
    }
}
