//! Ceremonies waiting for the user between phase one (`passkeyCreate` / `passkeyGet`) and phase
//! two (`passkeyCreateConfirm` / `passkeyGetApprove`).
//!
//! The table is a pure value: every call takes the monotonic `now` from the caller (the injected
//! `Scheduler`, never a clock of its own), so expiry is testable and a wall-clock jump neither
//! extends nor kills a ceremony.
//!
//! **Epoch.** [`Pending::clear`] runs when the session stops being unlocked and when the vault
//! is switched while unlocked; it bumps the epoch. A phase-one handler reads [`Pending::epoch`]
//! *before* it reads the vault and hands that value to `put_*`, which refuses the put when the
//! epoch moved in between. So a lock or a switch that lands between the vault read and the put
//! never leaves a ceremony behind for a vault that is gone. The pending mutex is never held
//! across a session call (`with_db`, `mutate`, `flush`): the session observer that clears the
//! table runs after the session lock is released and takes this mutex, so holding both would
//! invert the lock order.
//!
//! Entries hold no key material: the request (rpId, challenge, client data inputs, user
//! fields) and the candidate entry ids only. The private key is read from the entry at confirm
//! or approve time and dropped right after signing.

use std::time::Duration;

use mypass_core::passkey::{ValidatedCreate, ValidatedGet};
use uuid::Uuid;

/// How long a ceremony waits for the user, in seconds (the extension's own watchdog is 120 s).
pub const PENDING_TTL_SECS: u64 = 120;
/// [`PENDING_TTL_SECS`] as a duration.
pub const PENDING_TTL: Duration = Duration::from_secs(PENDING_TTL_SECS);
/// Most ceremonies kept per kind; a seventeenth evicts the oldest. A page can start ceremonies
/// on its own timing, so the table must stay bounded.
pub const MAX_PENDING_PER_KIND: usize = 16;

/// A `passkeyCreate` waiting for Save or Replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCreate {
    pub request: ValidatedCreate,
    /// The entries offered as save targets; confirm accepts only one of these (or a new entry).
    pub candidates: Vec<Uuid>,
}

/// A `passkeyGet` waiting for the user to pick an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingGet {
    pub request: ValidatedGet,
    /// The entries offered for sign-in; approve accepts only one of these.
    pub candidates: Vec<Uuid>,
}

/// A snapshot of the table's epoch, taken before the vault read of a phase-one handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Epoch(u64);

#[derive(Debug)]
struct Slot<T> {
    id: i64,
    /// Monotonic deadline.
    deadline: Duration,
    value: T,
}

/// Both tables plus the epoch. See the module docs.
#[derive(Debug, Default)]
pub struct Pending {
    epoch: u64,
    creates: Vec<Slot<PendingCreate>>,
    gets: Vec<Slot<PendingGet>>,
}

fn put<T>(slots: &mut Vec<Slot<T>>, id: i64, value: T, now: Duration) {
    slots.retain(|s| now < s.deadline && s.id != id);
    slots.push(Slot {
        id,
        deadline: now.saturating_add(PENDING_TTL),
        value,
    });
    while slots.len() > MAX_PENDING_PER_KIND {
        slots.remove(0);
    }
}

fn take<T>(slots: &mut Vec<Slot<T>>, id: i64, now: Duration) -> Option<T> {
    slots.retain(|s| now < s.deadline);
    let at = slots.iter().position(|s| s.id == id)?;
    Some(slots.remove(at).value)
}

impl Pending {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current epoch; read it before the vault read and pass it to `put_*`.
    #[must_use]
    pub fn epoch(&self) -> Epoch {
        Epoch(self.epoch)
    }

    /// Keeps `p` under `id` (replacing an older ceremony with the same id), unless the table was
    /// cleared since `epoch` was read. Returns whether it was kept.
    pub fn put_create(&mut self, epoch: Epoch, id: i64, p: PendingCreate, now: Duration) -> bool {
        if epoch != self.epoch() {
            return false;
        }
        put(&mut self.creates, id, p, now);
        true
    }

    /// Removes and returns the create ceremony `id` unless it expired (single use).
    pub fn take_create(&mut self, id: i64, now: Duration) -> Option<PendingCreate> {
        take(&mut self.creates, id, now)
    }

    /// Keeps `p` under `id`; see [`Pending::put_create`].
    pub fn put_get(&mut self, epoch: Epoch, id: i64, p: PendingGet, now: Duration) -> bool {
        if epoch != self.epoch() {
            return false;
        }
        put(&mut self.gets, id, p, now);
        true
    }

    /// Removes and returns the get ceremony `id` unless it expired (single use).
    pub fn take_get(&mut self, id: i64, now: Duration) -> Option<PendingGet> {
        take(&mut self.gets, id, now)
    }

    /// Drops every ceremony and bumps the epoch (lock, vault switch, toggle off).
    pub fn clear(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.creates.clear();
        self.gets.clear();
    }

    /// Drops the ceremony `id` from both tables (`passkeyCancel`: the extension does not say
    /// which kind it cancels).
    pub fn remove_both(&mut self, id: i64) {
        self.creates.retain(|s| s.id != id);
        self.gets.retain(|s| s.id != id);
    }

