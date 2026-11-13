//! Pairing: the six-digit code, the paired-browser store and the brute-force throttle.
//!
//! Flow: the extension asks for a pairing, the app shows a six-digit code, the user types it into
//! the extension, which submits it; on a match the app returns a fresh random secret once. The app
//! keeps only `sha256_hex(secret)` in the app-only secret store (never the shared file store, which
//! the host can read), and the extension presents the secret as `pairing` on every later request.
//! The secret is 256 bits of CSPRNG output, so an unsalted fast hash is enough: there is nothing to
//! brute-force offline, the hash only keeps the plaintext off disk.
//!
//! Failure rules (preflight R6): the paired list is loaded once and cached; `is_authorized` never
//! touches the secret store; a store that cannot be read is an error and is never written over, so
//! a denied or failed Keychain read can not erase the other paired browsers.
//!
//! Throttle rules (R7): 5 wrong attempts per code, 15 per 10 minutes across codes, 50 per 24 hours,
//! and 5 seconds between starts. All of it runs on the scheduler's monotonic clock, so a wall clock
//! set back does not reset a window, and it lives in memory: a restart clears it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, UNIX_EPOCH};

use mypass_platform::SecretStore;
use mypass_session::scheduler::Scheduler;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::endpoint::{generate_token, tokens_equal};
use crate::protocol::is_unsafe_text_char;

/// How long a code stays valid.
pub const PAIRING_CODE_TTL: Duration = Duration::from_secs(120);
/// Wrong attempts a single code survives; the fifth wrong one clears it.
pub const PAIRING_MAX_ATTEMPTS: u32 = 5;
/// Wrong attempts across all codes inside [`GLOBAL_FAILURE_WINDOW`] before `start` is refused.
pub const GLOBAL_FAILURE_BUDGET: u32 = 15;
/// The window of [`GLOBAL_FAILURE_BUDGET`].
pub const GLOBAL_FAILURE_WINDOW: Duration = Duration::from_secs(600);
/// Minimum time between two accepted starts.
pub const START_MIN_INTERVAL: Duration = Duration::from_secs(5);
/// Wrong attempts per [`DAILY_FAILURE_WINDOW`] before `start` is refused (R7).
pub const DAILY_FAILURE_CAP: u32 = 50;
/// The window of [`DAILY_FAILURE_CAP`].
pub const DAILY_FAILURE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// Secret-store service of the paired-browser list.
pub const PAIRING_SERVICE: &str = "com.mypass.extension.pairing";
/// Secret-store account of the paired-browser list.
pub const PAIRING_ACCOUNT: &str = "browsers";

/// Label used when the extension does not name its browser.
const DEFAULT_LABEL: &str = "Chromium browser";
/// Longest label kept, in characters.
const MAX_LABEL_CHARS: usize = 80;
/// More paired browsers than this is a bug or abuse, not a user.
const MAX_PAIRED_BROWSERS: usize = 64;
/// The stored list is tiny; anything larger is corrupt or hostile.
const MAX_STORE_BYTES: usize = 256 * 1024;

/// Failures of the pairing store. No variant carries a secret, a hash or stored content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairingError {
    /// The underlying secret store failed (read or write).
    #[error("pairing store unavailable: {0}")]
    Store(String),
    /// The stored list is not a valid paired-browser list; it is left untouched.
    #[error("the stored pairing list is invalid")]
    Corrupt,
    /// The list is already at its size cap.
    #[error("too many paired browsers")]
    TooMany,
}

/// A browser that completed pairing.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedBrowser {
    pub id: Uuid,
    pub label: String,
    /// Unix seconds.
    pub created_at: i64,
    /// `sha256_hex` of the pairing secret.
    pub secret_hash: String,
}

impl std::fmt::Debug for PairedBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairedBrowser")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("created_at", &self.created_at)
            .field("secret_hash", &"<redacted>")
            .finish()
    }
}

