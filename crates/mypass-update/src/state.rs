//! The Rust-only update state: `<config_dir>/update-state.json` holding the dismissed banner
//! version and the time of the last attempt. It is never part of `Settings`, so no webview can
//! write it.

use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// What is persisted between runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateState {
    /// The version string of the release whose banner the user dismissed ("Later"). Compared as a
    /// string, never by order: a release newer than the dismissed one brings the banner back.
    pub dismissed_version: Option<String>,
    /// Unix seconds of the last check attempt, written after every attempt including a failed one.
    pub last_run: Option<i64>,
}

/// Why the state could not be saved. Never fatal: a lost value only repeats a check or shows a
/// dismissed banner once more.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateStateError {
    #[error("the update state could not be saved: {0}")]
    Io(String),
}

/// The store. Reads never fail: a missing, unreadable, oversized, non-object or corrupt file reads
/// as defaults, and a field of the wrong type falls back alone.
#[derive(Debug)]
pub struct UpdateStateStore {
    path: PathBuf,
    /// Serialises the read-modify-write of the file.
    write: Mutex<()>,
}

impl UpdateStateStore {
    /// The file name under `config_dir`.
    pub const FILE_NAME: &'static str = "update-state.json";
    /// A larger file is not read (the real one is under a hundred bytes).
    pub const MAX_FILE_BYTES: u64 = 64 * 1024;
    /// A longer dismissed version is not a version this app wrote and reads as absent.
    const MAX_VERSION_LEN: usize = 64;

    /// The store at `<config_dir>/update-state.json`.
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join(Self::FILE_NAME),
            write: Mutex::new(()),
        }
    }

    /// The persisted state, defaults where anything is missing or bad.
    pub fn load(&self) -> UpdateState {
        read_state(&self.path)
    }

    /// Persists the dismissed banner version.
    pub fn set_dismissed_version(&self, version: &str) -> Result<(), UpdateStateError> {
        self.update(|s| s.dismissed_version = Some(version.to_owned()))
    }

    /// Persists the time of an attempt (unix seconds).
    pub fn record_run(&self, at_unix_secs: i64) -> Result<(), UpdateStateError> {
        self.update(|s| s.last_run = Some(at_unix_secs))
    }

    fn update(&self, change: impl FnOnce(&mut UpdateState)) -> Result<(), UpdateStateError> {
        let _guard: MutexGuard<'_, ()> = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = read_state(&self.path);
        change(&mut state);
        let mut text = serde_json::to_vec_pretty(&json!({
            "dismissedVersion": state.dismissed_version,
            "lastRun": state.last_run,
        }))
        .map_err(|e| UpdateStateError::Io(e.to_string()))?;
        text.push(b'\n');
        write_replace(&self.path, &text).map_err(|e| UpdateStateError::Io(e.to_string()))
    }
}

fn read_state(path: &Path) -> UpdateState {
    let Ok(file) = fs::File::open(path) else {
        return UpdateState::default();
    };
    let mut bytes = Vec::new();
    let limit = UpdateStateStore::MAX_FILE_BYTES + 1;
    if file.take(limit).read_to_end(&mut bytes).is_err()
        || bytes.len() as u64 > UpdateStateStore::MAX_FILE_BYTES
    {
        return UpdateState::default();
    }
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) else {
        return UpdateState::default();
    };
    UpdateState {
        dismissed_version: map
            .get("dismissedVersion")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.len() <= UpdateStateStore::MAX_VERSION_LEN)
            .map(str::to_owned),
        last_run: map.get("lastRun").and_then(Value::as_i64),
    }
}

