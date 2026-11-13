//! A hardened same-user file store: the host-shared [`SecretStore`] (`Platform.shared_secrets`).
//!
//! It holds exactly one record in production, the per-launch bridge endpoint (port, token, pid),
//! which the app writes and the native messaging host reads. Pairing hashes never live here: they
//! stay in the app-only Keychain store, because integrity matters for them and a file cannot
//! protect it against a same-user process.
//!
//! One file per item, named `hex(sha256(service \0 account)).secret`, in one directory. Rules:
//!
//! - Writes create the directory with mode 0700 and the temp file with mode 0600 at open (never a
//!   chmod after the write, so there is no window with wider access), write, `sync_all`, rename onto
//!   the final name, then sync the directory best-effort.
//! - Reads refuse (an `Err`, never `Ok(None)`) when the directory or the file is a symlink, is not
//!   a plain directory or file, has any group or other permission bit, or is owned by a user other
//!   than the owner of the home folder. A missing directory or file reads as `Ok(None)`, and `get`
//!   never creates anything: the host is read-only.
//! - Errors carry paths and error kinds, never contents; `Debug` shows the directory only.
//!
//! Off Unix the permission, owner and symlink-mode checks do not exist (Windows relies on the
//! per-user ACL the data folder inherits; DPAPI replaces this store in stage 6), but the atomic
//! write and the symlink refusal stay.

use crate::{PlatformError, PlatformResult, SecretStore};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
use zeroize::Zeroizing;

/// File name extension of one item.
const EXTENSION: &str = "secret";
/// How many temp names `set` tries before giving up.
const MAX_TEMP_ATTEMPTS: usize = 8;
/// The largest item `get` reads. The one production record is a few dozen bytes.
const MAX_ITEM_BYTES: u64 = 64 * 1024;
/// Prefix and suffix of the temp files this store creates.
const TEMP_PREFIX: &str = ".tmp-";
const TEMP_SUFFIX: &str = ".partial";
/// A temp file older than this is a leftover of a crashed write and is swept.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60);

/// Distinguishes the temp files of concurrent writers in one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A [`SecretStore`] that keeps one 0600 file per item in a 0700 directory.
pub struct FileSecretStore {
    dir: PathBuf,
    /// The home folder; its owner is the only owner `get` accepts.
    home: PathBuf,
}

impl std::fmt::Debug for FileSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSecretStore")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl FileSecretStore {
    /// A store in `dir`; `home` is the user's home folder (the owner reference of the checks).
    pub fn new(dir: PathBuf, home: PathBuf) -> Self {
        Self { dir, home }
    }

    fn item_path(&self, service: &str, account: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(service.as_bytes());
        hasher.update([0u8]);
        hasher.update(account.as_bytes());
        let digest = hasher.finalize();
        let mut name = String::with_capacity(digest.len() * 2 + EXTENSION.len() + 1);
        for byte in digest {
            name.push(hex_digit(byte >> 4));
            name.push(hex_digit(byte & 0x0f));
        }
        name.push('.');
        name.push_str(EXTENSION);
        self.dir.join(name)
    }

    fn temp_path(&self) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.dir.join(format!(
            "{TEMP_PREFIX}{}-{n}{TEMP_SUFFIX}",
            std::process::id()
        ))
    }

    /// Creates the directory (0700 on Unix) when missing, then checks it.
    fn ensure_dir(&self) -> PlatformResult<()> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&self.dir)
            .map_err(|e| io_error("create directory", &self.dir, &e))?;
        self.check_dir()?.ok_or_else(|| {
            PlatformError::Failed(format!("secret directory {} vanished", self.dir.display()))
        })
    }

    /// `Ok(Some(()))` for a sound directory, `Ok(None)` when it does not exist.
    fn check_dir(&self) -> PlatformResult<Option<()>> {
        let meta = match fs::symlink_metadata(&self.dir) {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error("inspect directory", &self.dir, &e)),
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(refused(&self.dir, "not a plain directory"));
        }
        self.check_trust(&meta, &self.dir)?;
        Ok(Some(()))
    }

    /// Mode and owner checks (Unix only).
    fn check_trust(&self, meta: &fs::Metadata, path: &Path) -> PlatformResult<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.mode() & 0o077 != 0 {
                return Err(refused(path, "group or other permission bits are set"));
            }
            let home = fs::metadata(&self.home)
                .map_err(|e| io_error("inspect home folder", &self.home, &e))?;
            if meta.uid() != home.uid() {
                return Err(refused(path, "owned by another user"));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (meta, path, &self.home);
        }
        Ok(())
    }

    fn create_temp(&self) -> PlatformResult<(File, PathBuf)> {
        for _ in 0..MAX_TEMP_ATTEMPTS {
            let path = self.temp_path();
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&path) {
                Ok(file) => return Ok((file, path)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io_error("create temp file", &path, &e)),
            }
        }
        Err(PlatformError::Failed(format!(
            "no free temp file name in {}",
            self.dir.display()
        )))
    }

    /// Removes temp files this store left behind (crashed writes), best effort.
    fn sweep_stale_temps(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX)) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let old = meta
                .modified()
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .is_some_and(|age| age > STALE_TEMP_AGE);
            if meta.is_file() && old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// The store `Platform.shared_secrets` uses: `AppPaths::secrets_shared`, owner checked against the
