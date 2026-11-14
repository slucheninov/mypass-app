//! The folder watcher of the secondary sync folder: one debounced callback per burst of changes to
//! the vault file.
//!
//! `notify` uses FSEvents on macOS (directory level, coalesced by the OS) and
//! `ReadDirectoryChangesW` on Windows. Only events for the vault file name count: other files in
//! the folder and the atomic-write temp file (`.<name>.<pid>.<nanos>.tmp`) never fire on their
//! own; the rename that completes an atomic write fires through the vault name. A rescan request
//! or a watch error also fires, so a lost event degrades to an extra sync, never a missed one.
//!
//! The debounce is a pure [`Debouncer`] driven with explicit instants (tested with synthetic
//! times); the worker thread only waits on its channel with a timeout and never sleeps.
//! [`EchoFilter`] lets the owner drop the event its own upload caused.
use crate::{backend::StorageError, checkpoint::SyncCheckpoint};
use notify::{
    event::{AccessKind, AccessMode},
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::{
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex, PoisonError,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long the folder must stay quiet after the last vault-file event before the callback runs.
pub const WATCH_DEBOUNCE: Duration = Duration::from_millis(500);

/// Coalesces a burst of events into one firing `delay` after the last event. Pure: the caller
/// passes the current instant, so tests drive it with synthetic times.
#[derive(Debug, Clone)]
pub struct Debouncer {
    delay: Duration,
    deadline: Option<Instant>,
}

impl Debouncer {
    /// A quiet debouncer that fires `delay` after the last event.
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            deadline: None,
        }
    }

    /// Record an event at `now`; the firing moves to `now + delay`.
    pub fn event(&mut self, now: Instant) {
        self.deadline = Some(now.checked_add(self.delay).unwrap_or(now));
    }

    /// When the pending firing is due, if one is pending.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Whether the pending firing is due at `now`; a due firing is consumed.
    pub fn fire_due(&mut self, now: Instant) -> bool {
        match self.deadline {
            Some(deadline) if deadline <= now => {
                self.deadline = None;
                true
            }
            _ => false,
        }
    }
}

/// Remembers the content this device last wrote to the remote, so the watcher event caused by
/// that write is recognised as an echo (same length and hash) and skipped.
#[derive(Debug, Default)]
pub struct EchoFilter {
    last: Mutex<Option<SyncCheckpoint>>,
}

impl EchoFilter {
    /// An empty filter: nothing is an echo.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the content about to be written (call it before the write, so an event that
    /// arrives right after the write is already recognised).
    pub fn note_own_write(&self, cp: SyncCheckpoint) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(cp);
    }

    /// Whether `current` (the remote as it is now) is exactly what this device last wrote.
    pub fn is_echo(&self, current: &SyncCheckpoint) -> bool {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|last| last.same_content(current))
    }

    /// Forget the last write (after a disconnect or a folder change).
    pub fn clear(&self) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

enum Signal {
    Event,
    Stop,
}

/// Watches one folder for changes to one file name. Dropping it stops it, like [`Self::stop`].
pub struct FolderWatcher {
    watcher: Option<RecommendedWatcher>,
    signals: Sender<Signal>,
    worker: Option<JoinHandle<()>>,
}

impl fmt::Debug for FolderWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FolderWatcher")
            .field("running", &self.worker.is_some())
            .finish_non_exhaustive()
    }
}

impl FolderWatcher {
    /// Watch `dir` for changes to `file_name`; `on_change` runs on the watcher's worker thread at
    /// least `debounce` after the last event of a burst. It must not block for long: hand the
    /// work to a scheduler. A missing folder is `Unavailable` (nothing is created).
    pub fn start(
        dir: &Path,
        file_name: &str,
        debounce: Duration,
        on_change: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, StorageError> {
        match fs::metadata(dir) {
            Ok(meta) if meta.is_dir() => {}
            _ => {
                return Err(StorageError::Unavailable(format!(
                    "folder unavailable: {}",
                    dir.display()
                )))
            }
        }
        let (signals, inbox) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("mypass-sync-watch".into())
            .spawn(move || run_worker(&inbox, debounce, on_change.as_ref()))
            .map_err(|e| StorageError::Io(format!("watcher thread: {e}")))?;
        let mut this = Self {
            watcher: None,
            signals: signals.clone(),
            worker: Some(worker),
        };
        let filter = NameFilter {
            dir: dir.to_path_buf(),
            name: OsString::from(file_name),
        };
        let handler = move |res: notify::Result<notify::Event>| {
            if filter.is_relevant(&res) {
                // The worker is gone only while stopping; nothing to deliver then.
                let _ = signals.send(Signal::Event);
            }
        };
        // On error `this` drops here, which stops the worker.
        let mut watcher = notify::recommended_watcher(handler).map_err(watch_error)?;
        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(watch_error)?;
        this.watcher = Some(watcher);
        Ok(this)
    }

    /// Stop watching. When this returns no callback is running and none will run again; a
    /// pending (debounced) firing is dropped.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        // Drop the OS watch first so no new event arrives, then end the worker.
        drop(self.watcher.take());
        let _ = self.signals.send(Signal::Stop);
        if let Some(worker) = self.worker.take() {
            // Dropped from inside the callback: the worker ends after the callback returns.
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for FolderWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn watch_error(e: notify::Error) -> StorageError {
    match e.kind {
        notify::ErrorKind::PathNotFound => StorageError::Unavailable(format!("watch: {e}")),
        _ => StorageError::Io(format!("watch: {e}")),
    }
}

fn run_worker(inbox: &Receiver<Signal>, debounce: Duration, on_change: &(dyn Fn() + Send + Sync)) {
    let mut debouncer = Debouncer::new(debounce);
    loop {
        let next = match debouncer.deadline() {
            None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(deadline) => {
                inbox.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
        };
        match next {
            Ok(Signal::Event) => debouncer.event(Instant::now()),
            Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                if debouncer.fire_due(Instant::now()) {
                    on_change();
                }
            }
        }
    }
}

/// Decides which `notify` events concern the vault file.
struct NameFilter {
    dir: PathBuf,
    name: OsString,
}

impl NameFilter {
    fn is_relevant(&self, res: &notify::Result<notify::Event>) -> bool {
        let event = match res {
            Ok(event) => event,
            // The watch is in trouble: let the owner re-check rather than miss a change.
            Err(_) => return true,
        };
        if event.need_rescan() {
            return true;
        }
        // Reads (including our own) are not changes; a close after writing is.
        if let EventKind::Access(kind) = event.kind {
            if kind != AccessKind::Close(AccessMode::Write) {
                return false;
            }
        }
        event.paths.iter().any(|p| self.names_the_vault(p))
    }

    fn names_the_vault(&self, path: &Path) -> bool {
        match path.file_name() {
            Some(name) if name == self.name => true,
            Some(name) => self.same_file_other_spelling(name),
            None => false,
        }
    }

    /// macOS may report a non-ASCII name in another Unicode normalisation (NFD against the NFC
    /// the app holds); APFS treats both as one file. Compare the files themselves then.
    #[cfg(unix)]
    fn same_file_other_spelling(&self, name: &std::ffi::OsStr) -> bool {
        use std::os::unix::fs::MetadataExt;
        if name.is_ascii() && self.name.is_ascii() {
            return false;
        }
        match (
            fs::metadata(self.dir.join(name)),
            fs::metadata(self.dir.join(&self.name)),
        ) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }

    /// NTFS does not normalise names: a byte comparison is exact.
    #[cfg(not(unix))]
    fn same_file_other_spelling(&self, _name: &std::ffi::OsStr) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