/// Lowercase hex SHA-256 of a pairing secret.
#[must_use]
pub fn sha256_hex(secret: &str) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(secret.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The paired browsers, as ONE secret-store item holding a JSON array. Feed it the app-only store
/// (`Platform.secrets`), never the host-shared one.
pub struct PairedBrowserStore {
    secrets: Arc<dyn SecretStore>,
    /// Serialises read-modify-write cycles of this instance.
    write: Mutex<()>,
}

impl PairedBrowserStore {
    #[must_use]
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            secrets,
            write: Mutex::new(()),
        }
    }

    /// Reads the list. A missing item is an empty list; a failed read or an unparsable item is an
    /// error, never an empty list.
    pub fn list(&self) -> Result<Vec<PairedBrowser>, PairingError> {
        let bytes = self
            .secrets
            .get(PAIRING_SERVICE, PAIRING_ACCOUNT)
            .map_err(|e| PairingError::Store(e.to_string()))?;
        match bytes {
            None => Ok(Vec::new()),
            Some(b) if b.len() > MAX_STORE_BYTES => Err(PairingError::Corrupt),
            Some(b) => serde_json::from_slice(&b).map_err(|_| PairingError::Corrupt),
        }
    }

    /// Adds a browser (replacing one with the same id). Writes only after a successful read.
    pub fn add(&self, browser: PairedBrowser) -> Result<(), PairingError> {
        let _guard = lock(&self.write);
        let mut all = self.list()?;
        all.retain(|b| b.id != browser.id);
        if all.len() >= MAX_PAIRED_BROWSERS {
            return Err(PairingError::TooMany);
        }
        all.push(browser);
        self.write_all(&all)
    }

    /// Removes a browser; an unknown id is not an error. Writes only after a successful read.
    pub fn remove(&self, id: Uuid) -> Result<(), PairingError> {
        let _guard = lock(&self.write);
        let mut all = self.list()?;
        let before = all.len();
        all.retain(|b| b.id != id);
        if all.len() == before {
            return Ok(());
        }
        self.write_all(&all)
    }

    /// True when `secret` belongs to a stored browser. Fails closed: an unreadable store is `false`.
    #[must_use]
    pub fn contains_secret(&self, secret: &str) -> bool {
        self.find_by_secret(secret).is_some()
    }

    /// The browser `secret` belongs to. Fails closed: an unreadable store is `None`.
    #[must_use]
    pub fn find_by_secret(&self, secret: &str) -> Option<PairedBrowser> {
        find_by_secret(&self.list().unwrap_or_default(), secret)
    }

    /// Writes `all` as the complete list, without reading the store first. The manager uses this:
    /// once loaded, its in-memory list is the source of truth, so a store that lost the item (the
    /// Keychain `set` is delete-then-add) is repaired by the next write instead of read back empty.
    pub fn save(&self, all: &[PairedBrowser]) -> Result<(), PairingError> {
        let _guard = lock(&self.write);
        self.write_all(all)
    }

    fn write_all(&self, all: &[PairedBrowser]) -> Result<(), PairingError> {
        let bytes = Zeroizing::new(serde_json::to_vec(all).map_err(|_| PairingError::Corrupt)?);
        self.secrets
            .set(PAIRING_SERVICE, PAIRING_ACCOUNT, &bytes)
            .map_err(|e| PairingError::Store(e.to_string()))
    }
}

/// Constant-time lookup: every stored hash is compared, the first match does not short-circuit.
fn find_by_secret(all: &[PairedBrowser], secret: &str) -> Option<PairedBrowser> {
    if secret.is_empty() {
        return None;
    }
    let presented = sha256_hex(secret);
    let mut found = None;
    for browser in all {
        if tokens_equal(&browser.secret_hash, &presented) && found.is_none() {
            found = Some(browser.clone());
        }
    }
    found
}

/// An active pairing code, as shown to the user.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingChallenge {
    /// Six digits, zero padded.
    pub code: String,
    pub browser: String,
    /// Unix seconds.
    pub expires_at: i64,
}

impl std::fmt::Debug for PairingChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingChallenge")
            .field("code", &"<redacted>")
            .field("browser", &self.browser)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Result of [`PairingManager::start`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    Started(PairingChallenge),
    /// Too soon after the last start, or a failure budget is spent.
    Throttled,
}

struct Active {
    code: Zeroizing<String>,
    browser: String,
    expires_at: i64,
    /// Monotonic deadline.
    deadline: Duration,
    wrong: u32,
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    last_start: Option<Duration>,
    /// Monotonic times of wrong attempts, oldest first, pruned to the daily window.
    failures: VecDeque<Duration>,
    /// Set when the daily cap is reached; only a restart clears it (R7).
    daily_latched: bool,
}

impl State {
    fn prune(&mut self, now: Duration) {
        while let Some(oldest) = self.failures.front() {
            if now.saturating_sub(*oldest) >= DAILY_FAILURE_WINDOW {
                self.failures.pop_front();
            } else {
                break;
            }
        }
    }

    fn failures_within(&self, now: Duration, window: Duration) -> usize {
        self.failures
            .iter()
            .filter(|t| now.saturating_sub(**t) < window)
            .count()
    }

    fn budget_spent(&self, now: Duration) -> bool {
        self.daily_latched
            || self.failures_within(now, GLOBAL_FAILURE_WINDOW) >= GLOBAL_FAILURE_BUDGET as usize
    }

    /// Records a wrong attempt and latches the daily cap when it is reached.
    fn record_failure(&mut self, now: Duration) {
        self.failures.push_back(now);
        if self.failures_within(now, DAILY_FAILURE_WINDOW) >= DAILY_FAILURE_CAP as usize {
            self.daily_latched = true;
        }
    }
}

type OnChange = Arc<dyn Fn() + Send + Sync>;

/// Drives pairing and answers "is this secret paired".
pub struct PairingManager {
    store: PairedBrowserStore,
    scheduler: Arc<dyn Scheduler>,
    state: Mutex<State>,
    /// The paired list, loaded once; `is_authorized` reads only this.
    cache: RwLock<Vec<PairedBrowser>>,
    /// Serialises list changes (store write plus cache update).
    mutate: Mutex<()>,
    on_change: Mutex<Option<OnChange>>,
}

impl std::fmt::Debug for PairingManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingManager").finish_non_exhaustive()
    }
}

