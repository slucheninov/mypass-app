//! System events that lock the vault: sleep, wake and screen lock.

use crate::PlatformResult;
use std::sync::Arc;

/// An OS event the session reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemEvent {
    /// The machine is about to sleep.
    Sleep,
    /// The machine woke up.
    Wake,
    /// The screen was locked (also while the machine stays awake).
    ScreenLocked,
    /// The screen was unlocked.
    ScreenUnlocked,
}

/// A live subscription. Dropping it, or calling [`Subscription::cancel`], unsubscribes; after that
/// the sink is never invoked again by a new event and the subscription no longer keeps it alive.
pub struct Subscription(Option<Box<dyn FnOnce() + Send>>);

impl Subscription {
    /// A subscription whose cancellation runs `unsubscribe` exactly once.
    pub fn new(unsubscribe: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(unsubscribe)))
    }

    /// Unsubscribes now (the same as dropping).
    pub fn cancel(mut self) {
        self.unsubscribe();
    }

    fn unsubscribe(&mut self) {
        if let Some(unsubscribe) = self.0.take() {
            unsubscribe();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.unsubscribe();
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("active", &self.0.is_some())
            .finish()
    }
}

/// Source of [`SystemEvent`]s.
pub trait SystemEvents: Send + Sync {
    /// Delivers every later event to `sink` until the returned [`Subscription`] is dropped. The
    /// implementation documents on which thread the sink runs; the sink must be cheap and must not
    /// block (hand the event to a channel).
    fn subscribe(
        &self,
        sink: Arc<dyn Fn(SystemEvent) + Send + Sync>,
    ) -> PlatformResult<Subscription>;
}
