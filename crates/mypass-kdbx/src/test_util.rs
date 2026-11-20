//! Feature-gated failure injection for downstream dev-dependencies. Never enable in production.
use crate::{file::io_error, FileIo, FileMeta, OpenedFile, ReplaceOutcome, Result, SaveOutcome};
use std::{
    io::ErrorKind,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

/// A structurally valid header with a checksum, an empty MAC, and no payload.
/// Use when the supplied KDF must be rejected before any key derivation.
pub fn vault_header_only(kdf: &crate::KdfParams) -> Vec<u8> {
    use crate::format::{
        header_auth,
        outer_header::{Cipher, Compression, OuterHeader},
        signature::Version,
    };
    let mut bytes = OuterHeader {
        version: Version { major: 4, minor: 0 },
        cipher: Cipher::Aes256Cbc,
        compression: Compression::None,
        master_seed: vec![0; 32],
        encryption_iv: vec![0; 16],
        kdf_parameters: kdf.to_variant(),
        public_custom_data: None,
    }
    .serialize();
    bytes.extend(header_auth::header_sha256(&bytes));
    bytes.extend([0; 32]);
    bytes
}

#[derive(Default)]
struct Failures {
    read: Option<ErrorKind>,
    write: Option<ErrorKind>,
    metadata: Option<ErrorKind>,
    strand: bool,
}
/// Test-only backend behind the `test-util` feature; never enable in production.
/// Persistent per-operation failures and counters, including failed calls.
pub struct FailingFileIo {
    inner: Arc<dyn FileIo>,
    failures: Mutex<Failures>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    metadata: AtomicUsize,
}
impl FailingFileIo {
    /// Wrap a backend with test-only failure switches and zeroed counters.
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
    /// Persistently fail reads with this error kind until `heal`.
    pub fn fail_read(&self, kind: ErrorKind) {
        self.failures().read = Some(kind);
    }
    /// Persistently fail writes before calling the backend until `heal`.
    pub fn fail_write_atomic(&self, kind: ErrorKind) {
        self.failures().write = Some(kind);
    }
    /// Persistently fail metadata calls until `heal`.
    pub fn fail_metadata(&self, kind: ErrorKind) {
        self.failures().metadata = Some(kind);
    }
    /// Fail the next conditional install and leave the previous file at its working name.
    pub fn strand_next_replace(&self) {
        self.failures().strand = true;
    }
    /// Clear test failures without resetting operation counters.
    pub fn heal(&self) {
        *self.failures() = Failures::default();
    }
    /// Return the number of attempted writes, including injected failures.
    pub fn write_calls(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }
    /// Return the number of attempted reads, including injected failures.
    pub fn read_calls(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
    /// Return the number of attempted metadata calls, including injected failures.
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
    fn read_limited(&self, path: &Path, max_len: u64) -> Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let failure = self.failures().read;
        if let Some(kind) = failure {
            return Err(io_error(kind.into()));
        }
        self.inner.read_limited(path, max_len)
    }
    fn open_read(&self, path: &Path) -> Result<OpenedFile> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let failure = self.failures().read;
        if let Some(kind) = failure {
            return Err(io_error(kind.into()));
        }
        self.inner.open_read(path)
    }
    fn replace_if(
        &self,
        path: &Path,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
        max_len: u64,
    ) -> Result<ReplaceOutcome> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        let failure = self.failures().write;
        if let Some(kind) = failure {
            return Err(io_error(kind.into()));
        }
        if let Some(expected) = expected {
            let strand = std::mem::take(&mut self.failures().strand);
            if strand {
                return crate::file::replace_if_stranded(path, expected, bytes, max_len);
            }
        }
        self.inner.replace_if(path, expected, bytes, max_len)
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        let failure = self.failures().write;
        if let Some(kind) = failure {
            return Err(io_error(kind.into()));
        }
        self.inner.write_atomic(path, bytes)
    }
    fn write_checked(&self, path: &Path, expected: &[u8; 32], bytes: &[u8]) -> Result<SaveOutcome> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        if let Some(kind) = self.failures().write {
            return Err(io_error(kind.into()));
        }
        self.inner.write_checked(path, expected, bytes)
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