    /// `(creates, gets)` currently held, expired ones included until the next put or take.
    #[must_use]
    pub fn len(&self) -> (usize, usize) {
        (self.creates.len(), self.gets.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.creates.is_empty() && self.gets.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
pub(crate) mod tests {
    use super::*;
    use mypass_core::passkey::{b64url, parse_create, parse_get};

    const ORIGIN: &str = "https://login.example.com";

    fn overrides() -> String {
        format!(
            r#""extensions":{{"remoteDesktopClientOverride":{{"origin":"{ORIGIN}","sameOriginWithAncestors":true}}}}"#
        )
    }

    /// A parsed create request for `example.com`.
    pub(crate) fn create_request() -> ValidatedCreate {
        let json = format!(
            r#"{{"challenge":"{}","rp":{{"id":"example.com","name":"Example"}},
            "user":{{"id":"{}","name":"alice"}},
            "pubKeyCredParams":[{{"type":"public-key","alg":-7}}],{}}}"#,
            b64url::encode(b"0123456789abcdef-challenge"),
            b64url::encode(b"user-handle"),
            overrides()
        );
        parse_create(ORIGIN, &json).unwrap()
    }

    /// A parsed get request for `example.com`.
    pub(crate) fn get_request() -> ValidatedGet {
        let json = format!(
            r#"{{"challenge":"{}","rpId":"example.com",{}}}"#,
            b64url::encode(b"0123456789abcdef-challenge"),
            overrides()
        );
        parse_get(ORIGIN, &json).unwrap()
    }

    pub(crate) fn pc() -> PendingCreate {
        PendingCreate {
            request: create_request(),
            candidates: vec![Uuid::from_u128(1)],
        }
    }

    pub(crate) fn pg() -> PendingGet {
        PendingGet {
            request: get_request(),
            candidates: vec![Uuid::from_u128(2)],
        }
    }

    const T0: Duration = Duration::from_secs(1_000);

    #[test]
    fn take_is_single_use() {
        let mut p = Pending::new();
        let e = p.epoch();
        assert!(p.put_create(e, 1, pc(), T0));
        assert!(p.put_get(e, 1, pg(), T0));
        assert_eq!(p.take_create(1, T0), Some(pc()));
        assert_eq!(p.take_create(1, T0), None);
        assert_eq!(p.take_get(1, T0), Some(pg()));
        assert_eq!(p.take_get(1, T0), None);
        assert!(p.is_empty());
    }

    #[test]
    fn pending_ttl_expiry_and_cap_evict_oldest() {
        let mut p = Pending::new();
        let e = p.epoch();
        assert!(p.put_create(e, 1, pc(), T0));
        // One second before the deadline it is still there, at the deadline it is gone.
        assert!(p
            .take_create(1, T0 + PENDING_TTL - Duration::from_secs(1))
            .is_some());
        assert!(p.put_create(e, 1, pc(), T0));
        assert!(p.take_create(1, T0 + PENDING_TTL).is_none());
        assert_eq!(p.len(), (0, 0));

        // The cap: the seventeenth ceremony evicts the oldest, per kind.
        for id in 0..=16 {
            assert!(p.put_get(e, id, pg(), T0));
        }
        assert_eq!(p.len(), (0, MAX_PENDING_PER_KIND));
        assert!(p.take_get(0, T0).is_none());
        assert!(p.take_get(16, T0).is_some());
        assert!(p.take_get(1, T0).is_some());

        // A put prunes expired ceremonies, so they never count against the cap.
        let mut p = Pending::new();
        for id in 0..10 {
            assert!(p.put_create(e, id, pc(), T0));
        }
        assert!(p.put_create(e, 99, pc(), T0 + PENDING_TTL));
        assert_eq!(p.len(), (1, 0));
    }

    #[test]
    fn same_id_replaces_the_older_ceremony() {
        let mut p = Pending::new();
        let e = p.epoch();
        assert!(p.put_create(e, 5, pc(), T0));
        let mut newer = pc();
        newer.candidates = vec![];
        assert!(p.put_create(e, 5, newer.clone(), T0 + Duration::from_secs(1)));
        assert_eq!(p.len(), (1, 0));
        assert_eq!(p.take_create(5, T0), Some(newer));
    }

    #[test]
    fn clear_bumps_the_epoch_and_refuses_a_stale_put() {
        let mut p = Pending::new();
        let before = p.epoch();
        assert!(p.put_create(before, 1, pc(), T0));
        assert!(p.put_get(before, 2, pg(), T0));
        p.clear();
        assert!(p.is_empty());
        assert_ne!(p.epoch(), before);
        // A handler that read the epoch before the clear must not store its ceremony.
        assert!(!p.put_create(before, 3, pc(), T0));
        assert!(!p.put_get(before, 3, pg(), T0));
        assert!(p.is_empty());
        assert!(p.put_create(p.epoch(), 3, pc(), T0));
    }

    #[test]
    fn remove_both_drops_create_and_get_and_keeps_others() {
        let mut p = Pending::new();
        let e = p.epoch();
        assert!(p.put_create(e, -4, pc(), T0));
        assert!(p.put_get(e, -4, pg(), T0));
        assert!(p.put_get(e, 8, pg(), T0));
        p.remove_both(-4);
        p.remove_both(-4);
        assert_eq!(p.len(), (0, 1));
        assert!(p.take_get(8, T0).is_some());
    }

    /// R27: the pending entries hold no key material. A source scan with comments stripped, so a
    /// doc line that names a key type cannot trip it.
    #[test]
    fn pending_table_holds_no_key_material() {
        let src = include_str!("pending.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        let stripped: String = code
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "SecretKey",
            "SigningKey",
            "PasskeySigningKey",
            "Zeroizing<",
            "PRIVATE_KEY",
        ] {
            assert!(!stripped.contains(forbidden), "{forbidden}");
        }
        // Sanity: the scan sees the code it guards.
        assert!(stripped.contains("pub struct PendingCreate"));
    }
}
