//! Auto-lock: the idle timer and the system events (sleep, wake, screen lock) that lock the vault.
//!
//! The idle timer runs on the [`Scheduler`]'s monotonic timeline, which on macOS does not advance
//! while the machine sleeps. After `Wake` the rule is therefore checked against the WALL clock
//! (`wall_now - last_activity >= timeout`), the only reliable measure across a sleep.
//!
//! Rules:
//! - Unlocking (any `StateChanged` to `Unlocked`) counts as activity and arms the timer; locking
//!   cancels it. A timer job that runs although it was cancelled (a cancelled job may still run
//!   once) re-checks the session state and the wall-clock idle time before locking.
//! - [`AutoLock::record_activity`] is for deliberate user activity only and is a no-op while
//!   locked (it never arms a timer then).
//! - `Sleep` locks when `lockOnDeviceSleep`, `ScreenLocked` locks when `lockOnScreenLock` (the
//!   old app's screen-lock bug fix), `Wake` locks with reason `Idle` when the idle time has run
//!   out during the sleep, `ScreenUnlocked` does nothing (it never unlocks the vault and is not
//!   activity). The `Never` timeout arms nothing.
//!
//! Threading: platform event sinks run on the main thread and must never block, but
//! `VaultSession::lock` waits for an in-flight save. [`AutoLock::event_sink`] (and
//! [`AutoLock::subscribe`]) therefore only hand the event to the scheduler (a zero-delay job);
//! [`AutoLock::on_system_event`] is the synchronous form for that job and for tests. The
//! auto-lock's own mutexes are never held while calling the session; lock order inside is
//! `tick` then `last_activity`, and the settings lock is taken alone.

use crate::scheduler::{lock, Scheduler, TaskHandle};
use crate::session::{LockReason, SessionEvent, SessionState, VaultSession};
use crate::settings::SettingsStore;
use mypass_platform::{PlatformResult, Subscription, SystemEvent, SystemEvents};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime};

struct Inner {
    session: VaultSession,
    settings: SettingsStore,
    scheduler: Arc<dyn Scheduler>,
    last_activity: Mutex<SystemTime>,
    tick: Mutex<Option<TaskHandle>>,
}

/// The auto-lock service (cheap to clone; every clone is the same service).
#[derive(Clone)]
pub struct AutoLock(Arc<Inner>);

impl std::fmt::Debug for AutoLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoLock")
            .field("armed", &lock(&self.0.tick).is_some())
            .finish_non_exhaustive()
    }
}

impl AutoLock {
    /// Registers a session observer (`Unlocked` arms the timer, `Locked`/`NoVault` cancels it)
    /// and arms the timer at once when the session is already unlocked.
    pub fn new(
        session: VaultSession,
        settings: SettingsStore,
        scheduler: Arc<dyn Scheduler>,
    ) -> Self {
        let now = scheduler.wall_now();
        let me = Self(Arc::new(Inner {
            session,
            settings,
            scheduler,
            last_activity: Mutex::new(now),
            tick: Mutex::new(None),
        }));
        let weak = Arc::downgrade(&me.0);
        me.0.session.add_observer(Arc::new(move |e: &SessionEvent| {
            if let Some(inner) = weak.upgrade() {
                AutoLock(inner).on_session_event(e);
            }
        }));
        if me.0.session.is_unlocked() {
            me.arm();
        }
        me
    }

    fn weak(&self) -> Weak<Inner> {
        Arc::downgrade(&self.0)
    }

    fn on_session_event(&self, e: &SessionEvent) {
        if let SessionEvent::StateChanged(s) = e {
            if s.state == SessionState::Unlocked {
                *lock(&self.0.last_activity) = self.0.scheduler.wall_now();
                self.arm();
            } else {
                self.disarm();
            }
        }
    }

    fn timeout(&self) -> Option<Duration> {
        self.0
            .settings
            .get()
            .auto_lock_idle_timeout
            .seconds()
            .map(Duration::from_secs)
    }