/// Replaces `path` with `bytes`: a sibling temporary file, flushed to disk, renamed over the
/// target, so a crash leaves either the old file or the new one. The state file is not secret, so
/// the default permissions apply. A failed write removes its own temporary file (this file is a
/// state file, never a vault copy).
fn write_replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, UpdateStateStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = UpdateStateStore::new(dir.path());
        (dir, store)
    }

    fn file(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join(UpdateStateStore::FILE_NAME)
    }

    #[test]
    fn a_missing_file_reads_defaults() {
        let (_dir, store) = store();
        assert_eq!(store.load(), UpdateState::default());
    }

    #[test]
    fn values_round_trip_and_survive_a_new_store() {
        let (dir, store) = store();
        store.set_dismissed_version("0.7.1").unwrap();
        store.record_run(1_800_000_000).unwrap();
        let again = UpdateStateStore::new(dir.path());
        assert_eq!(
            again.load(),
            UpdateState {
                dismissed_version: Some("0.7.1".into()),
                last_run: Some(1_800_000_000),
            }
        );
        // Each field is written without disturbing the other.
        again.record_run(1_800_086_400).unwrap();
        assert_eq!(
            again.load().dismissed_version.as_deref(),
            Some("0.7.1"),
            "recording a run keeps the dismissed version"
        );
    }

    #[test]
    fn the_file_uses_the_camel_case_keys_and_leaves_no_temp_file() {
        let (dir, store) = store();
        store.set_dismissed_version("1.2.3").unwrap();
        let text = fs::read_to_string(file(&dir)).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["dismissedVersion"], "1.2.3");
        assert!(value["lastRun"].is_null());
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("update-state.json")]);
    }

    #[test]
    fn state_store_corrupt_file_reads_defaults() {
        for junk in [
            &b"not json"[..],
            b"",
            b"[]",
            b"null",
            b"\"text\"",
            b"{\"dismissedVersion\":",
            b"\xff\xfe\x00",
        ] {
            let (dir, store) = store();
            fs::write(file(&dir), junk).unwrap();
            assert_eq!(store.load(), UpdateState::default(), "{junk:?}");
            // A corrupt file is replaced by the next write, not propagated.
            store.record_run(5).unwrap();
            assert_eq!(store.load().last_run, Some(5));
        }
    }

    #[test]
    fn a_bad_field_falls_back_alone() {
        let (dir, store) = store();
        fs::write(
            file(&dir),
            br#"{"dismissedVersion": 7, "lastRun": 1700000000}"#,
        )
        .unwrap();
        assert_eq!(
            store.load(),
            UpdateState {
                dismissed_version: None,
                last_run: Some(1_700_000_000),
            }
        );
        fs::write(
            file(&dir),
            br#"{"dismissedVersion": "0.8.0", "lastRun": "yesterday", "extra": true}"#,
        )
        .unwrap();
        assert_eq!(
            store.load(),
            UpdateState {
                dismissed_version: Some("0.8.0".into()),
                last_run: None,
            }
        );
    }

    #[test]
    fn an_oversized_file_or_version_reads_defaults() {
        let (dir, store) = store();
        let big = format!(
            "{{\"lastRun\": 1, \"pad\": \"{}\"}}",
            "x".repeat(UpdateStateStore::MAX_FILE_BYTES as usize)
        );
        fs::write(file(&dir), big).unwrap();
        assert_eq!(store.load(), UpdateState::default());
        let long = format!("{{\"dismissedVersion\": \"{}\"}}", "9".repeat(65));
        fs::write(file(&dir), long).unwrap();
        assert_eq!(store.load().dismissed_version, None);
    }

    #[test]
    fn the_config_dir_is_created_on_first_write() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        let store = UpdateStateStore::new(&nested);
        store.record_run(9).unwrap();
        assert_eq!(store.load().last_run, Some(9));
    }

    #[test]
    fn a_write_into_an_unwritable_place_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        // A regular file where the config dir should be.
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"x").unwrap();
        let store = UpdateStateStore::new(&blocker);
        assert!(matches!(store.record_run(1), Err(UpdateStateError::Io(_))));
        assert_eq!(store.load(), UpdateState::default());
    }
}
