//! Feature-gated failure injection for downstream dev-dependencies. Never enable in production.
use crate::{file::io_error, FileIo, FileMeta, Result};
use std::{
    io::ErrorKind,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[derive(Default)]
struct Failures {
    read: Option<ErrorKind>,
    write: Option<ErrorKind>,
    metadata: Option<ErrorKind>,
}
/// Persistent per-operation failures and counters, including failed calls.
pub struct FailingFileIo {
    inner: Arc<dyn FileIo>,
    failures: Mutex<Failures>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    metadata: AtomicUsize,
}
impl FailingFileIo {
    pub fn new(inner: Arc<dyn FileIo>) -> Self {
        Self {
            inner,
            failures: Mutex::new(Failures::default()),
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            metadata: AtomicUsize::new(0),
        }
    }
    fn failures(&self) -> std::sync::MutexGuard<'_, Failures> {
        self.failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn fail_read(&self, kind: ErrorKind) {
        self.failures().read = Some(kind);
    }
    pub fn fail_write_atomic(&self, kind: ErrorKind) {
        self.failures().write = Some(kind);
    }
    pub fn fail_metadata(&self, kind: ErrorKind) {
        self.failures().metadata = Some(kind);
    }
    pub fn heal(&self) {
        *self.failures() = Failures::default();
    }
    pub fn write_calls(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }
    pub fn read_calls(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
    pub fn metadata_calls(&self) -> usize {
        self.metadata.load(Ordering::Relaxed)
    }
}
impl FileIo for FailingFileIo {
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let failure = self.failures().read;
        if let Some(kind) = failure {
            return Err(io_error(kind.into()));
        }
        self.inner.read(path)
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        let failure = self.failures().write;
        if let Some(kind) = failure {
            return Err(io_error(kind.into()));
        }
        self.inner.write_atomic(path, bytes)
    }
    fn metadata(&self, path: &Path) -> Result<FileMeta> {
        self.metadata.fetch_add(1, Ordering::Relaxed);
        let failure = self.failures().metadata;
        if let Some(kind) = failure {
            return Err(io_error(kind.into()));
        }
        self.inner.metadata(path)
    }
}
