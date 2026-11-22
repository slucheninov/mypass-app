//! The app-owned vault unlock seam and the bridge-wide retry limits.

use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::time::Duration;

use mypass_session::scheduler::Scheduler;
use secrecy::SecretString;

pub const UNLOCK_FREE_ATTEMPTS: u32 = 5;
pub const UNLOCK_BASE_BACKOFF: Duration = Duration::from_secs(30);
pub const UNLOCK_MAX_BACKOFF: Duration = Duration::from_secs(900);
const DEVICE_UNLOCK_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnlockFailure {
    #[error("wrong password")]
    WrongPassword,
    #[error("unlock cancelled")]
    Cancelled,
    #[error("device unlock unavailable")]
    DeviceUnavailable,
    #[error("unlock in MyPass")]
    InApp,
    #[error("unlock failed")]
    Failed,
    /// The device prompt ran and did not recognize the user (a rejected fingerprint).
    #[error("not recognized")]
    NotRecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceOffer {
    pub offered: bool,
    pub touch_id: bool,
}

pub trait VaultUnlocker: Send + Sync {
    fn unlock_with_password(&self, password: SecretString) -> Result<(), UnlockFailure>;
    /// One device prompt. `automatic` when the popup starts it by itself on opening: the app then
    /// refuses it as `Cancelled`, without a prompt, while another prompt is in flight or within
    /// the app's debounce after the last one ended.
    fn unlock_with_device(&self, automatic: bool) -> Result<(), UnlockFailure>;
    fn device_offer(&self) -> DeviceOffer;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordAttempt {
    Unlocked,
    NotNeeded,
    WrongPassword,
    Throttled { retry_after_secs: u64 },
    InApp,
    Failed,
}

#[derive(Default)]
struct State {
    consecutive_wrong: u32,
    blocked_until: Option<Duration>,
    /// Device prompts started from a browser that did not recognize the user, since the last
    /// unlock. Every lock spell follows an unlock (which resets it), so this is the count of the
    /// current lock spell; the popup reads it on every opening.
    device_failures: u32,
    generation: u64,
}

impl State {
    fn retry_after_secs(&self, now: Duration) -> Option<u64> {
        let remaining = self.blocked_until?.checked_sub(now)?;
        if remaining.is_zero() {
            return None;
        }
        Some(
            remaining
                .as_secs()
                .saturating_add(u64::from(remaining.subsec_nanos() > 0)),
        )
    }
}

pub struct UnlockThrottle {
    // The session observer can synchronously reset the throttle during the unlocker call.
    // Serialize attempts separately so the observer and the options poll can both access state.
    attempts: Mutex<()>,
    last_device_prompt: Mutex<Option<Duration>>,
    state: Mutex<State>,
    scheduler: Arc<dyn Scheduler>,
}

impl UnlockThrottle {
    pub fn new(scheduler: Arc<dyn Scheduler>) -> Self {
        Self {
            attempts: Mutex::new(()),
            last_device_prompt: Mutex::new(None),
            state: Mutex::new(State::default()),
            scheduler,
        }
    }

    pub fn attempt(
        &self,
        is_unlocked: impl FnOnce() -> bool,
        try_unlock: impl FnOnce() -> Result<(), UnlockFailure>,
    ) -> PasswordAttempt {
        let _attempt = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        // A preceding attempt may have finished while this request was waiting.
        if is_unlocked() {
            return PasswordAttempt::NotNeeded;
        }
        let generation = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(retry_after_secs) = state.retry_after_secs(self.scheduler.monotonic_now()) {
                return PasswordAttempt::Throttled { retry_after_secs };
            }
            state.generation
        };
        match try_unlock() {
            Ok(()) => {
                self.reset();
                PasswordAttempt::Unlocked
            }
            Err(UnlockFailure::WrongPassword) => {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                // An independent successful unlock supersedes an in-flight failure.
                if state.generation != generation {
                    return PasswordAttempt::WrongPassword;
                }
                state.consecutive_wrong = state.consecutive_wrong.saturating_add(1);
                if state.consecutive_wrong < UNLOCK_FREE_ATTEMPTS {
                    return PasswordAttempt::WrongPassword;
                }
                let exponent = state.consecutive_wrong - UNLOCK_FREE_ATTEMPTS;
                let backoff = UNLOCK_BASE_BACKOFF
                    .saturating_mul(2_u32.saturating_pow(exponent))
                    .min(UNLOCK_MAX_BACKOFF);
                state.blocked_until = Some(self.scheduler.monotonic_now().saturating_add(backoff));
                PasswordAttempt::Throttled {
                    retry_after_secs: backoff.as_secs(),
                }
            }
            Err(UnlockFailure::InApp) => PasswordAttempt::InApp,
            Err(
                UnlockFailure::Cancelled
                | UnlockFailure::DeviceUnavailable
                | UnlockFailure::Failed
                | UnlockFailure::NotRecognized,
            ) => PasswordAttempt::Failed,
        }
    }

    pub(crate) fn device_attempt<T>(&self, try_unlock: impl FnOnce() -> T) -> Option<T> {
        // Hold this separate guard through the prompt. Rapid callers are dropped without
        // queuing another system prompt or blocking password attempts and session observers.
        let mut last = match self.last_device_prompt.try_lock() {
            Ok(last) => last,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let now = self.scheduler.monotonic_now();
        if last.is_some_and(|started| now.saturating_sub(started) < DEVICE_UNLOCK_DEBOUNCE) {
            return None;
        }
        *last = Some(now);
        Some(try_unlock())
    }

    /// The reset generation, read before a device prompt starts and handed back to
    /// [`Self::record_device_failure`].
    pub(crate) fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .generation
    }

    /// Counts a device prompt that did not recognize the user and returns the new count. An unlock
    /// that happened while the prompt ran (a new generation) supersedes it: nothing is counted.
    pub(crate) fn record_device_failure(&self, generation: u64) -> u32 {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.generation == generation {
            state.device_failures = state.device_failures.saturating_add(1);
        }
        state.device_failures
    }

    /// Device prompts from a browser that did not recognize the user in this lock spell.
    pub fn device_failures(&self) -> u32 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .device_failures
    }

    pub fn retry_after_secs(&self) -> Option<u64> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.retry_after_secs(self.scheduler.monotonic_now())
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        *state = State {
            generation: state.generation.wrapping_add(1),
            ..State::default()
        };
    }
}

