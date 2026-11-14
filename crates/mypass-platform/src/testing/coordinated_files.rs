//! [`FakeCoordinatedFiles`]: a scriptable [`CoordinatedFiles`] over real (temporary) files.
//!
//! Reads and writes run the accessor on the real path, so a test sees real bytes. On top of that a
//! test can make a path a placeholder that turns current only after a download request and a
//! number of polls (or never), inject one failure per operation (for example
//! `ErrorKind::TimedOut`), add conflict versions, and fire the presenter callbacks.

use super::lock;
use crate::coordinated::{
    Accessor, ChangeCallback, ConflictVersion, CoordinatedFiles, DownloadState,
};
use crate::Subscription;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The operations a failure can be injected into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoordinatedOp {
    Read,
    Write,
    DownloadState,
    RequestDownload,
    UnresolvedConflicts,
    MarkResolved,
    Present,
}

/// How a placeholder becomes current.
#[derive(Debug, Clone, Copy)]
struct Placeholder {
    requested: bool,
    /// Polls of `download_state` after the request until it reports `Current`; `None` = never.
    polls_left: Option<usize>,
    current: bool,
}

#[derive(Default)]
struct State {
    placeholders: HashMap<PathBuf, Placeholder>,
    ubiquitous: HashMap<PathBuf, DownloadState>,
    failures: HashMap<CoordinatedOp, io::ErrorKind>,
    conflicts: HashMap<PathBuf, Vec<ConflictVersion>>,
    resolved: Vec<ConflictVersion>,
    presenters: Vec<(u64, PathBuf, ChangeCallback)>,
    next_presenter: u64,
    counts: HashMap<CoordinatedOp, usize>,
    icloud_available: bool,
}

/// A scriptable [`CoordinatedFiles`]; see the module docs.
#[derive(Clone, Default)]
pub struct FakeCoordinatedFiles {
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for FakeCoordinatedFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeCoordinatedFiles")
            .finish_non_exhaustive()
    }
}

impl FakeCoordinatedFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `path` a placeholder: `NotDownloaded` until a download is requested, then `Current`
    /// after `polls_after_request` further polls (`None`: it never arrives).
    pub fn set_placeholder(&self, path: &Path, polls_after_request: Option<usize>) {
        lock(&self.state).placeholders.insert(
            path.to_path_buf(),
            Placeholder {
                requested: false,
                polls_left: polls_after_request,
                current: false,
            },
        );
    }

    /// Reports `state` for `path` (a downloaded ubiquitous item, say), unless it is a placeholder.
    pub fn set_download_state(&self, path: &Path, state: DownloadState) {
        lock(&self.state)
            .ubiquitous
            .insert(path.to_path_buf(), state);
    }

    /// The next call of `op` fails with `kind` (once).
    pub fn fail_next(&self, op: CoordinatedOp, kind: io::ErrorKind) {
        lock(&self.state).failures.insert(op, kind);
    }

    /// Adds an unresolved conflict version of `path`.
    pub fn add_conflict(&self, path: &Path, version: ConflictVersion) {
        lock(&self.state)
            .conflicts
            .entry(path.to_path_buf())
            .or_default()
            .push(version);
    }

    /// The versions marked resolved so far, in order.
    pub fn resolved(&self) -> Vec<ConflictVersion> {
        lock(&self.state).resolved.clone()
    }

    pub fn set_icloud_available(&self, available: bool) {
        lock(&self.state).icloud_available = available;
    }

    /// Fires every presenter of `path`, as another process's write would.
    pub fn emit_change(&self, path: &Path) {
        let callbacks: Vec<ChangeCallback> = lock(&self.state)
            .presenters
            .iter()
            .filter(|(_, p, _)| p == path)
            .map(|(_, _, cb)| Arc::clone(cb))
            .collect();
        for callback in callbacks {
            callback();
        }
    }

    /// Live presenters (all paths).
    pub fn presenter_count(&self) -> usize {
        lock(&self.state).presenters.len()
    }

    /// How many times `op` was called (failed calls included).
    pub fn count(&self, op: CoordinatedOp) -> usize {
        lock(&self.state).counts.get(&op).copied().unwrap_or(0)
    }

    /// Counts the call and takes an injected failure, if any.
    fn enter(&self, op: CoordinatedOp) -> io::Result<()> {
        let mut state = lock(&self.state);
        *state.counts.entry(op).or_default() += 1;
        match state.failures.remove(&op) {
            Some(kind) => Err(io::Error::new(kind, format!("injected {op:?} failure"))),
            None => Ok(()),
        }
    }
}