impl PairingManager {
    /// Loads the paired list once. A store that cannot be read is an error: the bridge then runs
    /// without pairing rather than with an empty list it could later write over.
    pub fn new(
        store: PairedBrowserStore,
        scheduler: Arc<dyn Scheduler>,
    ) -> Result<Self, PairingError> {
        let cache = store.list()?;
        Ok(Self {
            store,
            scheduler,
            state: Mutex::new(State::default()),
            cache: RwLock::new(cache),
            mutate: Mutex::new(()),
            on_change: Mutex::new(None),
        })
    }

    /// Registers the callback fired after the active challenge or the paired list changed. It runs
    /// outside every internal lock.
    pub fn set_on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.on_change) = Some(f);
    }

    fn notify(&self) {
        let f = lock(&self.on_change).clone();
        if let Some(f) = f {
            f();
        }
    }

    fn wall_secs(&self) -> i64 {
        self.scheduler
            .wall_now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok())
            .unwrap_or(0)
    }

    /// Starts a pairing, replacing any active one.
    pub fn start(&self, browser: Option<&str>) -> StartOutcome {
        let now = self.scheduler.monotonic_now();
        let challenge = {
            let mut st = lock(&self.state);
            st.prune(now);
            if let Some(last) = st.last_start {
                if now.saturating_sub(last) < START_MIN_INTERVAL {
                    return StartOutcome::Throttled;
                }
            }
            if st.budget_spent(now) {
                return StartOutcome::Throttled;
            }
            let code = Zeroizing::new(generate_code());
            let label = sanitize_label(browser);
            let expires_at = self
                .wall_secs()
                .saturating_add(i64::try_from(PAIRING_CODE_TTL.as_secs()).unwrap_or(120));
            let challenge = PairingChallenge {
                code: (*code).clone(),
                browser: label.clone(),
                expires_at,
            };
            st.last_start = Some(now);
            st.active = Some(Active {
                code,
                browser: label,
                expires_at,
                deadline: now.saturating_add(PAIRING_CODE_TTL),
                wrong: 0,
            });
            challenge
        };
        self.notify();
        StartOutcome::Started(challenge)
    }

    /// Checks a submitted code. `Ok(Some(secret))` on success (the only time the plaintext secret
    /// exists outside the extension); `Ok(None)` for a wrong, expired or absent code; `Err` when the
    /// store could not record the pairing (the challenge is then spent and nothing is stored).
    pub fn complete(&self, code: &str) -> Result<Option<String>, PairingError> {
        let submitted: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let now = self.scheduler.monotonic_now();
        let label = {
            let mut st = lock(&self.state);
            st.prune(now);
            let Some((matches, expired, wrong, label)) = st.active.as_ref().map(|a| {
                (
                    tokens_equal(&a.code, &submitted),
                    now > a.deadline,
                    a.wrong.saturating_add(1),
                    a.browser.clone(),
                )
            }) else {
                return Ok(None);
            };
            if matches && !expired {
                st.active = None;
                label
            } else {
                st.record_failure(now);
                let spent = st.budget_spent(now);
                if expired || spent || wrong >= PAIRING_MAX_ATTEMPTS {
                    st.active = None;
                    drop(st);
                    self.notify();
                } else if let Some(a) = st.active.as_mut() {
                    a.wrong = wrong;
                }
                return Ok(None);
            }
        };
        let secret = generate_token();
        let browser = PairedBrowser {
            id: Uuid::new_v4(),
            label,
            created_at: self.wall_secs(),
            secret_hash: sha256_hex(&secret),
        };
        let result = self.apply(|list| {
            if list.len() >= MAX_PAIRED_BROWSERS {
                return Err(PairingError::TooMany);
            }
            list.push(browser);
            Ok(())
        });
        self.notify();
        result.map(|()| Some(secret))
    }

    /// Drops the active challenge.
    pub fn cancel(&self) {
        let had = lock(&self.state).active.take().is_some();
        if had {
            self.notify();
        }
    }

    /// The challenge to show, if one is active and not expired.
    #[must_use]
    pub fn active(&self) -> Option<PairingChallenge> {
        let now = self.scheduler.monotonic_now();
        let mut st = lock(&self.state);
        let expired = st.active.as_ref().is_some_and(|a| now > a.deadline);
        if expired {
            st.active = None;
            return None;
        }
        st.active.as_ref().map(|a| PairingChallenge {
            code: (*a.code).clone(),
            browser: a.browser.clone(),
            expires_at: a.expires_at,
        })
    }

    /// True when `pairing` is the secret of a paired browser. Reads only the in-memory list.
    #[must_use]
    pub fn is_authorized(&self, pairing: Option<&str>) -> bool {
        self.authorized_browser(pairing).is_some()
    }

    /// The browser that `pairing` belongs to (the secret decides, never a client-supplied id).
    #[must_use]
    pub fn authorized_browser(&self, pairing: Option<&str>) -> Option<PairedBrowser> {
        let secret = pairing?;
        find_by_secret(
            &self.cache.read().unwrap_or_else(PoisonError::into_inner),
            secret,
        )
    }

    /// The paired browsers (the in-memory list).
    #[must_use]
    pub fn list(&self) -> Vec<PairedBrowser> {
        self.cache
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Revokes a browser. On a store error the in-memory list is unchanged and the error returned.
    pub fn revoke(&self, id: Uuid) -> Result<(), PairingError> {
        self.apply(|list| {
            list.retain(|b| b.id != id);
            Ok(())
        })?;
        self.notify();
        Ok(())
    }

    /// Changes the list: edits a copy of the in-memory list and persists the WHOLE copy (never a
    /// re-read of the store plus the change). On success the copy becomes the list. On failure the
    /// in-memory list stays as it was, one attempt is made to put the previous list back into the
    /// store (the Keychain `set` deletes before it adds, so a failed add can leave nothing there),
    /// and the next successful change persists everything again.
    fn apply(
        &self,
        change: impl FnOnce(&mut Vec<PairedBrowser>) -> Result<(), PairingError>,
    ) -> Result<(), PairingError> {
        let _m = lock(&self.mutate);
        let before = self.list();
        let mut after = before.clone();
        change(&mut after)?;
        if after == before {
            return Ok(());
        }
        if let Err(e) = self.store.save(&after) {
            let _ = self.store.save(&before);
            return Err(e);
        }
        *self.cache.write().unwrap_or_else(PoisonError::into_inner) = after;
        Ok(())
    }
}

