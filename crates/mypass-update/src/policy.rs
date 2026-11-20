//! The pure decision logic of the update check: when a check runs, what its outcome does to the
//! status and the banner, what dismissing and disabling do, and when the schedule fires.
//!
//! Port of the rules of the old app's `AppUpdateChecker`, with no clock, no thread, no settings and
//! no I/O: the session's `UpdateService` owns those (scheduler, settings, state store, observers)
//! and asks [`UpdatePolicy`] what to do. Every rule below is therefore tested without a scheduler.

use crate::asset::{checksum_asset_for, select_asset, ReleasePlatform};
use crate::feed::{ReleaseFeedError, ReleaseInfo};
use crate::version::SemanticVersion;
use std::time::Duration;

/// The first scheduled check runs this long after the schedule starts.
pub const INITIAL_DELAY: Duration = Duration::from_secs(10);
/// Every later scheduled check runs this long after the previous one fired. A failure does not
/// shorten it: there is no retry before the next interval, so an outage cannot cause a retry storm.
pub const INTERVAL: Duration = Duration::from_secs(86_400);

/// Why a check failed, as a kind. The UI maps a kind to localised text; raw error text, which can
/// carry an HTTP status line or a server string, never reaches the webview.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UpdateErrorKind {
    /// The network, DNS, TLS or the timeout failed.
    Offline,
    /// GitHub answered 403 or 429.
    RateLimited,
    /// GitHub answered with any other status than 200.
    BadStatus,
    /// The answer was not a release this app accepts.
    Unexpected,
    /// The running version could not be parsed, so there is nothing to compare with.
    UnknownVersion,
}

impl From<&ReleaseFeedError> for UpdateErrorKind {
    fn from(error: &ReleaseFeedError) -> Self {
        match error {
            ReleaseFeedError::Offline => Self::Offline,
            ReleaseFeedError::BadStatus(403 | 429) => Self::RateLimited,
            ReleaseFeedError::BadStatus(_) => Self::BadStatus,
            ReleaseFeedError::Malformed | ReleaseFeedError::UnparsableVersion(_) => {
                Self::Unexpected
            }
        }
    }
}

/// The state of the update check, as the settings pane and the About pane see it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    /// The running version is the newest (or newer); `checked_at` is unix seconds.
    UpToDate {
        checked_at: i64,
    },
    /// A newer release exists (the banner may still be hidden by a dismissal).
    Available(ReleaseInfo),
    Failed(UpdateErrorKind),
}

/// What to do when a check is requested.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BeginDecision {
    /// Do nothing: a scheduled check with the toggle off, or a scheduled check that cannot compare.
    Skip,
    /// A manual check that cannot start: set `Failed(kind)` (always `UnknownVersion`).
    Fail(UpdateErrorKind),
    /// A check is already running: this caller returns at once. `upgrade_to_manual` is true when
    /// this caller is a manual press during a scheduled check, which must then surface a failure.
    AlreadyRunning { upgrade_to_manual: bool },
    /// Start the check: remember the previous status, set `Checking`, run the feed.
    Run,
}

/// What a finished check does to the banner.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BannerChange {
    /// Show this release.
    Show(ReleaseInfo),
    /// Hide the banner.
    Clear,
    /// Leave the banner as it is (a failure never hides or raises it).
    Keep,
}

/// The result of a finished check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckOutcome {
    pub status: UpdateStatus,
    pub banner: BannerChange,
}

/// The inputs of [`UpdatePolicy::finish`].
#[derive(Clone, Copy, Debug)]
pub struct FinishInput<'a> {
    /// What the feed returned.
    pub result: &'a Result<ReleaseInfo, ReleaseFeedError>,
    /// The running version.
    pub current: SemanticVersion,
    /// The build target, or None when no artifact is published for it.
    pub platform: Option<ReleasePlatform>,
    /// The status before the check began (restored by a failed scheduled check).
    pub previous: &'a UpdateStatus,
    /// Whether the check counts as manual, including an upgrade by a second caller.
    pub manual: bool,
    /// The persisted dismissed version string.
    pub dismissed_version: Option<&'a str>,
    /// The wall clock in unix seconds.
    pub now: i64,
}

/// What disabling or enabling the automatic check does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EnabledChange {
    pub clear_banner: bool,
    pub status: UpdateStatus,
}