/// A bridge without an app-core-backed unlocker.
pub struct NoUnlocker;

impl VaultUnlocker for NoUnlocker {
    fn unlock_with_password(&self, _password: SecretString) -> Result<(), UnlockFailure> {
        Err(UnlockFailure::Failed)
    }

    fn unlock_with_device(&self, _automatic: bool) -> Result<(), UnlockFailure> {
        Err(UnlockFailure::Failed)
    }

    fn device_offer(&self) -> DeviceOffer {
        DeviceOffer::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test-only
mod tests {
    use super::*;
    use mypass_session::scheduler::ManualScheduler;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Barrier;
    use std::time::SystemTime;

    fn fixture() -> (Arc<ManualScheduler>, UnlockThrottle) {
        let clock = ManualScheduler::new(SystemTime::UNIX_EPOCH);
        let throttle = UnlockThrottle::new(clock.clone());
        (clock, throttle)
    }

    fn wrong(throttle: &UnlockThrottle) -> PasswordAttempt {
        throttle.attempt(|| false, || Err(UnlockFailure::WrongPassword))
    }

    fn four_wrong(throttle: &UnlockThrottle) {
        for _ in 0..4 {
            assert_eq!(wrong(throttle), PasswordAttempt::WrongPassword);
        }
    }

    fn throttled(seconds: u64) -> PasswordAttempt {
        PasswordAttempt::Throttled {
            retry_after_secs: seconds,
        }
    }

    #[test]
    fn four_wrong_passwords_answer_wrong_password() {
        four_wrong(&fixture().1);
    }

    #[test]
    fn fifth_wrong_password_is_throttled_for_30_seconds() {
        let (_, throttle) = fixture();
        four_wrong(&throttle);
        assert_eq!(wrong(&throttle), throttled(30));
    }

    #[test]
    fn backoff_doubles_to_the_cap() {
        let (clock, throttle) = fixture();
        four_wrong(&throttle);
        for seconds in [30, 60, 120, 240, 480, 900, 900] {
            assert_eq!(wrong(&throttle), throttled(seconds));
            clock.advance(Duration::from_secs(seconds));
        }
    }

    #[test]
    fn throttled_attempt_never_reaches_the_unlocker() {
        let (clock, throttle) = fixture();
        four_wrong(&throttle);
        assert_eq!(wrong(&throttle), throttled(30));
        clock.advance(Duration::from_secs(10));
        assert_eq!(
            throttle.attempt(|| false, || panic!("attempt must not run")),
            throttled(20)
        );
        clock.advance(Duration::from_secs(20));
        assert_eq!(wrong(&throttle), throttled(60));
    }

    #[test]
    fn retry_after_rounds_up_and_is_at_least_one() {
        let (clock, throttle) = fixture();
        four_wrong(&throttle);
        assert_eq!(wrong(&throttle), throttled(30));
        clock.advance(Duration::from_millis(29_400));
        assert_eq!(throttle.retry_after_secs(), Some(1));
        assert_eq!(wrong(&throttle), throttled(1));
    }

    #[test]
    fn success_resets_the_count() {
        let (_, throttle) = fixture();
        four_wrong(&throttle);
        assert_eq!(
            throttle.attempt(|| false, || Ok(())),
            PasswordAttempt::Unlocked
        );
        four_wrong(&throttle);
    }

    #[test]
    fn reset_clears_a_running_backoff() {
        let (_, throttle) = fixture();
        four_wrong(&throttle);
        assert_eq!(wrong(&throttle), throttled(30));
        throttle.reset();
        assert_eq!(throttle.retry_after_secs(), None);
        four_wrong(&throttle);
    }

    #[test]
    fn successful_unlock_can_reset_from_a_synchronous_observer() {
        let (_, throttle) = fixture();
        four_wrong(&throttle);
        assert_eq!(
            throttle.attempt(
                || false,
                || {
                    throttle.reset();
                    Ok(())
                }
            ),
            PasswordAttempt::Unlocked
        );
        four_wrong(&throttle);
    }

    #[test]
    fn device_failures_count_up_and_reset_with_the_throttle() {
        let (_, throttle) = fixture();
        assert_eq!(throttle.device_failures(), 0);
        let generation = throttle.generation();
        assert_eq!(throttle.record_device_failure(generation), 1);
        assert_eq!(throttle.record_device_failure(generation), 2);
        assert_eq!(throttle.device_failures(), 2);
        throttle.reset();
        assert_eq!(throttle.device_failures(), 0);
    }

    #[test]
    fn device_failure_after_an_unlock_during_the_prompt_is_not_counted() {
        let (_, throttle) = fixture();
        let generation = throttle.generation();
        throttle.reset();
        assert_eq!(throttle.record_device_failure(generation), 0);
        assert_eq!(throttle.device_failures(), 0);
    }

    #[test]
    fn device_failures_never_touch_the_password_backoff() {
        let (_, throttle) = fixture();
        let generation = throttle.generation();
        for _ in 0..10 {
            throttle.record_device_failure(generation);
        }
        assert_eq!(throttle.retry_after_secs(), None);
        four_wrong(&throttle);
        assert_eq!(throttle.device_failures(), 10);
    }

    #[test]
    fn device_prompt_drops_concurrent_requests_even_after_the_debounce() {
        let (clock, throttle) = fixture();
        let (entered, prompt_started) = std::sync::mpsc::channel();
        let (release, wait_for_release) = std::sync::mpsc::channel();
        let (answer, concurrent_result) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let throttle = &throttle;
            scope.spawn(move || {
                assert_eq!(
                    throttle.device_attempt(|| {
                        entered.send(()).unwrap();
                        wait_for_release.recv().unwrap();
                    }),
                    Some(())
                );
            });
            prompt_started.recv().unwrap();
            clock.advance(DEVICE_UNLOCK_DEBOUNCE);
            scope.spawn(move || answer.send(throttle.device_attempt(|| ())).unwrap());
            let result = concurrent_result.recv_timeout(Duration::from_secs(5));
            release.send(()).unwrap();
            assert_eq!(result.unwrap(), None);
        });
        assert_eq!(throttle.device_attempt(|| ()), Some(()));
    }

