//! Notes that a browser-extension request wanted an unlock, so the unlock completion path can
//! answer it (K13). A note is fresh for [`UnlockRequestState::FRESHNESS`] and is consumed at most
//! once.

use crate::scheduler::{lock, Scheduler};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// The last unlock request noted by the bridge (wall time of the note). Cheap to clone; clones
/// share the note.
#[derive(Clone)]
pub struct UnlockRequestState {
    noted: Arc<Mutex<Option<SystemTime>>>,
    scheduler: Arc<dyn Scheduler>,
}

impl std::fmt::Debug for UnlockRequestState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockRequestState")
            .field("noted", &*lock(&self.noted))
            .finish_non_exhaustive()
    }
}

impl UnlockRequestState {
    /// How long a note stays consumable.
    pub const FRESHNESS: Duration = Duration::from_secs(10);

    /// No note yet; times come from `scheduler.wall_now()`.
    pub fn new(scheduler: Arc<dyn Scheduler>) -> Self {
        Self {
            noted: Arc::new(Mutex::new(None)),
            scheduler,
        }
    }

    /// Records a request now (a second note refreshes the window).
    pub fn note(&self) {
        *lock(&self.noted) = Some(self.scheduler.wall_now());
    }

    /// True at most once per note, and only within [`Self::FRESHNESS`] of it. Always forgets the
    /// note. A note dated in the future (the wall clock went backwards) is not fresh.
    pub fn consume(&self) -> bool {
        let Some(at) = lock(&self.noted).take() else {
            return false;
        };
        self.scheduler
            .wall_now()
            .duration_since(at)
            .is_ok_and(|age| age <= Self::FRESHNESS)
    }

    /// Forgets the note.
    pub fn clear(&self) {
        *lock(&self.noted) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::ManualScheduler;
    use std::time::UNIX_EPOCH;

    fn state() -> (UnlockRequestState, Arc<ManualScheduler>) {
        let s = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_000));
        (UnlockRequestState::new(s.clone()), s)
    }

    #[test]
    fn consume_within_ten_seconds_returns_true_exactly_once() {
        let (u, s) = state();
        u.note();
        s.advance(Duration::from_secs(10));
        assert!(u.consume());
        assert!(!u.consume());
    }

    #[test]
    fn consume_after_ten_seconds_returns_false() {
        let (u, s) = state();
        u.note();
        s.jump_wall_clock(Duration::from_millis(10_001));
        assert!(!u.consume());
        assert!(!u.consume());
    }

    #[test]
    fn consume_without_note_returns_false() {
        let (u, _) = state();
        assert!(!u.consume());
    }

    #[test]
    fn second_note_refreshes_the_window() {
        let (u, s) = state();
        u.note();
        s.advance(Duration::from_secs(8));
        u.note();
        s.advance(Duration::from_secs(8));
        assert!(u.consume());
    }

    #[test]
    fn clear_forgets_the_note() {
        let (u, _) = state();
        u.note();
        u.clear();
        assert!(!u.consume());
    }

    #[test]
    fn clones_share_the_note() {
        let (u, _) = state();
        let v = u.clone();
        u.note();
        assert!(v.consume());
        assert!(!u.consume());
    }
}
