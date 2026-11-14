// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! `FileIo::read_limited`: the bounded, regular-file-only read for vault files that other software
//! writes (sync folders, iCloud Drive, migration targets).
use mypass_kdbx::{
    test_util::FailingFileIo, FileIo, FileMeta, KdbxError, PlainFileIo, Result,
    MAX_VAULT_FILE_BYTES,
};
use std::{
    fs,
    io::ErrorKind,
    path::Path,
    sync::{Arc, Mutex},
};

fn kind<T: std::fmt::Debug>(r: Result<T>) -> ErrorKind {
    match r {
        Err(KdbxError::Io { kind, .. }) => kind,
        other => panic!("expected an i/o error, got {other:?}"),
    }
}

#[test]
fn max_vault_file_bytes_is_at_least_the_decompressed_cap() {
    // The parsers accept up to 1 GiB of decompressed payload; the file cap must not be tighter.
    const { assert!(MAX_VAULT_FILE_BYTES >= 1 << 30) };
}

#[test]
fn plain_read_limited_returns_a_regular_file_at_the_limit() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("vault.kdbx");
    fs::write(&p, b"0123456789").unwrap();
    assert_eq!(PlainFileIo.read_limited(&p, 10).unwrap(), b"0123456789");
}

#[test]
fn plain_read_limited_refuses_a_file_over_the_limit() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("vault.kdbx");
    fs::write(&p, b"0123456789A").unwrap();
    assert_eq!(
        kind(PlainFileIo.read_limited(&p, 10)),
        ErrorKind::FileTooLarge
    );
}

#[test]
fn plain_read_limited_refuses_a_directory() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(
        kind(PlainFileIo.read_limited(d.path(), 1 << 20)),
        ErrorKind::InvalidInput
    );
}

#[test]
fn plain_read_limited_missing_file_is_not_found() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(
        kind(PlainFileIo.read_limited(&d.path().join("absent.kdbx"), 1 << 20)),
        ErrorKind::NotFound
    );
}

#[cfg(unix)]
#[test]
fn plain_read_limited_refuses_an_endless_device_file() {
    // `/dev/zero` reports length 0 and never ends: only a bounded, type-checked read is safe.
    assert_eq!(
        kind(PlainFileIo.read_limited(Path::new("/dev/zero"), 1 << 20)),
        ErrorKind::InvalidInput
    );
}

#[cfg(unix)]
#[test]
fn plain_read_limited_follows_a_symlink_to_a_regular_file() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("real.kdbx");
    let link = d.path().join("vault.kdbx");
    fs::write(&target, b"bytes").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(PlainFileIo.read_limited(&link, 1 << 20).unwrap(), b"bytes");
}

/// A backend with only the three required methods, so `read_limited` is the trait default.
struct DefaultOnly {
    events: Mutex<Vec<&'static str>>,
}
impl FileIo for DefaultOnly {
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        self.events.lock().unwrap().push("read");
        PlainFileIo.read(path)
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        PlainFileIo.write_atomic(path, bytes)
    }
    fn metadata(&self, path: &Path) -> Result<FileMeta> {
        self.events.lock().unwrap().push("metadata");
        PlainFileIo.metadata(path)
    }
}

#[test]
fn default_read_limited_refuses_by_reported_length_before_reading() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("vault.kdbx");
    fs::write(&p, b"0123456789A").unwrap();
    let io = DefaultOnly {
        events: Mutex::new(vec![]),
    };
    assert_eq!(kind(io.read_limited(&p, 10)), ErrorKind::FileTooLarge);
    assert_eq!(*io.events.lock().unwrap(), ["metadata"]);
    assert_eq!(io.read_limited(&p, 11).unwrap(), b"0123456789A");
}

#[test]
fn failing_file_io_injects_and_forwards_read_limited() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("vault.kdbx");
    fs::write(&p, b"0123456789A").unwrap();
    let io = FailingFileIo::new(Arc::new(PlainFileIo));
    io.fail_read(ErrorKind::TimedOut);
    assert_eq!(kind(io.read_limited(&p, 100)), ErrorKind::TimedOut);
    io.heal();
    // Forwarded to the inner backend's bounded read, which also refuses non-regular files.
    assert_eq!(kind(io.read_limited(&p, 10)), ErrorKind::FileTooLarge);
    assert_eq!(
        kind(io.read_limited(d.path(), 100)),
        ErrorKind::InvalidInput
    );
    assert_eq!(io.read_limited(&p, 100).unwrap(), b"0123456789A");
    assert_eq!(io.read_calls(), 4);
}