    fn idle_for(&self) -> Duration {
        let last = *lock(&self.0.last_activity);
        self.0
            .scheduler
            .wall_now()
            .duration_since(last)
            .unwrap_or(Duration::ZERO)
    }

    fn disarm(&self) {
        if let Some(h) = lock(&self.0.tick).take() {
            h.cancel();
        }
    }

    /// (Re)arms the idle timer for the time left until the timeout.
    fn arm(&self) {
        let timeout = self.timeout();
        let mut tick = lock(&self.0.tick);
        if let Some(h) = tick.take() {
            h.cancel();
        }
        let Some(timeout) = timeout else {
            return;
        };
        let delay = timeout.saturating_sub(self.idle_for());
        let weak = self.weak();
        let job = Box::new(move || {
            if let Some(inner) = weak.upgrade() {
                AutoLock(inner).on_tick();
            }
        });
        *tick = Some(self.0.scheduler.schedule(delay, job));
    }

    fn on_tick(&self) {
        if !self.0.session.is_unlocked() {
            return;
        }
        match self.timeout() {
            Some(t) if self.idle_for() >= t => self.0.session.lock(LockReason::Idle),
            Some(_) => self.arm(),
            None => {}
        }
    }

    /// Deliberate user activity: resets the idle time and re-arms the timer. No-op while locked.
    pub fn record_activity(&self) {
        if !self.0.session.is_unlocked() {
            return;
        }
        *lock(&self.0.last_activity) = self.0.scheduler.wall_now();
        self.arm();
    }

    /// Re-arms the timer with the current `autoLockIdleTimeout` (the idle time so far counts).
    pub fn settings_changed(&self) {
        if self.0.session.is_unlocked() {
            self.arm();
        } else {
            self.disarm();
        }
    }

    /// Applies a system event synchronously. May block for an in-flight save (through
    /// `VaultSession::lock`): never call it from a platform sink, use [`AutoLock::event_sink`].
    pub fn on_system_event(&self, ev: SystemEvent) {
        let settings = self.0.settings.get();
        match ev {
            SystemEvent::Sleep if settings.lock_on_device_sleep => {
                self.0.session.lock(LockReason::Sleep)
            }
            SystemEvent::ScreenLocked if settings.lock_on_screen_lock => {
                self.0.session.lock(LockReason::ScreenLock)
            }
            SystemEvent::Wake => {
                if !self.0.session.is_unlocked() {
                    return;
                }
                match self.timeout() {
                    Some(t) if self.idle_for() >= t => self.0.session.lock(LockReason::Idle),
                    // The monotonic timer paused during the sleep: re-arm for the time left.
                    Some(_) => self.arm(),
                    None => {}
                }
            }
            SystemEvent::Sleep | SystemEvent::ScreenLocked | SystemEvent::ScreenUnlocked => {}
        }
    }

    /// A sink for `SystemEvents::subscribe`: it never blocks and never panics, it only schedules
    /// [`AutoLock::on_system_event`] as a zero-delay job.
    pub fn event_sink(&self) -> Arc<dyn Fn(SystemEvent) + Send + Sync> {
        let weak = self.weak();
        let scheduler = self.0.scheduler.clone();
        Arc::new(move |ev: SystemEvent| {
            let weak = weak.clone();
            scheduler.schedule(
                Duration::ZERO,
                Box::new(move || {
                    if let Some(inner) = weak.upgrade() {
                        AutoLock(inner).on_system_event(ev);
                    }
                }),
            );
        })
    }