/// home folder.
pub fn shared_store(paths: &crate::AppPaths) -> FileSecretStore {
    FileSecretStore::new(paths.secrets_shared.clone(), paths.home().to_path_buf())
}

impl SecretStore for FileSecretStore {
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
        self.ensure_dir()?;
        self.sweep_stale_temps();
        let target = self.item_path(service, account);
        let (mut file, temp) = self.create_temp()?;
        let written = file
            .write_all(secret)
            .and_then(|()| file.sync_all())
            .map_err(|e| io_error("write temp file", &temp, &e));
        drop(file);
        let result = written.and_then(|()| {
            fs::rename(&temp, &target).map_err(|e| io_error("rename temp file", &target, &e))
        });
        if result.is_err() {
            // Best effort: never leave a partial secret behind.
            let _ = fs::remove_file(&temp);
            return result;
        }
        // Best-effort durability of the rename.
        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>> {
        if self.check_dir()?.is_none() {
            return Ok(None);
        }
        let path = self.item_path(service, account);
        let link_meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error("inspect file", &path, &e)),
        };
        if link_meta.file_type().is_symlink() || !link_meta.is_file() {
            return Err(refused(&path, "not a regular file"));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        // Refuse a symlink swapped in after the check above, and never block on a FIFO.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::custom_flags(
            &mut options,
            libc::O_NOFOLLOW | libc::O_NONBLOCK,
        );
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error("open file", &path, &e)),
        };
        // Checked on the open handle, so a swap after the checks above cannot matter.
        let meta = file
            .metadata()
            .map_err(|e| io_error("inspect open file", &path, &e))?;
        if !meta.is_file() {
            return Err(refused(&path, "not a regular file"));
        }
        self.check_trust(&meta, &path)?;
        if meta.len() > MAX_ITEM_BYTES {
            return Err(refused(&path, "larger than the item size limit"));
        }
        let capacity = usize::try_from(meta.len())
            .map_err(|_| refused(&path, "larger than the item size limit"))?;
        // Pre-sized from the metadata so the buffer is never reallocated (a realloc would leave an
        // unzeroed copy behind); `take` bounds a file that grows after the check.
        let mut bytes = Zeroizing::new(Vec::with_capacity(capacity));
        std::io::Read::read_to_end(&mut std::io::Read::take(&mut file, meta.len()), &mut bytes)
            .map_err(|e| io_error("read file", &path, &e))?;
        Ok(Some(std::mem::take(&mut bytes)))
    }

    fn delete(&self, service: &str, account: &str) -> PlatformResult<()> {
        if self.check_dir()?.is_none() {
            return Ok(());
        }
        let path = self.item_path(service, account);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_error("delete file", &path, &e)),
        }
    }
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).unwrap_or('0')
}

fn io_error(action: &str, path: &Path, e: &std::io::Error) -> PlatformError {
    PlatformError::Failed(format!("{action} {}: {:?}", path.display(), e.kind()))
}

