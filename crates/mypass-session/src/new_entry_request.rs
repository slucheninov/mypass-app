//! The one pending request for the main window to start a new entry (the tray panel's "+", the
//! browser extension's New address or New file).
//!
//! The request lives here, in the backend, not in a webview: the event that tells the main window
//! about it carries nothing, and the main window takes the request with a command only it may
//! call. So a request that arrives before the main window listens is not lost (the window takes it
//! once its listener is registered), and an event forged by another webview finds nothing to take.
//!
//! One request at most: a newer one replaces it. It is taken at most once and only within
//! [`NewEntryRequestState::TTL`] of being made (monotonic time, so a wall-clock change neither
//! extends nor cuts it short). It carries only an entry type, no secret.

use crate::dto::EntryTypeDto;
use crate::scheduler::{lock, Scheduler};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The pending new-entry request. Cheap to clone; clones share the request.
#[derive(Clone)]
pub struct NewEntryRequestState {
    /// The entry type and the monotonic time of the request.
    pending: Arc<Mutex<Option<(EntryTypeDto, Duration)>>>,
    scheduler: Arc<dyn Scheduler>,
}

impl std::fmt::Debug for NewEntryRequestState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewEntryRequestState")
            .field("pending", &*lock(&self.pending))
            .finish_non_exhaustive()
    }
}

impl NewEntryRequestState {
    /// How long a request waits to be taken (for example for an unlock). Past it the user has
    /// moved on, and an editor appearing out of nowhere after a later unlock would only surprise.
    pub const TTL: Duration = Duration::from_secs(120);

    /// No request yet; times come from `scheduler.monotonic_now()`.
    pub fn new(scheduler: Arc<dyn Scheduler>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(None)),
            scheduler,
        }
    }

    /// Records a request now; it replaces any earlier one.
    pub fn request(&self, entry_type: EntryTypeDto) {
        *lock(&self.pending) = Some((entry_type, self.scheduler.monotonic_now()));
    }

    /// The pending request, if it is younger than [`Self::TTL`]; always forgets it, so a request
    /// is taken at most once.
    pub fn take(&self) -> Option<EntryTypeDto> {
        let (entry_type, at) = lock(&self.pending).take()?;
        let age = self.scheduler.monotonic_now().saturating_sub(at);
        (age <= Self::TTL).then_some(entry_type)
    }

    /// Whether a request is waiting (expired or not); for tests and diagnostics.
    pub fn is_pending(&self) -> bool {
        lock(&self.pending).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::ManualScheduler;
    use std::time::UNIX_EPOCH;

    fn state() -> (NewEntryRequestState, Arc<ManualScheduler>) {
        let s = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_000));
        (NewEntryRequestState::new(s.clone()), s)
    }

    #[test]
    fn nothing_is_pending_at_first() {
        let (r, _) = state();
        assert!(!r.is_pending());
        assert_eq!(r.take(), None);
    }

    #[test]
    fn a_request_is_taken_exactly_once() {
        let (r, _) = state();
        r.request(EntryTypeDto::Address);
        assert!(r.is_pending());
        assert_eq!(r.take(), Some(EntryTypeDto::Address));
        assert_eq!(r.take(), None);
        assert!(!r.is_pending());
    }

    #[test]
    fn the_newest_request_wins() {
        let (r, _) = state();
        r.request(EntryTypeDto::Address);
        r.request(EntryTypeDto::Files);
        assert_eq!(r.take(), Some(EntryTypeDto::Files));
        assert_eq!(r.take(), None);
    }

    #[test]
    fn a_request_lives_for_the_ttl_and_no_longer() {
        let (r, s) = state();
        r.request(EntryTypeDto::Login);
        s.advance(NewEntryRequestState::TTL);
        assert_eq!(r.take(), Some(EntryTypeDto::Login));

        r.request(EntryTypeDto::Login);
        s.advance(NewEntryRequestState::TTL + Duration::from_millis(1));
        assert_eq!(r.take(), None);
        assert!(!r.is_pending(), "an expired request is forgotten as well");
    }

    #[test]
    fn a_newer_request_restarts_the_wait() {
        let (r, s) = state();
        r.request(EntryTypeDto::Address);
        s.advance(Duration::from_secs(100));
        r.request(EntryTypeDto::Files);
        s.advance(Duration::from_secs(100));
        assert_eq!(r.take(), Some(EntryTypeDto::Files));
    }

    #[test]
    fn clones_share_the_request() {
        let (r, _) = state();
        let other = r.clone();
        r.request(EntryTypeDto::Files);
        assert_eq!(other.take(), Some(EntryTypeDto::Files));
        assert_eq!(r.take(), None);
    }
}