impl CoordinatedFiles for FakeCoordinatedFiles {
    fn coordinated_read(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()> {
        self.enter(CoordinatedOp::Read)?;
        accessor(path)
    }

    fn coordinated_write(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()> {
        self.enter(CoordinatedOp::Write)?;
        accessor(path)
    }

    fn download_state(&self, path: &Path) -> io::Result<DownloadState> {
        self.enter(CoordinatedOp::DownloadState)?;
        let mut state = lock(&self.state);
        if let Some(placeholder) = state.placeholders.get_mut(path) {
            if !placeholder.current && placeholder.requested {
                match placeholder.polls_left {
                    Some(0) => placeholder.current = true,
                    Some(n) => placeholder.polls_left = Some(n - 1),
                    None => {}
                }
            }
            return Ok(if placeholder.current {
                DownloadState::Current
            } else {
                DownloadState::NotDownloaded
            });
        }
        if let Some(fixed) = state.ubiquitous.get(path) {
            return Ok(*fixed);
        }
        drop(state);
        std::fs::metadata(path).map(|_| DownloadState::NotUbiquitous)
    }

    fn request_download(&self, path: &Path) -> io::Result<()> {
        self.enter(CoordinatedOp::RequestDownload)?;
        if let Some(placeholder) = lock(&self.state).placeholders.get_mut(path) {
            placeholder.requested = true;
        }
        Ok(())
    }

    fn unresolved_conflicts(&self, path: &Path) -> io::Result<Vec<ConflictVersion>> {
        self.enter(CoordinatedOp::UnresolvedConflicts)?;
        Ok(lock(&self.state)
            .conflicts
            .get(path)
            .cloned()
            .unwrap_or_default())
    }

    fn mark_conflicts_resolved(&self, path: &Path, versions: &[ConflictVersion]) -> io::Result<()> {
        self.enter(CoordinatedOp::MarkResolved)?;
        let mut state = lock(&self.state);
        let mut newly_resolved = Vec::new();
        if let Some(current) = state.conflicts.get_mut(path) {
            current.retain(|v| {
                let resolve = versions.iter().any(|r| r.path == v.path);
                if resolve {
                    newly_resolved.push(v.clone());
                }
                !resolve
            });
        }
        state.resolved.extend(newly_resolved);
        Ok(())
    }

    fn icloud_available(&self) -> bool {
        lock(&self.state).icloud_available
    }

    fn present(&self, path: &Path, on_change: ChangeCallback) -> io::Result<Subscription> {
        self.enter(CoordinatedOp::Present)?;
        let id = {
            let mut state = lock(&self.state);
            let id = state.next_presenter;
            state.next_presenter += 1;
            state.presenters.push((id, path.to_path_buf(), on_change));
            id
        };
        let weak = Arc::downgrade(&self.state);
        Ok(Subscription::new(move || {
            if let Some(state) = weak.upgrade() {
                lock(&state).presenters.retain(|(i, _, _)| *i != id);
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn version(name: &str) -> ConflictVersion {
        ConflictVersion {
            path: PathBuf::from(name),
            modified: None,
            saving_computer: Some("Other Mac".into()),
        }
    }

    #[test]
    fn placeholder_turns_current_only_after_a_request_and_the_polls() {
        let fake = FakeCoordinatedFiles::new();
        let path = Path::new("/vault.kdbx");
        fake.set_placeholder(path, Some(2));
        assert_eq!(
            fake.download_state(path).unwrap(),
            DownloadState::NotDownloaded
        );
        assert_eq!(
            fake.download_state(path).unwrap(),
            DownloadState::NotDownloaded
        );
        fake.request_download(path).unwrap();
        assert_eq!(
            fake.download_state(path).unwrap(),
            DownloadState::NotDownloaded
        );
        assert_eq!(
            fake.download_state(path).unwrap(),
            DownloadState::NotDownloaded
        );
        assert_eq!(fake.download_state(path).unwrap(), DownloadState::Current);
        assert_eq!(fake.count(CoordinatedOp::RequestDownload), 1);
    }

    #[test]
    fn placeholder_that_never_arrives_stays_not_downloaded() {
        let fake = FakeCoordinatedFiles::new();
        let path = Path::new("/vault.kdbx");
        fake.set_placeholder(path, None);
        fake.request_download(path).unwrap();
        for _ in 0..10 {
            assert_eq!(
                fake.download_state(path).unwrap(),
                DownloadState::NotDownloaded
            );
        }
    }

    #[test]
    fn injected_failure_hits_once() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("v");
        std::fs::write(&path, b"x").unwrap();
        let fake = FakeCoordinatedFiles::new();
        fake.fail_next(CoordinatedOp::Read, io::ErrorKind::TimedOut);
        let err = fake.coordinated_read(&path, &mut |_| Ok(())).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        fake.coordinated_read(&path, &mut |_| Ok(())).unwrap();
        assert_eq!(fake.count(CoordinatedOp::Read), 2);
        assert_eq!(
            fake.download_state(&path).unwrap(),
            DownloadState::NotUbiquitous
        );
    }

    #[test]
    fn only_the_listed_conflicts_are_resolved() {
        let fake = FakeCoordinatedFiles::new();
        let path = Path::new("/vault.kdbx");
        fake.add_conflict(path, version("a"));
        let listed = fake.unresolved_conflicts(path).unwrap();
        fake.add_conflict(path, version("b"));
        fake.mark_conflicts_resolved(path, &listed).unwrap();
        assert_eq!(fake.resolved(), vec![version("a")]);
        assert_eq!(fake.unresolved_conflicts(path).unwrap(), vec![version("b")]);
    }

    #[test]
    fn presenters_fire_for_their_path_until_dropped() {
        let fake = FakeCoordinatedFiles::new();
        let path = Path::new("/vault.kdbx");
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        let subscription = fake
            .present(
                path,
                Arc::new(move || {
                    h.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        assert_eq!(fake.presenter_count(), 1);
        fake.emit_change(Path::new("/other.kdbx"));
        fake.emit_change(path);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        drop(subscription);
        assert_eq!(fake.presenter_count(), 0);
        fake.emit_change(path);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