/// The pure rules; no state of its own.
#[derive(Debug, Clone, Copy)]
pub struct UpdatePolicy;

impl UpdatePolicy {
    /// Decides what a request for a check does.
    ///
    /// A manual check runs even when the toggle is off (it is the user's own request); the
    /// schedule honours the toggle. An unparsable running version skips a scheduled check and
    /// fails a manual one. `in_flight` is `Some(is_manual)` while a check runs: a second caller
    /// returns at once, and a manual one upgrades a scheduled run so a failure is surfaced to the
    /// user who asked.
    pub fn begin(
        manual: bool,
        enabled: bool,
        current: Option<SemanticVersion>,
        in_flight: Option<bool>,
    ) -> BeginDecision {
        if !manual && !enabled {
            return BeginDecision::Skip;
        }
        if current.is_none() {
            return if manual {
                BeginDecision::Fail(UpdateErrorKind::UnknownVersion)
            } else {
                BeginDecision::Skip
            };
        }
        match in_flight {
            Some(running_is_manual) => BeginDecision::AlreadyRunning {
                upgrade_to_manual: manual && !running_is_manual,
            },
            None => BeginDecision::Run,
        }
    }

    /// Applies a finished check.
    ///
    /// - A newer release sets `Available`; the banner is shown unless its version string equals
    ///   the dismissed one (equality, not order: a release newer than the dismissed one always
    ///   brings the banner back).
    /// - A missing target payload/checksum, unsupported target, equal/older release or prerelease
    ///   offered to a stable build sets `UpToDate` and hides the banner.
    /// - A failed manual check sets `Failed(kind)`; a failed scheduled check silently restores the
    ///   previous status. Neither touches the banner.
    ///
    /// The caller writes `last_run` after every attempt, failures included.
    pub fn finish(input: FinishInput<'_>) -> CheckOutcome {
        match input.result {
            Ok(release)
                if release.version > input.current
                    && (input.current.is_prerelease() || !release.version.is_prerelease())
                    && input
                        .platform
                        .and_then(|platform| {
                            let asset = select_asset(&release.assets, platform, &release.version)?;
                            checksum_asset_for(asset, &release.assets)
                        })
                        .is_some() =>
            {
                let dismissed = input
                    .dismissed_version
                    .is_some_and(|d| d == release.version.to_string());
                CheckOutcome {
                    status: UpdateStatus::Available(release.clone()),
                    banner: if dismissed {
                        BannerChange::Clear
                    } else {
                        BannerChange::Show(release.clone())
                    },
                }
            }
            Ok(_) => CheckOutcome {
                status: UpdateStatus::UpToDate {
                    checked_at: input.now,
                },
                banner: BannerChange::Clear,
            },
            Err(error) => CheckOutcome {
                status: if input.manual {
                    UpdateStatus::Failed(UpdateErrorKind::from(error))
                } else {
                    input.previous.clone()
                },
                banner: BannerChange::Keep,
            },
        }
    }

    /// The version string to persist when the user dismisses the banner: the version the banner
    /// is actually showing, not whatever the status says, because the banner stays up through a
    /// failed manual check and a running check and "Later" must stick in those states too.
    pub fn dismissal(banner: Option<&ReleaseInfo>) -> Option<String> {
        banner.map(|release| release.version.to_string())
    }

    /// What a change of the automatic-check toggle does. Turning it on changes nothing; turning it
    /// off hides the banner and resets an `Available` or `Failed` status to `Idle`.
    pub fn enabled_changed(enabled: bool, status: &UpdateStatus) -> EnabledChange {
        if enabled {
            return EnabledChange {
                clear_banner: false,
                status: status.clone(),
            };
        }
        let status = match status {
            UpdateStatus::Available(_) | UpdateStatus::Failed(_) => UpdateStatus::Idle,
            other => other.clone(),
        };
        EnabledChange {
            clear_banner: true,
            status,
        }
    }

