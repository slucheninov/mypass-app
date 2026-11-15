//! Stateful update checking. Timers only dispatch work; the feed never blocks the session scheduler.
use crate::scheduler::{lock, Scheduler, TaskHandle};
use crate::settings::SettingsStore;
use mypass_update::UpdateStatus as PolicyStatus;
use mypass_update::{
    BannerChange, BeginDecision, FinishInput, ReleaseFeed, ReleaseInfo, SemanticVersion,
    UpdatePolicy, UpdateStateStore,
};
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UpdateFailure {
    Offline,
    RateLimited,
    BadStatus,
    Unexpected,
    UnknownVersion,
}
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate { checked_at: f64 },
    Available { version: String, url: String },
    Failed { kind: UpdateFailure },
}
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRelease {
    pub version: String,
    pub url: String,
}
/// Status and banner are separate: a failed manual check must preserve an existing banner.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub status: UpdateState,
    pub banner: Option<UpdateRelease>,
}
pub type UpdateObserver = Arc<dyn Fn(UpdateStatus) + Send + Sync>;
struct State {
    status: PolicyStatus,
    banner: Option<ReleaseInfo>,
    flight: Option<bool>,
    generation: u64,
    timer: Option<TaskHandle>,
    scheduling: bool,
    shutdown: bool,
    observers: Vec<UpdateObserver>,
}
struct Inner {
    scheduler: Arc<dyn Scheduler>,
    settings: SettingsStore,
    feed: Arc<dyn ReleaseFeed>,
    current: Option<SemanticVersion>,
    store: UpdateStateStore,
    state: Mutex<State>,
}
#[derive(Clone)]
pub struct UpdateService(Arc<Inner>);
impl UpdateService {
    pub fn new(
        scheduler: Arc<dyn Scheduler>,
        settings: SettingsStore,
        feed: Arc<dyn ReleaseFeed>,
        version: &str,
        config_dir: &Path,
    ) -> Self {
        Self(Arc::new(Inner {
            scheduler,
            settings,
            feed,
            current: SemanticVersion::parse(version).ok(),
            store: UpdateStateStore::new(config_dir),
            state: Mutex::new(State {
                status: PolicyStatus::Idle,
                banner: None,
                flight: None,
                generation: 0,
                timer: None,
                scheduling: false,
                shutdown: false,
                observers: Vec::new(),
            }),
        }))
    }
    pub fn status(&self) -> UpdateStatus {
        snapshot(&lock(&self.0.state))
    }
    pub fn add_observer(&self, observer: UpdateObserver) {
        lock(&self.0.state).observers.push(observer);
    }
    fn emit(&self) {
        let (status, observers) = {
            let s = lock(&self.0.state);
            (snapshot(&s), s.observers.clone())
        };
        for observer in observers {
            observer(status.clone());
        }
    }
    /// Blocking; callers use a worker. A second manual caller upgrades the running attempt.
    pub fn check(&self, manual: bool) -> UpdateStatus {
        self.check_generation(manual, None)
    }
    fn check_generation(&self, manual: bool, generation: Option<u64>) -> UpdateStatus {
        let previous = {
            let mut s = lock(&self.0.state);
            if s.shutdown || generation.is_some_and(|g| !s.scheduling || s.generation != g) {
                return snapshot(&s);
            }
            match UpdatePolicy::begin(
                manual,
                self.0.settings.get().update_check_enabled,
                self.0.current,
                s.flight,
            ) {
                BeginDecision::Skip => return snapshot(&s),
                BeginDecision::Fail(k) => {
                    s.status = PolicyStatus::Failed(k);
                    drop(s);
                    self.emit();
                    return self.status();
                }
                BeginDecision::AlreadyRunning { upgrade_to_manual } => {
                    if upgrade_to_manual {
                        s.flight = Some(true);
                    }
                    return snapshot(&s);
                }
                BeginDecision::Run => {
                    s.flight = Some(manual);
                    std::mem::replace(&mut s.status, PolicyStatus::Checking)
                }
            }
        };
        self.emit();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.feed.latest()))
                .unwrap_or(Err(mypass_update::ReleaseFeedError::Offline));
        let now = self
            .0
            .scheduler
            .wall_now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64;
        {
            let mut s = lock(&self.0.state);
            if !s.shutdown {
                if let Some(current) = self.0.current {
                    let state = self.0.store.load();
                    let out = UpdatePolicy::finish(FinishInput {
                        result: &result,
                        current,
                        previous: &previous,
                        manual: s.flight.unwrap_or(manual),
                        dismissed_version: state.dismissed_version.as_deref(),
                        now,
                    });
                    s.status = out.status;
                    match out.banner {
                        BannerChange::Show(r) => s.banner = Some(r),
                        BannerChange::Clear => s.banner = None,
                        BannerChange::Keep => {}
                    }
                    if !self.0.settings.get().update_check_enabled && s.flight == Some(false) {
                        s.banner = None;
                        s.status = PolicyStatus::Idle;
                    }
                }
                let _ = self.0.store.record_run(now);
            }
            s.flight = None;
        }
        self.emit();
        self.status()
    }
    pub fn dismiss(&self) -> Result<(), crate::ApiError> {
        let mut s = lock(&self.0.state);
        if let Some(version) = UpdatePolicy::dismissal(s.banner.as_ref()) {
            self.0.store.set_dismissed_version(&version).map_err(|_| {
                crate::ApiError::new(crate::ErrorCode::Io, "could not save update dismissal")
            })?;
        }
        s.banner = None;
        drop(s);
        self.emit();
        Ok(())
    }
    pub fn enabled_changed(&self) {
        let mut s = lock(&self.0.state);
        let change =
            UpdatePolicy::enabled_changed(self.0.settings.get().update_check_enabled, &s.status);
        s.status = change.status;
        if change.clear_banner {
            s.banner = None;
        }
        drop(s);
        self.emit();
    }
    /// Revalidate even an injected feed; no URL ever comes from the webview.
    pub fn release_url(&self) -> Option<String> {
        let s = lock(&self.0.state);
        let r = s.banner.as_ref().or(match &s.status {
            PolicyStatus::Available(r) => Some(r),
            _ => None,
        })?;
        mypass_update::validate_release_url(&r.url)
    }
    pub fn start_scheduling(&self) {
        let mut s = lock(&self.0.state);
        if s.scheduling || s.shutdown {
            return;
        }
        s.scheduling = true;
        s.generation += 1;
        self.arm(&mut s, mypass_update::INITIAL_DELAY);
    }
    fn arm(&self, s: &mut State, delay: Duration) {
        let weak = Arc::downgrade(&self.0);
        let generation = s.generation;
        s.timer = Some(self.0.scheduler.schedule(
            delay,
            Box::new(move || {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let svc = Self(inner);
                let mut s = lock(&svc.0.state);
                if s.shutdown || !s.scheduling || s.generation != generation {
                    return;
                }
                // At most one feed invocation; a slow feed cannot delay auto-lock.
                let run = svc.clone();
                let _ = std::thread::Builder::new()
                    .name("mypass-update".into())
                    .spawn(move || {
                        run.check_generation(false, Some(generation));
                    });
                svc.arm(&mut s, mypass_update::INTERVAL);
            }),
        ));
    }
    pub fn stop_scheduling(&self) {
        let mut s = lock(&self.0.state);
        s.scheduling = false;
        s.generation += 1;
        if let Some(timer) = s.timer.take() {
            timer.cancel();
        }
    }
    /// Withdraw queued and in-flight results without waiting on network I/O.
    pub fn shutdown(&self) {
        let mut s = lock(&self.0.state);
        s.shutdown = true;
        s.scheduling = false;
        s.generation += 1;
        if let Some(timer) = s.timer.take() {
            timer.cancel();
        }
    }
}
fn snapshot(s: &State) -> UpdateStatus {
    use mypass_update::UpdateErrorKind as K;
    let status = match &s.status {
        PolicyStatus::Idle => UpdateState::Idle,
        PolicyStatus::Checking => UpdateState::Checking,
        PolicyStatus::UpToDate { checked_at } => UpdateState::UpToDate {
            checked_at: *checked_at as f64 * 1000.0,
        },
        PolicyStatus::Available(r) => UpdateState::Available {
            version: r.version.to_string(),
            url: r.url.clone(),
        },
        PolicyStatus::Failed(k) => UpdateState::Failed {
            kind: match k {
                K::Offline => UpdateFailure::Offline,
                K::RateLimited => UpdateFailure::RateLimited,
                K::BadStatus => UpdateFailure::BadStatus,
                K::Unexpected => UpdateFailure::Unexpected,
                K::UnknownVersion => UpdateFailure::UnknownVersion,
            },
        },
    };
    UpdateStatus {
        status,
        banner: s.banner.as_ref().map(|r| UpdateRelease {
            version: r.version.to_string(),
            url: r.url.clone(),
        }),
    }
}
#[cfg(test)]
mod tests;
