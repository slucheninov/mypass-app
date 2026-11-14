// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! `FileIo::read_limited`: the bounded, regular-file-only read for vault files that other software
//! writes (sync folders, iCloud Drive, migration targets).
use mypass_kdbx::{
    test_util::FailingFileIo, FileIo, FileMeta, KdbxError, OpenedFile, PlainFileIo, Result,
    MAX_VAULT_FILE_BYTES,
};
use std::{
    fs,
    io::{self, ErrorKind, Read},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
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
fn default_read_limited_fails_closed_without_a_bounded_open() {
    // A backend that never implemented `open_read` must not serve untrusted reads through its
    // unbounded `read` (a FIFO or `/dev/zero` behind it would hang or exhaust memory).
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("vault.kdbx");
    fs::write(&p, b"0123456789A").unwrap();
    let io = DefaultOnly {
        events: Mutex::new(vec![]),
    };
    assert_eq!(kind(io.read_limited(&p, 100)), ErrorKind::Unsupported);
    assert_eq!(
        kind(io.replace_if(&p, None, b"x", 100)),
        ErrorKind::Unsupported
    );
    assert!(io.events.lock().unwrap().is_empty(), "never read or stat");
    assert_eq!(fs::read(&p).unwrap(), b"0123456789A");
}

/// An endless reader that counts the bytes handed out.
struct Endless(Arc<AtomicU64>);
impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        buf.fill(7);
        self.0.fetch_add(buf.len() as u64, Ordering::SeqCst);
        Ok(buf.len())
    }
}

/// A backend whose `open_read` hands out a handle that lies about its length.
struct Lying {
    regular: bool,
    served: Arc<AtomicU64>,
}
impl FileIo for Lying {
    fn read(&self, _: &Path) -> Result<Vec<u8>> {
        panic!("the unbounded read is never used");
    }
    fn write_atomic(&self, _: &Path, _: &[u8]) -> Result<()> {
        Ok(())
    }
    fn metadata(&self, _: &Path) -> Result<FileMeta> {
        panic!("the path is never trusted");
    }
    fn open_read(&self, _: &Path) -> Result<OpenedFile> {
        Ok(OpenedFile::new(
            Box::new(Endless(self.served.clone())),
            self.regular,
            0,
        ))
    }
}

#[test]
fn default_read_limited_bounds_the_read_whatever_the_handle_reports() {
    let served = Arc::new(AtomicU64::new(0));
    let io = Lying {
        regular: true,
        served: served.clone(),
    };
    assert_eq!(
        kind(io.read_limited(Path::new("growing"), 1000)),
        ErrorKind::FileTooLarge
    );
    // `take(limit + 1)` caps what is pulled from the reader (one buffer of slack at most).
    assert!(served.load(Ordering::SeqCst) <= 1001 + 64 * 1024);
}

#[test]
fn default_read_limited_refuses_a_non_regular_handle_before_reading() {
    let served = Arc::new(AtomicU64::new(0));
    let io = Lying {
        regular: false,
        served: served.clone(),
    };
    assert_eq!(
        kind(io.read_limited(Path::new("device"), 1000)),
        ErrorKind::InvalidInput
    );
    assert_eq!(served.load(Ordering::SeqCst), 0);
}

/// Runs `f` on a helper thread; a hang (the bug) fails the test instead of blocking it forever.
fn bounded<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(20))
        .expect("the bounded read blocked (deadlock bound, not timing)")
}

#[cfg(unix)]
#[test]
fn plain_read_limited_refuses_a_fifo_without_blocking_on_the_open() {
    // There is no path check before the open any more: the open itself is nonblocking and the
    // type check runs on the opened handle, so a path swapped for a FIFO at any moment (here:
    // one that is a FIFO from the start, the worst case of that race) is refused at once.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("vault.kdbx");
    let status = std::process::Command::new("mkfifo")
        .arg(&p)
        .status()
        .unwrap();
    assert!(status.success());
    let path = p.clone();
    let result = bounded(move || PlainFileIo.read_limited(&path, 1 << 20));
    assert_eq!(kind(result), ErrorKind::InvalidInput);
    let path = p.clone();
    let result = bounded(move || PlainFileIo.replace_if(&path, Some(&[0; 32]), b"new", 1 << 20));
    assert_eq!(kind(result), ErrorKind::InvalidInput);
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
