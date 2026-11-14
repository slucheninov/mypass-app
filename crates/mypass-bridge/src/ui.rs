//! What the bridge asks of the app's UI, and the in-memory store of save proposals.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use mypass_session::scheduler::Scheduler;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::pairing::PairingChallenge;

/// The complete UI surface of the bridge. Stage 4 adds no methods.
pub trait BridgeUi: Send + Sync {
    fn pairing_code(&self, c: &PairingChallenge);
    fn pairing_finished(&self);
    /// The password never travels through the UI call: the UI reads it back with the proposal id.
    fn save_login_proposal(&self, id: &str, url: &str, username: &str);
    fn save_totp_proposal(&self, id: &str, url: &str, entry_id: Option<Uuid>);
    fn unlock_requested(&self);
    fn open_app_requested(&self);
    fn open_browser_settings_requested(&self);
    fn credential_filled(&self, id: Uuid);
    /// The extension changed the app-side passkey toggle (`PasskeyService`, only on a real
    /// change); the app re-emits its settings so open windows show the new value.
    fn passkey_proxy_changed(&self, _enabled: bool) {}
}

/// A save proposal waiting for the user. The secret lives only here.
pub enum Proposal {
    Login {
        url: String,
        username: String,
        password: Zeroizing<String>,
    },
    Totp {
        url: String,
        otp: Zeroizing<String>,
        entry_id: Option<Uuid>,
    },
}

impl std::fmt::Debug for Proposal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Login { .. } => f.write_str("Proposal::Login { .. }"),
            Self::Totp { .. } => f.write_str("Proposal::Totp { .. }"),
        }
    }
}

/// Most proposals kept at once; the oldest is dropped for a ninth.
const MAX_PROPOSALS: usize = 8;
/// How long a proposal waits for the user.
const PROPOSAL_TTL: Duration = Duration::from_secs(120);

struct Slot {
    id: String,
    proposal: Proposal,
    /// Monotonic deadline.
    deadline: Duration,
}

struct Inner {
    scheduler: Arc<dyn Scheduler>,
    slots: Mutex<Vec<Slot>>,
}

impl Inner {
    fn slots(&self) -> MutexGuard<'_, Vec<Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn prune(&self) {
        let now = self.scheduler.monotonic_now();
        self.slots().retain(|s| now < s.deadline);
    }
}

/// In-memory proposals: at most 8, 120 s each, cleared on lock. Dropped proposals are zeroized.
pub struct ProposalStore(Arc<Inner>);

impl ProposalStore {
    #[must_use]
    pub fn new(scheduler: Arc<dyn Scheduler>) -> Self {
        Self(Arc::new(Inner {
            scheduler,
            slots: Mutex::new(Vec::new()),
        }))
    }

    /// Keeps `p` and returns its id (unguessable, single use).
    pub fn put(&self, p: Proposal) -> String {
        let id = Uuid::new_v4().simple().to_string();
        let deadline = self
            .0
            .scheduler
            .monotonic_now()
            .saturating_add(PROPOSAL_TTL);
        {
            let mut slots = self.0.slots();
            slots.push(Slot {
                id: id.clone(),
                proposal: p,
                deadline,
            });
            if slots.len() > MAX_PROPOSALS {
                slots.remove(0);
            }
        }
        // The secret must not outlive its TTL in memory even when nobody calls `take`.
        let weak: Weak<Inner> = Arc::downgrade(&self.0);
        self.0.scheduler.schedule(
            PROPOSAL_TTL,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.prune();
                }
            }),
        );
        id
    }

    /// Removes and returns the proposal, unless it expired.
    pub fn take(&self, id: &str) -> Option<Proposal> {
        let now = self.0.scheduler.monotonic_now();
        let mut slots = self.0.slots();
        let at = slots.iter().position(|s| s.id == id)?;
        let slot = slots.remove(at);
        (now < slot.deadline).then_some(slot.proposal)
    }

    pub fn clear(&self) {
        self.0.slots().clear();
    }
}

impl std::fmt::Debug for ProposalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProposalStore").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use mypass_session::scheduler::ManualScheduler;
    use std::time::UNIX_EPOCH;

    fn login(n: usize) -> Proposal {
        Proposal::Login {
            url: format!("https://e{n}.example"),
            username: "u".into(),
            password: Zeroizing::new("pw".into()),
        }
    }

    #[test]
    fn take_returns_once_and_expires_after_ttl() {
        let sched = ManualScheduler::new(UNIX_EPOCH);
        let store = ProposalStore::new(sched.clone());
        let id = store.put(login(0));
        assert!(store.take(&id).is_some());
        assert!(store.take(&id).is_none());
        let id = store.put(login(1));
        sched.advance(Duration::from_secs(120));
        assert!(store.take(&id).is_none());
    }

    #[test]
    fn ninth_proposal_drops_the_oldest() {
        let store = ProposalStore::new(ManualScheduler::new(UNIX_EPOCH));
        let ids: Vec<String> = (0..9).map(|n| store.put(login(n))).collect();
        assert!(store.take(&ids[0]).is_none());
        assert!(store.take(&ids[8]).is_some());
    }

    #[test]
    fn clear_drops_everything_and_debug_hides_secrets() {
        let store = ProposalStore::new(ManualScheduler::new(UNIX_EPOCH));
        let id = store.put(login(0));
        assert!(!format!("{:?}", login(0)).contains("pw"));
        store.clear();
        assert!(store.take(&id).is_none());
    }
}