    #[test]
    fn concurrent_wrong_passwords_are_serialised_and_counted() {
        let (_, throttle) = fixture();
        let start = Barrier::new(8);
        let calls = AtomicUsize::new(0);
        let active = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        throttle.attempt(
                            || false,
                            || {
                                assert_eq!(active.fetch_add(1, SeqCst), 0);
                                calls.fetch_add(1, SeqCst);
                                std::thread::yield_now();
                                active.fetch_sub(1, SeqCst);
                                Err(UnlockFailure::WrongPassword)
                            },
                        )
                    })
                })
                .collect();
            assert_eq!(
                threads
                    .into_iter()
                    .map(|t| t.join().unwrap())
                    .filter(|answer| *answer == throttled(30))
                    .count(),
                4
            );
        });
        assert_eq!(calls.load(SeqCst), 5);
    }

    #[test]
    fn vault_unlocked_while_an_attempt_waits_skips_the_unlocker_and_backoff() {
        let (_, throttle) = fixture();
        four_wrong(&throttle);
        assert_eq!(wrong(&throttle), throttled(30));
        assert_eq!(
            throttle.attempt(|| true, || panic!("unlocked vault must skip the unlocker")),
            PasswordAttempt::NotNeeded,
        );
        assert_eq!(throttle.retry_after_secs(), Some(30));
    }

    #[test]
    fn throttle_constants_are_pinned() {
        assert_eq!(UNLOCK_FREE_ATTEMPTS, 5);
        assert_eq!(UNLOCK_BASE_BACKOFF, Duration::from_secs(30));
        assert_eq!(UNLOCK_MAX_BACKOFF, Duration::from_secs(900));
    }
}