fn refused(path: &Path, why: &str) -> PlatformError {
    PlatformError::Failed(format!("refusing {}: {why}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: &Path) -> FileSecretStore {
        FileSecretStore::new(root.join("secrets").join("shared"), root.to_path_buf())
    }

    #[test]
    fn round_trip_set_get_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.set("svc", "acct", b"s3cret").unwrap();
        assert_eq!(s.get("svc", "acct").unwrap(), Some(b"s3cret".to_vec()));
        s.delete("svc", "acct").unwrap();
        assert_eq!(s.get("svc", "acct").unwrap(), None);
        s.delete("svc", "acct").unwrap();
    }

    #[test]
    fn overwrite_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.set("svc", "acct", b"one").unwrap();
        s.set("svc", "acct", b"two-longer").unwrap();
        assert_eq!(s.get("svc", "acct").unwrap(), Some(b"two-longer".to_vec()));
        s.set("svc", "acct", b"x").unwrap();
        assert_eq!(s.get("svc", "acct").unwrap(), Some(b"x".to_vec()));
    }

    #[test]
    fn distinct_service_account_pairs_do_not_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.set("a", "bc", b"first").unwrap();
        s.set("ab", "c", b"second").unwrap();
        assert_eq!(s.get("a", "bc").unwrap(), Some(b"first".to_vec()));
        assert_eq!(s.get("ab", "c").unwrap(), Some(b"second".to_vec()));
    }

    #[test]
    fn no_temp_files_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.set("svc", "a", b"1").unwrap();
        s.set("svc", "a", b"2").unwrap();
        s.set("svc", "b", b"3").unwrap();
        let names: Vec<String> = fs::read_dir(tmp.path().join("secrets").join("shared"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().all(|n| n.ends_with(".secret")), "{names:?}");
    }

    #[test]
    fn get_on_missing_dir_is_none_and_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        assert_eq!(s.get("svc", "acct").unwrap(), None);
        s.delete("svc", "acct").unwrap();
        assert!(!tmp.path().join("secrets").exists());
    }

    #[test]
    fn debug_shows_the_directory_only() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.set("svc", "acct", b"topsecret").unwrap();
        let text = format!("{s:?}");
        assert!(text.contains("FileSecretStore"));
        assert!(!text.contains("topsecret"));
    }

    #[test]
    fn oversized_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        let big = vec![7u8; usize::try_from(MAX_ITEM_BYTES).unwrap() + 1];
        s.set("svc", "acct", &big).unwrap();
        assert!(matches!(
            s.get("svc", "acct"),
            Err(PlatformError::Failed(m)) if m.starts_with("refusing")
        ));
        let ok = vec![7u8; usize::try_from(MAX_ITEM_BYTES).unwrap()];
        s.set("svc", "acct", &ok).unwrap();
        assert_eq!(
            s.get("svc", "acct").unwrap().map(|v| v.len()),
            Some(ok.len())
        );
    }

    #[test]
    fn stale_partial_files_are_swept_and_fresh_ones_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.set("svc", "acct", b"1").unwrap();
        let stale = s.dir.join(".tmp-1-1.partial");
        let fresh = s.dir.join(".tmp-1-2.partial");
        let other = s.dir.join("unrelated.txt");
        for f in [&stale, &fresh, &other] {
            fs::write(f, b"x").unwrap();
        }
        let old = SystemTime::now() - STALE_TEMP_AGE * 2;
        File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old)
            .unwrap();
        s.set("svc", "acct", b"2").unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists());
        assert!(other.exists());
    }

    #[cfg(unix)]
    mod unix {
        use super::*;

        /// The refusal error: `Failed` whose text starts with `refusing`.
        fn assert_refused<T: std::fmt::Debug>(r: PlatformResult<T>) {
            match r {
                Err(PlatformError::Failed(msg)) => {
                    assert!(msg.starts_with("refusing"), "{msg}");
                }
                other => panic!("expected a refusal, got {other:?}"),
            }
        }

        use std::os::unix::fs::{symlink, PermissionsExt};

        fn mode(path: &Path) -> u32 {
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        fn item_file(s: &FileSecretStore) -> PathBuf {
            s.item_path("svc", "acct")
        }

        #[test]
        fn dir_is_0700_and_file_is_0600_after_set() {
            let tmp = tempfile::tempdir().unwrap();
            let s = store(tmp.path());
            s.set("svc", "acct", b"s3cret").unwrap();
            assert_eq!(mode(&s.dir), 0o700);
            assert_eq!(mode(&item_file(&s)), 0o600);
        }

        #[test]
        fn file_mode_is_0600_on_overwrite_too() {
            let tmp = tempfile::tempdir().unwrap();
            let s = store(tmp.path());
            s.set("svc", "acct", b"one").unwrap();
            s.set("svc", "acct", b"two").unwrap();
            assert_eq!(mode(&item_file(&s)), 0o600);
        }

        #[test]
        fn get_refuses_symlinked_file_and_symlinked_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let s = store(tmp.path());
            s.set("svc", "acct", b"s3cret").unwrap();

            // Symlinked file.
            let real = tmp.path().join("elsewhere");
            fs::write(&real, b"planted").unwrap();
            fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
            let file = item_file(&s);
            fs::remove_file(&file).unwrap();
            symlink(&real, &file).unwrap();
            assert_refused(s.get("svc", "acct"));

            // Symlinked directory.
            let moved = tmp.path().join("moved");
            fs::rename(&s.dir, &moved).unwrap();
            symlink(&moved, &s.dir).unwrap();
            assert_refused(s.get("svc", "acct"));
            assert_refused(s.set("svc", "acct", b"x"));
        }

        #[test]
        fn get_refuses_group_or_world_readable_file() {
            let tmp = tempfile::tempdir().unwrap();
            let s = store(tmp.path());
            s.set("svc", "acct", b"s3cret").unwrap();
            let file = item_file(&s);
            for bad in [0o640, 0o604, 0o644] {
                fs::set_permissions(&file, fs::Permissions::from_mode(bad)).unwrap();
                assert_refused(s.get("svc", "acct"));
            }
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(s.get("svc", "acct").unwrap().is_some());
        }

        #[test]
        fn get_refuses_group_or_world_accessible_directory() {
            let tmp = tempfile::tempdir().unwrap();
            let s = store(tmp.path());
            s.set("svc", "acct", b"s3cret").unwrap();
            fs::set_permissions(&s.dir, fs::Permissions::from_mode(0o750)).unwrap();
            assert_refused(s.get("svc", "acct"));
            assert_refused(s.set("svc", "acct", b"x"));
        }
    }
}