    /// The delay before the next scheduled tick: [`INITIAL_DELAY`] before the first one,
    /// [`INTERVAL`] before every later one, whatever the previous tick did.
    pub fn next_delay(ticks_fired: u64) -> Duration {
        if ticks_fired == 0 {
            INITIAL_DELAY
        } else {
            INTERVAL
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> SemanticVersion {
        SemanticVersion::parse(s).unwrap()
    }

    fn release(version: &str) -> ReleaseInfo {
        ReleaseInfo {
            assets: vec![
                crate::ReleaseAsset {
                    name: format!("MyPass-{version}-macos-arm64.dmg"),
                },
                crate::ReleaseAsset {
                    name: "SHA256SUMS.txt".into(),
                },
            ],
            version: v(version),
            tag: format!("v{version}"),
            url: format!("https://github.com/slucheninov/mypass-app/releases/tag/v{version}"),
        }
    }

    /// A small model of the service: the policy plus the three pieces of state the session owns.
    struct Model {
        status: UpdateStatus,
        banner: Option<ReleaseInfo>,
        dismissed: Option<String>,
        last_run: Option<i64>,
        enabled: bool,
        current: Option<SemanticVersion>,
        in_flight: Option<bool>,
        now: i64,
    }

    impl Model {
        fn new(current: &str) -> Self {
            Self {
                status: UpdateStatus::Idle,
                banner: None,
                dismissed: None,
                last_run: None,
                enabled: true,
                current: Some(v(current)),
                in_flight: None,
                now: 1_800_000_000,
            }
        }

        /// A complete check against `result` (the feed call is instantaneous here).
        fn check(&mut self, manual: bool, result: Result<ReleaseInfo, ReleaseFeedError>) {
            match UpdatePolicy::begin(manual, self.enabled, self.current, self.in_flight) {
                BeginDecision::Skip | BeginDecision::AlreadyRunning { .. } => {}
                BeginDecision::Fail(kind) => self.status = UpdateStatus::Failed(kind),
                BeginDecision::Run => {
                    let previous = self.status.clone();
                    self.status = UpdateStatus::Checking;
                    self.in_flight = Some(manual);
                    let manual = self.in_flight.unwrap_or(manual);
                    self.finish(&result, &previous, manual);
                }
            }
        }

        fn finish(
            &mut self,
            result: &Result<ReleaseInfo, ReleaseFeedError>,
            previous: &UpdateStatus,
            manual: bool,
        ) {
            let outcome = UpdatePolicy::finish(FinishInput {
                result,
                current: self.current.unwrap(),
                platform: Some(ReleasePlatform::MacosArm64),
                previous,
                manual,
                dismissed_version: self.dismissed.as_deref(),
                now: self.now,
            });
            self.status = outcome.status;
            match outcome.banner {
                BannerChange::Show(r) => self.banner = Some(r),
                BannerChange::Clear => self.banner = None,
                BannerChange::Keep => {}
            }
            self.last_run = Some(self.now);
            self.in_flight = None;
        }

        fn dismiss(&mut self) {
            if let Some(version) = UpdatePolicy::dismissal(self.banner.as_ref()) {
                self.dismissed = Some(version);
            }
            self.banner = None;
        }

        fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
            let change = UpdatePolicy::enabled_changed(enabled, &self.status);
            if change.clear_banner {
                self.banner = None;
            }
            self.status = change.status;
        }
    }

    #[test]
    fn newer_version_raises_banner() {
        let mut m = Model::new("0.7.0");
        m.check(false, Ok(release("0.7.1")));
        assert_eq!(m.status, UpdateStatus::Available(release("0.7.1")));
        assert_eq!(m.banner, Some(release("0.7.1")));
        assert_eq!(m.last_run, Some(m.now));
    }

    #[test]
    fn equal_or_older_version_is_up_to_date() {
        for latest in ["0.7.0", "0.6.9", "0.1.0"] {
            let mut m = Model::new("0.7.0");
            m.banner = Some(release("0.8.0"));
            m.check(false, Ok(release(latest)));
            assert_eq!(m.status, UpdateStatus::UpToDate { checked_at: m.now });
            assert_eq!(m.banner, None, "an up-to-date answer hides the banner");
        }
    }

    #[test]
    fn dismissed_version_hides_banner_until_newer_release() {
        let mut m = Model::new("0.7.0");
        m.check(false, Ok(release("0.7.1")));
        m.dismiss();
        assert_eq!(m.dismissed.as_deref(), Some("0.7.1"));
        m.check(false, Ok(release("0.7.1")));
        assert_eq!(m.banner, None, "the dismissed version stays hidden");
        assert_eq!(
            m.status,
            UpdateStatus::Available(release("0.7.1")),
            "the status still reports the release"
        );
        m.check(false, Ok(release("0.7.2")));
        assert_eq!(
            m.banner,
            Some(release("0.7.2")),
            "a newer release returns it"
        );
    }

    #[test]
    fn dismissal_compares_strings_not_order() {
        // Dismissed 0.9.0, then the feed (oddly) reports an older but still newer-than-current
        // 0.8.0: it is not the dismissed string, so it shows.
        let mut m = Model::new("0.7.0");
        m.dismissed = Some("0.9.0".into());
        m.check(false, Ok(release("0.8.0")));
        assert_eq!(m.banner, Some(release("0.8.0")));
    }

    #[test]
    fn dismiss_persists_the_banner_version_even_during_a_failed_manual_check() {
        let mut m = Model::new("0.7.0");
        m.check(false, Ok(release("0.7.1")));
        m.check(true, Err(ReleaseFeedError::Offline));
        assert_eq!(m.status, UpdateStatus::Failed(UpdateErrorKind::Offline));
        assert_eq!(
            m.banner,
            Some(release("0.7.1")),
            "the banner survives the failure"
        );
        m.dismiss();
        assert_eq!(m.dismissed.as_deref(), Some("0.7.1"));
        assert_eq!(
            UpdatePolicy::dismissal(None),
            None,
            "nothing shown, nothing stored"
        );
    }

    #[test]
    fn scheduled_check_honours_disabled_toggle() {
        let mut m = Model::new("0.7.0");
        m.enabled = false;
        m.check(false, Ok(release("0.7.1")));
        assert_eq!(m.status, UpdateStatus::Idle);
        assert_eq!(m.banner, None);
        assert_eq!(m.last_run, None, "a skipped check is not an attempt");
        assert_eq!(
            UpdatePolicy::begin(false, false, Some(v("0.7.0")), None),
            BeginDecision::Skip
        );
    }

    #[test]
    fn manual_check_runs_when_disabled() {
        let mut m = Model::new("0.7.0");
        m.enabled = false;
        m.check(true, Ok(release("0.7.1")));
        assert_eq!(m.status, UpdateStatus::Available(release("0.7.1")));
        assert_eq!(m.banner, Some(release("0.7.1")));
    }

    #[test]
    fn in_flight_guard_ignores_second_caller_and_upgrades_to_manual() {
        let current = Some(v("0.7.0"));
        // A manual press during a scheduled run upgrades it.
        assert_eq!(
            UpdatePolicy::begin(true, true, current, Some(false)),
            BeginDecision::AlreadyRunning {
                upgrade_to_manual: true
            }
        );
        // A scheduled tick during a manual run, or a second manual press, changes nothing.
        assert_eq!(
            UpdatePolicy::begin(false, true, current, Some(true)),
            BeginDecision::AlreadyRunning {
                upgrade_to_manual: false
            }
        );
        assert_eq!(
            UpdatePolicy::begin(true, true, current, Some(true)),
            BeginDecision::AlreadyRunning {
                upgrade_to_manual: false
            }
        );
        // The upgraded run then surfaces its failure.
        let previous = UpdateStatus::UpToDate { checked_at: 1 };
        let outcome = UpdatePolicy::finish(FinishInput {
            result: &Err(ReleaseFeedError::Offline),
            current: v("0.7.0"),
            platform: Some(ReleasePlatform::MacosArm64),
            previous: &previous,
            manual: true,
            dismissed_version: None,
            now: 2,
        });
        assert_eq!(
            outcome.status,
            UpdateStatus::Failed(UpdateErrorKind::Offline)
        );
    }

    #[test]
    fn scheduled_offline_failure_keeps_previous_status_silently() {
        let mut m = Model::new("0.7.0");
        m.check(false, Ok(release("0.7.1")));
        let (status, banner) = (m.status.clone(), m.banner.clone());
        m.now += 86_400;
        m.check(false, Err(ReleaseFeedError::Offline));
        assert_eq!(m.status, status, "no Failed state from a scheduled check");
        assert_eq!(m.banner, banner);

        let mut idle = Model::new("0.7.0");
        idle.check(false, Err(ReleaseFeedError::BadStatus(500)));
        assert_eq!(idle.status, UpdateStatus::Idle);
    }

    #[test]
    fn manual_offline_failure_reports_offline() {
        let mut m = Model::new("0.7.0");
        m.check(true, Err(ReleaseFeedError::Offline));
        assert_eq!(m.status, UpdateStatus::Failed(UpdateErrorKind::Offline));
    }

    #[test]
    fn failure_kinds_map_from_feed_errors() {
        let kind = |e: ReleaseFeedError| UpdateErrorKind::from(&e);
        assert_eq!(
            kind(ReleaseFeedError::BadStatus(403)),
            UpdateErrorKind::RateLimited
        );
        assert_eq!(
            kind(ReleaseFeedError::BadStatus(429)),
            UpdateErrorKind::RateLimited
        );
        assert_eq!(
            kind(ReleaseFeedError::BadStatus(404)),
            UpdateErrorKind::BadStatus
        );
        assert_eq!(
            kind(ReleaseFeedError::BadStatus(503)),
            UpdateErrorKind::BadStatus
        );
        assert_eq!(
            kind(ReleaseFeedError::Malformed),
            UpdateErrorKind::Unexpected
        );
        assert_eq!(
            kind(ReleaseFeedError::UnparsableVersion("x".into())),
            UpdateErrorKind::Unexpected
        );
        assert_eq!(kind(ReleaseFeedError::Offline), UpdateErrorKind::Offline);
        let mut m = Model::new("0.7.0");
        m.check(true, Err(ReleaseFeedError::BadStatus(429)));
        assert_eq!(m.status, UpdateStatus::Failed(UpdateErrorKind::RateLimited));
    }

    #[test]
    fn last_run_is_recorded_for_a_failed_attempt() {
        let mut m = Model::new("0.7.0");
        m.check(false, Err(ReleaseFeedError::Offline));
        assert_eq!(m.last_run, Some(m.now), "the caller stamps every attempt");
    }

    #[test]
    fn disabling_clears_banner_and_failed_status() {
        let mut m = Model::new("0.7.0");
        m.check(false, Ok(release("0.7.1")));
        m.set_enabled(false);
        assert_eq!(m.banner, None);
        assert_eq!(m.status, UpdateStatus::Idle);

        m.set_enabled(true);
        m.check(true, Err(ReleaseFeedError::Offline));
        assert_eq!(m.status, UpdateStatus::Failed(UpdateErrorKind::Offline));
        m.set_enabled(false);
        assert_eq!(m.status, UpdateStatus::Idle);

        // UpToDate and Checking are kept; enabling changes nothing.
        let up = UpdateStatus::UpToDate { checked_at: 3 };
        let change = UpdatePolicy::enabled_changed(false, &up);
        assert_eq!(change.status, up);
        assert!(change.clear_banner);
        let on = UpdatePolicy::enabled_changed(true, &UpdateStatus::Available(release("1.0.0")));
        assert!(!on.clear_banner);
        assert_eq!(on.status, UpdateStatus::Available(release("1.0.0")));
    }

    #[test]
    fn unparsable_running_version_skips_scheduled_and_reports_on_manual() {
        let mut m = Model::new("0.7.0");
        m.current = None;
        m.check(false, Ok(release("0.7.1")));
        assert_eq!(m.status, UpdateStatus::Idle);
        assert_eq!(m.last_run, None);
        m.check(true, Ok(release("0.7.1")));
        assert_eq!(
            m.status,
            UpdateStatus::Failed(UpdateErrorKind::UnknownVersion)
        );
        assert_eq!(m.banner, None);
    }

    #[test]
    fn schedule_fires_at_10s_then_every_24h() {
        assert_eq!(UpdatePolicy::next_delay(0), Duration::from_secs(10));
        assert_eq!(UpdatePolicy::next_delay(1), Duration::from_secs(86_400));
        assert_eq!(UpdatePolicy::next_delay(2), Duration::from_secs(86_400));
        assert_eq!(INITIAL_DELAY, Duration::from_secs(10));
        assert_eq!(INTERVAL, Duration::from_secs(86_400));
    }

    #[test]
    fn no_retry_before_next_interval_after_failure() {
        // The delay is a function of the tick count only, so a failed tick cannot shorten it.
        for ticks in 1..50 {
            assert!(UpdatePolicy::next_delay(ticks) >= INTERVAL);
        }
    }
}
