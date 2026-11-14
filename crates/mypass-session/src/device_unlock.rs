//! Device unlock (Touch ID / the Mac account password): the unlock window policy.
//!
//! [`UnlockWindowPolicy`] decides whether device unlock may be offered, from when it and the real
//! master password were each last used. It is a port of the old app's `UnlockWindowPolicy`.

use crate::settings::{RequireAccountPasswordWindow, UnlockWithDeviceWindow};
use std::time::{Duration, SystemTime};

/// Whether device unlock may be offered now. Pure; the timestamps come from the Rust-only unlock
/// state store, never from `Settings`, so the webview cannot extend a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnlockWindowPolicy;

impl UnlockWindowPolicy {
    /// `true` when both events happened and each lies within its window (inclusive: exactly the
    /// window length is still allowed). A missing timestamp means "never happened" and always
    /// gives `false`, even for a `Never` window: an unlimited window only excuses the time passed
    /// since a real event, not the absence of one. A timestamp in the future (the clock went
    /// backwards) counts as elapsed 0.
    pub fn device_auth_allowed(
        now: SystemTime,
        last_device_unlock: Option<SystemTime>,
        last_full_password: Option<SystemTime>,
        device: UnlockWithDeviceWindow,
        account: RequireAccountPasswordWindow,
    ) -> bool {
        let (Some(device_at), Some(password_at)) = (last_device_unlock, last_full_password) else {
            return false;
        };
        within(now, device_at, device.seconds()) && within(now, password_at, account.seconds())
    }
}

/// `now - at <= window`; a `None` window never expires; `at` after `now` is elapsed 0.
fn within(now: SystemTime, at: SystemTime, window: Option<u64>) -> bool {
    let Some(window) = window else {
        return true;
    };
    let elapsed = now.duration_since(at).unwrap_or(Duration::ZERO);
    elapsed <= Duration::from_secs(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    use RequireAccountPasswordWindow as A;
    use UnlockWithDeviceWindow as D;

    const HOUR: u64 = 3600;
    const DAY: u64 = 86_400;

    fn t(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
    }

    fn allowed(now: u64, device_at: u64, password_at: u64, d: D, a: A) -> bool {
        UnlockWindowPolicy::device_auth_allowed(
            t(now),
            Some(t(device_at)),
            Some(t(password_at)),
            d,
            a,
        )
    }

    #[test]
    fn device_window_boundary_is_inclusive() {
        assert!(allowed(8 * HOUR, 0, 0, D::EightHours, A::ThirtyDays));
        assert!(!allowed(8 * HOUR + 1, 0, 0, D::EightHours, A::ThirtyDays));
        // Exactly 30 days on the account window, with a fresh device unlock.
        assert!(allowed(30 * DAY, 30 * DAY, 0, D::EightHours, A::ThirtyDays));
        assert!(!allowed(
            30 * DAY + 1,
            30 * DAY,
            0,
            D::EightHours,
            A::ThirtyDays
        ));
    }

    #[test]
    fn account_window_expiry_blocks_even_inside_device_window() {
        // Device unlock a minute ago, master password 31 days ago.
        assert!(!allowed(
            31 * DAY,
            31 * DAY - 60,
            0,
            D::TwentyFourHours,
            A::ThirtyDays
        ));
        assert!(!allowed(2 * DAY, 2 * DAY, 0, D::Never, A::OneDay));
    }

    #[test]
    fn missing_timestamp_never_allows_even_when_never() {
        let now = t(10);
        for (device_at, password_at) in [(None, Some(t(0))), (Some(t(0)), None), (None, None)] {
            assert!(
                !UnlockWindowPolicy::device_auth_allowed(
                    now,
                    device_at,
                    password_at,
                    D::Never,
                    A::Never
                ),
                "{device_at:?} {password_at:?}"
            );
        }
    }

    #[test]
    fn never_windows_allow_any_elapsed_with_real_events() {
        assert!(allowed(10_000 * DAY, 0, 0, D::Never, A::Never));
        assert!(allowed(
            89 * DAY,
            89 * DAY - HOUR,
            0,
            D::OneHour,
            A::NinetyDays
        ));
    }

    #[test]
    fn clock_going_backwards_does_not_panic_and_allows() {
        // Both events are dated after `now`: elapsed counts as 0 for every window.
        for d in [D::OneHour, D::Never] {
            for a in [A::OneDay, A::Never] {
                assert!(allowed(0, DAY, 2 * DAY, d, a), "{d:?} {a:?}");
            }
        }
        assert!(UnlockWindowPolicy::device_auth_allowed(
            UNIX_EPOCH,
            Some(t(0)),
            Some(t(0)),
            D::OneHour,
            A::OneDay
        ));
    }
}