/// A label is shown in the UI: trimmed, free of control and format characters, capped; empty means default.
fn sanitize_label(raw: Option<&str>) -> String {
    let cleaned: String = raw
        .unwrap_or("")
        .chars()
        .filter(|c| !is_unsafe_text_char(*c))
        .take(MAX_LABEL_CHARS)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        DEFAULT_LABEL.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Six digits, uniform over 10^6 by rejection sampling on OS randomness.
fn generate_code() -> String {
    // The largest multiple of 10^6 that fits in a u32; values at or above it are rejected.
    const LIMIT: u32 = 4_294_000_000;
    let mut rng = rand::rngs::OsRng;
    loop {
        let v = rng.next_u32();
        if v < LIMIT {
            return format!("{:06}", v % 1_000_000);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use mypass_platform::testing::InMemorySecretStore;
    use mypass_platform::PlatformError;
    use mypass_session::scheduler::ManualScheduler;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn manual() -> Arc<ManualScheduler> {
        ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_000_000))
    }

    fn manager() -> (
        Arc<ManualScheduler>,
        Arc<InMemorySecretStore>,
        PairingManager,
    ) {
        let sched = manual();
        let mem = InMemorySecretStore::new();
        let m = PairingManager::new(PairedBrowserStore::new(mem.clone()), sched.clone()).unwrap();
        (sched, mem, m)
    }

    fn started(m: &PairingManager) -> PairingChallenge {
        match m.start(None) {
            StartOutcome::Started(c) => c,
            StartOutcome::Throttled => panic!("throttled"),
        }
    }

    fn wrong_code(c: &PairingChallenge) -> String {
        if c.code == "000000" {
            "000001"
        } else {
            "000000"
        }
        .to_owned()
    }

    /// A secret store that counts reads and can fail them.
    struct Flaky {
        inner: Arc<InMemorySecretStore>,
        gets: AtomicUsize,
        sets: AtomicUsize,
        fail_get: AtomicBool,
        fail_set: AtomicBool,
    }

    impl Flaky {
        fn new(inner: Arc<InMemorySecretStore>) -> Arc<Self> {
            Arc::new(Self {
                inner,
                gets: AtomicUsize::new(0),
                sets: AtomicUsize::new(0),
                fail_get: AtomicBool::new(false),
                fail_set: AtomicBool::new(false),
            })
        }
    }

    impl SecretStore for Flaky {
        fn set(&self, s: &str, a: &str, v: &[u8]) -> mypass_platform::PlatformResult<()> {
            self.sets.fetch_add(1, Ordering::SeqCst);
            if self.fail_set.load(Ordering::SeqCst) {
                return Err(PlatformError::Failed("denied".into()));
            }
            self.inner.set(s, a, v)
        }
        fn get(&self, s: &str, a: &str) -> mypass_platform::PlatformResult<Option<Vec<u8>>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            if self.fail_get.load(Ordering::SeqCst) {
                return Err(PlatformError::Failed("denied".into()));
            }
            self.inner.get(s, a)
        }
        fn delete(&self, s: &str, a: &str) -> mypass_platform::PlatformResult<()> {
            self.inner.delete(s, a)
        }
    }

    fn browser(secret: &str) -> PairedBrowser {
        PairedBrowser {
            id: Uuid::new_v4(),
            label: "B".into(),
            created_at: 1,
            secret_hash: sha256_hex(secret),
        }
    }

    #[test]
    fn start_returns_six_digit_zero_padded_code() {
        let (_s, _m, m) = manager();
        let c = started(&m);
        assert_eq!(c.code.len(), 6);
        assert!(c.code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(c.expires_at, 1_000_000 + 120);
        assert_eq!(m.active(), Some(c));
    }

    #[test]
    fn default_label_is_chromium_browser() {
        let (_s, _m, m) = manager();
        assert_eq!(started(&m).browser, "Chromium browser");
        let (_s, _m, m) = manager();
        assert!(
            matches!(m.start(Some("  \n ")), StartOutcome::Started(c) if c.browser == "Chromium browser")
        );
        let (_s, _m, m) = manager();
        assert!(
            matches!(m.start(Some("Brave\u{7}")), StartOutcome::Started(c) if c.browser == "Brave")
        );
    }

    #[test]
    fn correct_code_returns_secret_and_stores_only_hash() {
        let (_s, mem, m) = manager();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        assert_ne!(secret, sha256_hex(&secret));
        let stored = mem.get(PAIRING_SERVICE, PAIRING_ACCOUNT).unwrap().unwrap();
        let text = String::from_utf8(stored).unwrap();
        assert!(!text.contains(&secret), "plaintext secret was persisted");
        assert!(text.contains(&sha256_hex(&secret)));
        assert!(m.is_authorized(Some(&secret)));
    }

    #[test]
    fn only_hash_is_persisted() {
        let (_s, mem, m) = manager();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        let stored = mem.get(PAIRING_SERVICE, PAIRING_ACCOUNT).unwrap().unwrap();
        assert!(!stored.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }

    #[test]
    fn code_is_single_use() {
        let (_s, _m, m) = manager();
        let c = started(&m);
        assert!(m.complete(&c.code).unwrap().is_some());
        assert!(m.complete(&c.code).unwrap().is_none());
        assert_eq!(m.active(), None);
    }

    #[test]
    fn whitespace_in_submitted_code_is_ignored() {
        let (_s, _m, m) = manager();
        let c = started(&m);
        let spaced = format!(" {} {}\n", &c.code[..3], &c.code[3..]);
        assert!(m.complete(&spaced).unwrap().is_some());
    }

    #[test]
    fn expired_after_120s_fails() {
        let (s, _m, m) = manager();
        let c = started(&m);
        s.advance(PAIRING_CODE_TTL + Duration::from_secs(1));
        assert_eq!(m.active(), None);
        assert!(m.complete(&c.code).unwrap().is_none());
    }

    #[test]
    fn valid_at_exactly_120s() {
        let (s, _m, m) = manager();
        let c = started(&m);
        s.advance(PAIRING_CODE_TTL);
        assert!(m.complete(&c.code).unwrap().is_some());
    }

    #[test]
    fn expired_attempt_counts_against_the_budget() {
        let (s, _m, m) = manager();
        let c = started(&m);
        s.advance(PAIRING_CODE_TTL + Duration::from_secs(1));
        m.complete(&c.code).unwrap();
        assert_eq!(lock(&m.state).failures.len(), 1);
    }

    #[test]
    fn fifth_wrong_attempt_kills_the_code() {
        let (_s, _m, m) = manager();
        let c = started(&m);
        for _ in 0..4 {
            assert!(m.complete(&wrong_code(&c)).unwrap().is_none());
        }
        assert!(m.complete(&c.code).unwrap().is_some());

        let (_s, _m, m) = manager();
        let c = started(&m);
        for _ in 0..5 {
            assert!(m.complete(&wrong_code(&c)).unwrap().is_none());
        }
        assert_eq!(m.active(), None);
        assert!(m.complete(&c.code).unwrap().is_none());
    }

    #[test]
    fn correct_code_on_fifth_attempt_succeeds() {
        let (_s, _m, m) = manager();
        let c = started(&m);
        for _ in 0..4 {
            m.complete(&wrong_code(&c)).unwrap();
        }
        assert!(m.complete(&c.code).unwrap().is_some());
        // A success consumes no budget.
        assert_eq!(lock(&m.state).failures.len(), 4);
    }

    fn burn_challenge(s: &ManualScheduler, m: &PairingManager) {
        let c = started(m);
        for _ in 0..PAIRING_MAX_ATTEMPTS {
            m.complete(&wrong_code(&c)).unwrap();
        }
        s.advance(START_MIN_INTERVAL);
    }

    #[test]
    fn restarting_pairing_does_not_reset_global_failure_budget() {
        let (s, _m, m) = manager();
        for _ in 0..3 {
            burn_challenge(&s, &m);
        }
        assert_eq!(m.start(None), StartOutcome::Throttled);
        s.advance(GLOBAL_FAILURE_WINDOW);
        assert!(matches!(m.start(None), StartOutcome::Started(_)));
    }

    #[test]
    fn daily_cap_latches_until_restart() {
        let (s, _m, m) = manager();
        // One challenge of 5 wrong per 10-minute window never trips the 15-per-10-minutes budget,
        // but 10 of them reach the daily cap of 50.
        for _ in 0..10 {
            let c = started(&m);
            for _ in 0..PAIRING_MAX_ATTEMPTS {
                m.complete(&wrong_code(&c)).unwrap();
            }
            s.advance(GLOBAL_FAILURE_WINDOW);
        }
        assert_eq!(m.start(None), StartOutcome::Throttled);
        // Neither the passing of the window nor of a month releases it: a restart does.
        s.advance(DAILY_FAILURE_WINDOW);
        assert_eq!(m.start(None), StartOutcome::Throttled);
        s.advance(DAILY_FAILURE_WINDOW * 30);
        assert_eq!(m.start(None), StartOutcome::Throttled);
        let (_s2, _m2, fresh) = manager();
        assert!(matches!(fresh.start(None), StartOutcome::Started(_)));
    }

    #[test]
    fn correct_code_fails_after_the_budget_is_spent() {
        let (s, _m, m) = manager();
        for _ in 0..2 {
            burn_challenge(&s, &m);
        }
        let c = started(&m);
        for _ in 0..5 {
            m.complete(&wrong_code(&c)).unwrap();
        }
        assert_eq!(m.start(None), StartOutcome::Throttled);
        assert!(m.complete(&c.code).unwrap().is_none());
        assert!(m.list().is_empty());
    }

    #[test]
    fn correct_code_after_budget_spent_mid_challenge_is_refused() {
        let (s, _m, m) = manager();
        for _ in 0..2 {
            burn_challenge(&s, &m);
        }
        let c = started(&m);
        for _ in 0..4 {
            m.complete(&wrong_code(&c)).unwrap();
        }
        assert!(m.complete(&wrong_code(&c)).unwrap().is_none());
        assert!(m.complete(&c.code).unwrap().is_none());
    }

    #[test]
    fn setting_the_wall_clock_back_does_not_reset_a_window() {
        let (s, _m, m) = manager();
        for _ in 0..3 {
            burn_challenge(&s, &m);
        }
        assert_eq!(m.start(None), StartOutcome::Throttled);
        s.set_wall_clock(UNIX_EPOCH);
        assert_eq!(m.start(None), StartOutcome::Throttled);
        s.set_wall_clock(UNIX_EPOCH + Duration::from_secs(10_000_000));
        assert_eq!(m.start(None), StartOutcome::Throttled);
        // The start interval and the code lifetime also follow the monotonic clock.
        let (s, _m, m) = manager();
        let c = started(&m);
        s.jump_wall_clock(PAIRING_CODE_TTL * 10);
        assert_eq!(m.start(None), StartOutcome::Throttled);
        s.set_wall_clock(UNIX_EPOCH);
        assert!(m.complete(&c.code).unwrap().is_some());
    }

    #[test]
    fn wrong_length_and_non_digit_codes_are_rejected() {
        for bad in [
            "",
            "12345",
            "1234567",
            "abcdef",
            "12 34 5x",
            "\u{0661}\u{0662}\u{0663}\u{0664}\u{0665}\u{0666}",
        ] {
            let (_s, _m, m) = manager();
            let c = started(&m);
            assert_ne!(bad, c.code);
            assert!(m.complete(bad).unwrap().is_none(), "{bad:?}");
            assert!(m.list().is_empty());
        }
        let (_s, _m, m) = manager();
        let c = started(&m);
        assert!(m.complete(&format!("{}0", c.code)).unwrap().is_none());
        assert!(m.complete(&c.code[..5]).unwrap().is_none());
    }

    #[test]
    fn format_characters_are_dropped_from_labels() {
        let (_s, _m, m) = manager();
        assert!(matches!(
            m.start(Some("Br\u{202E}a\u{200B}ve")),
            StartOutcome::Started(c) if c.browser == "Brave"
        ));
        let (_s, _m, m) = manager();
        assert!(matches!(
            m.start(Some("\u{200B}\u{FEFF}")),
            StartOutcome::Started(c) if c.browser == "Chromium browser"
        ));
    }

    /// Like the Keychain store: `set` deletes the item first, and can fail after that.
    struct DeleteThenFail {
        inner: Arc<InMemorySecretStore>,
        fail_set: AtomicBool,
    }

    impl SecretStore for DeleteThenFail {
        fn set(&self, s: &str, a: &str, v: &[u8]) -> mypass_platform::PlatformResult<()> {
            self.inner.delete(s, a)?;
            if self.fail_set.load(Ordering::SeqCst) {
                return Err(PlatformError::Failed("add failed".into()));
            }
            self.inner.set(s, a, v)
        }
        fn get(&self, s: &str, a: &str) -> mypass_platform::PlatformResult<Option<Vec<u8>>> {
            self.inner.get(s, a)
        }
        fn delete(&self, s: &str, a: &str) -> mypass_platform::PlatformResult<()> {
            self.inner.delete(s, a)
        }
    }

    #[test]
    fn failed_write_then_successful_write_loses_no_paired_browser() {
        let mem = InMemorySecretStore::new();
        let fake = Arc::new(DeleteThenFail {
            inner: mem.clone(),
            fail_set: AtomicBool::new(false),
        });
        let (sched, m) = {
            let sched = manual();
            let m =
                PairingManager::new(PairedBrowserStore::new(fake.clone()), sched.clone()).unwrap();
            (sched, m)
        };
        let c = started(&m);
        let s1 = m.complete(&c.code).unwrap().unwrap();

        // The write fails after the item was deleted: the store is now empty.
        sched.advance(START_MIN_INTERVAL);
        let c = started(&m);
        fake.fail_set.store(true, Ordering::SeqCst);
        assert!(m.complete(&c.code).is_err());
        assert!(mem.get(PAIRING_SERVICE, PAIRING_ACCOUNT).unwrap().is_none());
        assert!(
            m.is_authorized(Some(&s1)),
            "memory still has the first browser"
        );
        assert_eq!(m.list().len(), 1);

        // The next successful write persists the whole in-memory list, not "empty store + change".
        fake.fail_set.store(false, Ordering::SeqCst);
        sched.advance(START_MIN_INTERVAL);
        let c = started(&m);
        let s3 = m.complete(&c.code).unwrap().unwrap();
        let reloaded = PairingManager::new(PairedBrowserStore::new(mem.clone()), manual()).unwrap();
        assert_eq!(reloaded.list().len(), 2);
        assert!(reloaded.is_authorized(Some(&s1)));
        assert!(reloaded.is_authorized(Some(&s3)));
    }

    #[test]
    fn failed_write_is_followed_by_one_restore_attempt() {
        let mem = InMemorySecretStore::new();
        let flaky = Flaky::new(mem.clone());
        let m = PairingManager::new(PairedBrowserStore::new(flaky.clone()), manual()).unwrap();
        let c = started(&m);
        m.complete(&c.code).unwrap().unwrap();
        let sets = flaky.sets.load(Ordering::SeqCst);
        flaky.fail_set.store(true, Ordering::SeqCst);
        let id = m.list()[0].id;
        assert!(m.revoke(id).is_err());
        assert_eq!(
            flaky.sets.load(Ordering::SeqCst),
            sets + 2,
            "write plus restore"
        );
        assert_eq!(m.list().len(), 1);
    }

    #[test]
    fn budget_spent_mid_challenge_clears_it() {
        let (s, _m, m) = manager();
        for _ in 0..2 {
            burn_challenge(&s, &m);
        }
        let c = started(&m);
        for _ in 0..4 {
            m.complete(&wrong_code(&c)).unwrap();
        }
        // 14 wrong so far; the 15th spends the global budget and ends this challenge.
        m.complete(&wrong_code(&c)).unwrap();
        assert_eq!(m.active(), None);
    }

    #[test]
    fn start_throttled_within_five_seconds() {
        let (s, _m, m) = manager();
        started(&m);
        s.advance(Duration::from_secs(4));
        assert_eq!(m.start(None), StartOutcome::Throttled);
        s.advance(Duration::from_secs(1));
        assert!(matches!(m.start(None), StartOutcome::Started(_)));
    }

    #[test]
    fn is_authorized_true_only_for_stored_secret() {
        let (_s, _m, m) = manager();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        assert!(m.is_authorized(Some(&secret)));
        assert!(!m.is_authorized(Some("nope")));
        assert!(!m.is_authorized(Some("")));
        assert!(!m.is_authorized(None));
        assert!(!m.is_authorized(Some(&sha256_hex(&secret))));
    }

    #[test]
    fn authorized_browser_resolves_from_secret_not_client_id() {
        let (s, _m, m) = manager();
        let c = started(&m);
        let s1 = m.complete(&c.code).unwrap().unwrap();
        s.advance(START_MIN_INTERVAL);
        let c = match m.start(Some("Second")) {
            StartOutcome::Started(c) => c,
            StartOutcome::Throttled => panic!(),
        };
        let s2 = m.complete(&c.code).unwrap().unwrap();
        assert_eq!(m.authorized_browser(Some(&s2)).unwrap().label, "Second");
        assert_eq!(
            m.authorized_browser(Some(&s1)).unwrap().label,
            "Chromium browser"
        );
        assert_ne!(
            m.authorized_browser(Some(&s1)).unwrap().id,
            m.authorized_browser(Some(&s2)).unwrap().id
        );
    }

    #[test]
    fn revoke_removes_and_deauthorises() {
        let (_s, mem, m) = manager();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        let id = m.authorized_browser(Some(&secret)).unwrap().id;
        m.revoke(id).unwrap();
        assert!(!m.is_authorized(Some(&secret)));
        assert!(m.list().is_empty());
        assert!(PairedBrowserStore::new(mem).list().unwrap().is_empty());
    }

    #[test]
    fn store_list_survives_new_instance_over_same_secret_store() {
        let (_s, mem, m) = manager();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        let m2 = PairingManager::new(PairedBrowserStore::new(mem), manual()).unwrap();
        assert!(m2.is_authorized(Some(&secret)));
        assert_eq!(m2.list().len(), 1);
    }

    #[test]
    fn pairing_store_uses_app_only_secret_store() {
        // The constructor takes the secret store it is given and nothing else: the wiring test of
        // Task 18 asserts that `platform.secrets` (app-only) is what arrives here.
        let mem = InMemorySecretStore::new();
        let store = PairedBrowserStore::new(mem.clone());
        store.add(browser("x")).unwrap();
        assert!(mem.get(PAIRING_SERVICE, PAIRING_ACCOUNT).unwrap().is_some());
    }

    #[test]
    fn json_field_names_match_the_old_keychain_item() {
        let b = browser("x");
        let v: serde_json::Value = serde_json::to_value(&b).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["createdAt", "id", "label", "secretHash"]);
    }

    #[test]
    fn generated_codes_cover_all_digits_uniformly_smoke() {
        let mut seen = [[false; 10]; 6];
        for _ in 0..2000 {
            let code = generate_code();
            assert_eq!(code.len(), 6);
            for (i, b) in code.bytes().enumerate() {
                seen[i][usize::from(b - b'0')] = true;
            }
        }
        assert!(seen.iter().all(|pos| pos.iter().all(|d| *d)));
    }

    #[test]
    fn debug_redacts_code_and_secret() {
        let (_s, _m, m) = manager();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        let b = m.authorized_browser(Some(&secret)).unwrap();
        let c2 = started_after(&m);
        for text in [format!("{c2:?}"), format!("{b:?}"), format!("{m:?}")] {
            assert!(!text.contains(&c2.code));
            assert!(!text.contains(&b.secret_hash));
            assert!(!text.contains(&secret));
        }
    }

    fn started_after(m: &PairingManager) -> PairingChallenge {
        lock(&m.state).last_start = None;
        started(m)
    }

    #[test]
    fn on_change_fires_for_start_complete_cancel_revoke() {
        let (_s, _m, m) = manager();
        let count = Arc::new(AtomicUsize::new(0));
        let c2 = count.clone();
        m.set_on_change(Arc::new(move || {
            c2.fetch_add(1, Ordering::SeqCst);
        }));
        let c = started(&m);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let secret = m.complete(&c.code).unwrap().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        lock(&m.state).last_start = None;
        started(&m);
        m.cancel();
        assert_eq!(count.load(Ordering::SeqCst), 4);
        let id = m.authorized_browser(Some(&secret)).unwrap().id;
        m.revoke(id).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn add_after_failed_read_never_overwrites_existing_pairings() {
        let mem = InMemorySecretStore::new();
        let flaky = Flaky::new(mem.clone());
        let store = PairedBrowserStore::new(flaky.clone());
        store.add(browser("one")).unwrap();
        let before = mem.get(PAIRING_SERVICE, PAIRING_ACCOUNT).unwrap();
        flaky.fail_get.store(true, Ordering::SeqCst);
        let sets = flaky.sets.load(Ordering::SeqCst);
        assert!(matches!(
            store.add(browser("two")),
            Err(PairingError::Store(_))
        ));
        assert!(matches!(
            store.remove(Uuid::new_v4()),
            Err(PairingError::Store(_))
        ));
        assert_eq!(flaky.sets.load(Ordering::SeqCst), sets, "no write happened");
        assert_eq!(mem.get(PAIRING_SERVICE, PAIRING_ACCOUNT).unwrap(), before);
        assert!(!store.contains_secret("one"), "fails closed");
    }

    #[test]
    fn corrupt_store_fails_closed() {
        let mem = InMemorySecretStore::new();
        mem.set(PAIRING_SERVICE, PAIRING_ACCOUNT, b"{not json")
            .unwrap();
        let store = PairedBrowserStore::new(mem.clone());
        assert_eq!(store.list(), Err(PairingError::Corrupt));
        assert_eq!(store.add(browser("x")), Err(PairingError::Corrupt));
        assert!(PairingManager::new(PairedBrowserStore::new(mem.clone()), manual()).is_err());
        assert_eq!(
            mem.get(PAIRING_SERVICE, PAIRING_ACCOUNT).unwrap().unwrap(),
            b"{not json"
        );
    }

    #[test]
    fn is_authorized_does_not_hit_the_store() {
        let mem = InMemorySecretStore::new();
        let flaky = Flaky::new(mem);
        let sched = manual();
        let m = PairingManager::new(PairedBrowserStore::new(flaky.clone()), sched).unwrap();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        let gets = flaky.gets.load(Ordering::SeqCst);
        flaky.fail_get.store(true, Ordering::SeqCst);
        for _ in 0..10 {
            assert!(m.is_authorized(Some(&secret)));
            assert!(!m.is_authorized(Some("other")));
            let _ = m.list();
        }
        assert_eq!(flaky.gets.load(Ordering::SeqCst), gets);
    }

    #[test]
    fn failed_write_on_complete_stores_and_authorises_nothing() {
        let mem = InMemorySecretStore::new();
        let flaky = Flaky::new(mem);
        let m = PairingManager::new(PairedBrowserStore::new(flaky.clone()), manual()).unwrap();
        let c = started(&m);
        flaky.fail_set.store(true, Ordering::SeqCst);
        assert!(m.complete(&c.code).is_err());
        assert!(m.list().is_empty());
        assert_eq!(m.active(), None);
    }

    #[test]
    fn failed_revoke_keeps_the_browser() {
        let mem = InMemorySecretStore::new();
        let flaky = Flaky::new(mem);
        let m = PairingManager::new(PairedBrowserStore::new(flaky.clone()), manual()).unwrap();
        let c = started(&m);
        let secret = m.complete(&c.code).unwrap().unwrap();
        let id = m.authorized_browser(Some(&secret)).unwrap().id;
        flaky.fail_set.store(true, Ordering::SeqCst);
        assert!(m.revoke(id).is_err());
        assert!(m.is_authorized(Some(&secret)));
    }

    #[test]
    fn sha256_hex_matches_a_known_vector() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