    /// Subscribes [`AutoLock::event_sink`] to `events`; keep the subscription alive.
    pub fn subscribe(&self, events: &dyn SystemEvents) -> PlatformResult<Subscription> {
        events.subscribe(self.event_sink())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::ManualScheduler;
    use crate::session::SessionDeps;
    use crate::settings::{IdleTimeout, SettingsPatch};
    use crate::testing::fast_database;
    use mypass_kdbx::PlainFileIo;
    use mypass_platform::testing::FakeSystemEvents;
    use secrecy::SecretString;
    use std::time::UNIX_EPOCH;
    use tempfile::TempDir;

    const MIN: u64 = 60;

    struct Fx {
        _dir: TempDir,
        session: VaultSession,
        settings: SettingsStore,
        clock: Arc<ManualScheduler>,
        auto: AutoLock,
    }

    fn pw() -> SecretString {
        SecretString::from("pw".to_owned())
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let settings = SettingsStore::in_memory();
        let clock = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let session = VaultSession::new(SessionDeps {
            settings: settings.clone(),
            scheduler: clock.clone(),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        let auto = AutoLock::new(session.clone(), settings.clone(), clock.clone());
        session
            .create_vault_with(&dir.path().join("v.kdbx"), pw(), fast_database("v"))
            .unwrap();
        Fx {
            _dir: dir,
            session,
            settings,
            clock,
            auto,
        }
    }

    impl Fx {
        fn advance(&self, secs: u64) {
            self.clock.advance(Duration::from_secs(secs));
        }
        fn locked_for(&self) -> Option<LockReason> {
            let s = self.session.status();
            (s.state == SessionState::Locked)
                .then_some(s.lock_reason)
                .flatten()
        }
        fn set(&self, patch: SettingsPatch) {
            self.settings.update(patch).unwrap();
            self.auto.settings_changed();
        }
        fn timeout(&self, t: IdleTimeout) {
            self.set(SettingsPatch {
                auto_lock_idle_timeout: Some(t),
                ..SettingsPatch::default()
            });
        }
    }

    #[test]
    fn idle_timeout_locks_after_configured_time() {
        let f = fx();
        f.advance(30 * MIN - 1);
        assert!(f.session.is_unlocked());
        f.advance(1);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn activity_resets_the_timer() {
        let f = fx();
        f.advance(20 * MIN);
        f.auto.record_activity();
        f.advance(20 * MIN);
        assert!(
            f.session.is_unlocked(),
            "40 min after unlock, 20 after activity"
        );
        f.advance(10 * MIN);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn immediately_option_locks_after_two_seconds() {
        let f = fx();
        f.timeout(IdleTimeout::Immediately);
        f.advance(1);
        assert!(f.session.is_unlocked());
        f.advance(1);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn never_option_never_locks() {
        let f = fx();
        f.timeout(IdleTimeout::Never);
        assert_eq!(f.clock.pending(), 0);
        f.advance(30 * 24 * 60 * MIN);
        assert!(f.session.is_unlocked());
        f.auto.on_system_event(SystemEvent::Wake);
        assert!(f.session.is_unlocked());
    }

    #[test]
    fn settings_changed_rearms_with_new_timeout() {
        let f = fx();
        f.advance(2 * MIN);
        f.timeout(IdleTimeout::FiveMinutes);
        f.advance(3 * MIN - 1);
        assert!(f.session.is_unlocked());
        f.advance(1);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn longer_timeout_after_change_extends_the_timer() {
        let f = fx();
        f.timeout(IdleTimeout::OneHour);
        f.advance(59 * MIN);
        assert!(f.session.is_unlocked());
        f.advance(MIN);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn sleep_locks_when_enabled() {
        let f = fx();
        f.auto.on_system_event(SystemEvent::Sleep);
        assert_eq!(f.locked_for(), Some(LockReason::Sleep));
    }

    #[test]
    fn sleep_does_nothing_when_disabled() {
        let f = fx();
        f.set(SettingsPatch {
            lock_on_device_sleep: Some(false),
            ..SettingsPatch::default()
        });
        f.auto.on_system_event(SystemEvent::Sleep);
        assert!(f.session.is_unlocked());
    }

    #[test]
    fn screen_lock_locks_when_enabled() {
        let f = fx();
        f.auto.on_system_event(SystemEvent::ScreenLocked);
        assert_eq!(f.locked_for(), Some(LockReason::ScreenLock));
    }

    #[test]
    fn screen_lock_ignored_when_disabled() {
        let f = fx();
        f.set(SettingsPatch {
            lock_on_screen_lock: Some(false),
            ..SettingsPatch::default()
        });
        f.auto.on_system_event(SystemEvent::ScreenLocked);
        assert!(f.session.is_unlocked());
    }

    #[test]
    fn screen_unlock_event_does_not_unlock_or_count_as_activity() {
        let f = fx();
        f.advance(20 * MIN);
        f.auto.on_system_event(SystemEvent::ScreenUnlocked);
        f.advance(10 * MIN);
        assert_eq!(f.locked_for(), Some(LockReason::Idle), "not activity");
        f.auto.on_system_event(SystemEvent::ScreenUnlocked);
        assert_eq!(f.locked_for(), Some(LockReason::Idle), "never unlocks");
    }

    #[test]
    fn wake_after_long_sleep_locks_when_idle_exceeded() {
        let f = fx();
        f.clock.jump_wall_clock(Duration::from_secs(45 * MIN));
        assert!(f.session.is_unlocked(), "no job ran during the sleep");
        f.auto.on_system_event(SystemEvent::Wake);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn wake_after_short_sleep_keeps_unlocked() {
        let f = fx();
        f.clock.jump_wall_clock(Duration::from_secs(5 * MIN));
        f.auto.on_system_event(SystemEvent::Wake);
        assert!(f.session.is_unlocked());
        // The timer is re-armed for the time left (25 min of monotonic time).
        f.advance(25 * MIN - 1);
        assert!(f.session.is_unlocked());
        f.advance(1);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn activity_while_locked_does_not_arm_a_timer() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(f.clock.pending(), 0);
        f.auto.record_activity();
        f.auto.settings_changed();
        assert_eq!(f.clock.pending(), 0);
    }

    #[test]
    fn lock_cancels_pending_tick() {
        let f = fx();
        assert_eq!(f.clock.pending(), 1);
        f.session.lock(LockReason::Manual);
        assert_eq!(f.clock.pending(), 0);
    }

    #[test]
    fn unlock_arms_timer_again() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.advance(60 * MIN);
        f.session.unlock(pw()).unwrap();
        assert_eq!(f.clock.pending(), 1);
        f.advance(30 * MIN - 1);
        assert!(f.session.is_unlocked());
        f.advance(1);
        assert_eq!(f.locked_for(), Some(LockReason::Idle));
    }

    #[test]
    fn cancelled_tick_that_still_runs_rechecks_idle_time() {
        let f = fx();
        f.advance(10 * MIN);
        f.auto.record_activity();
        // Simulate the stale tick running anyway: it must not lock (only 0 s idle).
        f.auto.on_tick();
        assert!(f.session.is_unlocked());
        assert_eq!(f.clock.pending(), 1);
    }

    #[test]
    fn event_sink_hands_events_to_the_scheduler() {
        let f = fx();
        let events = FakeSystemEvents::new();
        let sub = f.auto.subscribe(events.as_ref()).unwrap();
        events.emit(SystemEvent::ScreenLocked);
        assert!(f.session.is_unlocked(), "the sink itself never locks");
        f.clock.advance(Duration::ZERO);
        assert_eq!(f.locked_for(), Some(LockReason::ScreenLock));
        drop(sub);
        assert_eq!(events.subscriber_count(), 0);
    }

    #[test]
    fn dropped_auto_lock_stops_acting() {
        let Fx {
            _dir,
            session,
            clock,
            auto,
            ..
        } = fx();
        let sink = auto.event_sink();
        drop(auto);
        sink(SystemEvent::ScreenLocked);
        clock.advance(Duration::from_secs(31 * MIN));
        assert!(session.is_unlocked());
    }
}
