//! The decisions of the app's lifecycle, as plain functions over `AppCore` and small seams:
//! platform events into auto-lock, the quit flow, the external-change watcher, the launch-at-login
//! reconcile, the settings-effect runner and the tray refresh.
//!
//! The Tauri-runtime glue in `shell_runtime.rs` only calls these. Nothing here listens to a
//! webview event: every trigger is a Rust call (a session observer, a platform sink, a menu or
//! window callback).

use crate::events::{SaveStateChanged, SettingsChanged};
use crate::shell::{
    reconcile_login_item, second_instance_action, show_main, LoginItemOutcome, SecondInstance,
    WindowOps,
};
use crate::state::EffectRunner;
use crate::tray::{tray_menu_model, RecentEntry, TrayMenuModel, MAX_RECENTS};
use mypass_platform::{PlatformResult, Subscription};
use mypass_session::scheduler::Scheduler;
use mypass_session::session::SessionError;
use mypass_session::settings::{Settings, SettingsPatch, Theme};
use mypass_session::{AppCore, LockReason, Platform, SaveState, SaveStatus, SessionState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::shell::Effect;

// ---------------------------------------------------------------------------------------------
// Locking from a callback

/// Hands a lock to the scheduler instead of locking inline: `lock` waits for an in-flight save,
/// and a hotkey or menu callback runs on the main thread, which must never wait on a disk write.
/// A lock of an already locked session is a no-op.
pub fn lock_via_scheduler(core: &AppCore, reason: LockReason) {
    let session = core.session.clone();
    core.scheduler()
        .schedule(Duration::ZERO, Box::new(move || session.lock(reason)));
}

// ---------------------------------------------------------------------------------------------
// Second instance and platform events

/// The single-instance callback: a second launch shows and focuses the main window and leaves a
/// locked session locked. The `.kdbx` path in `argv` is for stage-5 file association; ignored.
pub fn handle_second_instance(windows: &dyn WindowOps, argv: &[String]) {
    let SecondInstance::ShowMainWindow { .. } = second_instance_action(argv);
    // A window that cannot be shown is not an error of the second launch.
    let _ = show_main(windows);
}

/// Feeds the platform's sleep and screen-lock events to auto-lock. The sink only hands each event
/// to the scheduler (`AutoLock::event_sink`), because a sink must never block. Keep the returned
/// subscription alive: dropping it unsubscribes.
pub fn attach_platform_events(core: &AppCore, platform: &Platform) -> PlatformResult<Subscription> {
    core.autolock.subscribe(platform.events.as_ref())
}

// ---------------------------------------------------------------------------------------------
// Quit

/// How long the final flush may take before the user is asked what to do.
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// What the quit flow decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOutcome {
    /// Let the process exit (the quit gate stays set).
    Exit,
    /// Keep running (the session is open again).
    Stay,
}

/// The three session steps of the quit flow, a seam so a test can record their order.
pub trait QuitSteps {
    /// Closes every opening and mutating call.
    fn begin_quit(&self);
    /// The final save; `Ok` only when everything is on disk.
    fn flush(&self, timeout: Duration) -> Result<(), SessionError>;
    /// Re-opens the session after a cancelled quit.
    fn cancel_quit(&self);
}

impl QuitSteps for AppCore {
    fn begin_quit(&self) {
        AppCore::begin_quit(self);
    }
    fn flush(&self, timeout: Duration) -> Result<(), SessionError> {
        self.session.flush_blocking(timeout)
    }
    fn cancel_quit(&self) {
        AppCore::cancel_quit(self);
    }
}

/// The quit flow. `begin_quit` comes FIRST, so no edit can arrive after the decision; then the
/// final flush; then `quit_decision`. A failed flush asks the user through `ask` (`true` means
/// Quit Anyway): Cancel re-opens the session and stays, Quit Anyway exits with the gate still set
/// (the encrypted bytes were kept by `lock`/the session, nothing is silently dropped).
pub fn on_exit_requested(steps: &dyn QuitSteps, ask: &dyn Fn(&str) -> bool) -> ExitOutcome {
    use crate::shell::{quit_decision, QuitDecision};
    steps.begin_quit();
    match quit_decision(steps.flush(FLUSH_TIMEOUT)) {
        QuitDecision::Proceed => ExitOutcome::Exit,
        QuitDecision::PromptUser { message } => {
            if ask(&message) {
                ExitOutcome::Exit
            } else {
                steps.cancel_quit();
                ExitOutcome::Stay
            }
        }
    }
}

/// What an `ExitRequested` run event does. The quit flow blocks (a flush, then a modal dialog),
/// so it never runs on the event-loop thread: the event is always prevented, the flow runs on a
/// worker, and a confirmed exit calls `exit` again, which this gate lets through.
#[derive(Debug, Default)]
pub struct ExitGate {
    running: AtomicBool,
    confirmed: AtomicBool,
}

/// The answer of [`ExitGate::request`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitRequest {
    /// Prevent the exit and run the quit flow on a worker.
    Start,
    /// A quit flow is already running (a second Cmd+Q): prevent the exit, do nothing else.
    AlreadyRunning,
    /// The flow decided to exit: let it through.
    Allow,
}

impl ExitGate {
    pub fn request(&self) -> ExitRequest {
        if self.confirmed.load(Ordering::SeqCst) {
            ExitRequest::Allow
        } else if self.running.swap(true, Ordering::SeqCst) {
            ExitRequest::AlreadyRunning
        } else {
            ExitRequest::Start
        }
    }

    /// Records the outcome of the flow: `Exit` confirms (the next request is allowed), `Stay`
    /// re-opens the gate for a later quit attempt.
    pub fn finish(&self, outcome: ExitOutcome) {
        match outcome {
            ExitOutcome::Exit => self.confirmed.store(true, Ordering::SeqCst),
            ExitOutcome::Stay => self.running.store(false, Ordering::SeqCst),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// External changes

/// How often an unlocked vault is checked for a change on disk (besides on window focus).
pub const EXTERNAL_POLL: Duration = Duration::from_secs(30);

/// Checks the vault file for external changes and reports a failure once per distinct message
/// (a vault that stays unreadable does not raise a toast every 30 seconds).
pub struct ExternalWatcher {
    core: Arc<AppCore>,
    report: Arc<dyn Fn(SaveStatus) + Send + Sync>,
    last_error: Mutex<Option<String>>,
}

impl ExternalWatcher {
    pub fn new(core: Arc<AppCore>, report: Arc<dyn Fn(SaveStatus) + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            core,
            report,
            last_error: Mutex::new(None),
        })
    }

    /// One check; a no-op while nothing is unlocked. May block for an in-flight save and does
    /// file I/O: never call it on the event-loop thread.
    pub fn check(&self) {
        if !self.core.session.is_unlocked() {
            return;
        }
        let mut last = self
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match self.core.session.check_external_change() {
            Ok(_) => *last = None,
            Err(error) => {
                let message = error.to_string();
                if last.as_deref() != Some(message.as_str()) {
                    *last = Some(message.clone());
                    (self.report)(SaveStatus {
                        state: SaveState::Failed,
                        message: Some(message),
                    });
                }
            }
        }
    }
}

/// Runs `watcher.check()` every `every` on the scheduler, for as long as the process lives.
pub fn start_external_poll(
    watcher: Arc<ExternalWatcher>,
    scheduler: Arc<dyn Scheduler>,
    every: Duration,
) {
    let next = scheduler.clone();
    scheduler.schedule(
        every,
        Box::new(move || {
            watcher.check();
            start_external_poll(watcher, next, every);
        }),
    );
}

// ---------------------------------------------------------------------------------------------
// Launch at login

/// Brings the OS login item to `desired` and, when the OS state wins, writes the actual state
/// back through the settings store. Returns the corrected settings when the stored value changed
/// (the caller emits `SettingsChanged`). Never calls the `settings_update` command, so it is safe
/// under the settings gate.
pub fn sync_login_item(core: &AppCore, desired: bool) -> Option<Settings> {
    let should_be = match reconcile_login_item(desired, core.platform().launch_at_login.as_ref()) {
        LoginItemOutcome::InSync | LoginItemOutcome::Applied => return None,
        LoginItemOutcome::NeedsApproval { setting_should_be }
        | LoginItemOutcome::Failed {
            setting_should_be, ..
        }
        | LoginItemOutcome::Unsupported { setting_should_be } => setting_should_be,
    };
    if core.settings.get().launch_at_login == should_be {
        return None;
    }
    core.settings_update(SettingsPatch {
        launch_at_login: Some(should_be),
        ..SettingsPatch::default()
    })
    .ok()
}

// ---------------------------------------------------------------------------------------------
// The tray menu

/// The tray menu for the current state: the language from the settings, recent entries only
/// while unlocked.
pub fn current_tray_model(core: &AppCore, update: Option<&str>) -> TrayMenuModel {
    let state = core.session.status().state;
    let recents: Vec<RecentEntry> = if state == SessionState::Unlocked {
        core.entry_recent(MAX_RECENTS as u32)
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|e| RecentEntry {
                        id: e.id,
                        title: e.title,
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    tray_menu_model(state, &recents, core.settings.get().language, update)
}

/// Rebuilds the tray menu after the session or the vault changed. Requests coalesce: any number
/// arriving before the scheduled build runs cause one build, and the model is built on the
/// scheduler thread, so neither the caller nor the main thread waits for the vault.
pub struct TrayRefresher {
    core: Arc<AppCore>,
    apply: Arc<dyn Fn(TrayMenuModel) + Send + Sync>,
    pending: Arc<AtomicBool>,
}

/// The delay that lets a burst of vault edits collapse into one tray rebuild.
pub const TRAY_REFRESH_DELAY: Duration = Duration::from_millis(250);

impl TrayRefresher {
    pub fn new(core: Arc<AppCore>, apply: Arc<dyn Fn(TrayMenuModel) + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            core,
            apply,
            pending: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Rebuilds the menu after `delay` (one build for any number of requests meanwhile).
    pub fn request(&self, delay: Duration) {
        if self.pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let core = self.core.clone();
        let apply = self.apply.clone();
        let pending = self.pending.clone();
        self.core.scheduler().schedule(
            delay,
            Box::new(move || {
                // Cleared first: a request that arrives while building schedules another build.
                pending.store(false, Ordering::SeqCst);
                apply(current_tray_model(&core, None));
            }),
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Settings effects

/// What the effect runner needs from the runtime. Every method may be called from any thread and
/// must not block on the main thread (the runtime implementation hops to it without waiting).
pub trait ShellTarget: Send + Sync {
    fn set_tray_visible(&self, visible: bool);
    fn rebuild_tray_menu(&self);
    fn register_hotkeys(&self);
    fn apply_theme(&self, theme: Theme);
}

/// The real [`EffectRunner`]: maps each effect onto the core or the runtime target.
///
/// Called off the main thread while `settings_update` holds the settings gate: it never takes the
/// gate and never calls `settings_update` (the login-item write-back goes straight to the store).
pub struct ShellEffects {
    core: Arc<AppCore>,
    target: Arc<dyn ShellTarget>,
}

impl ShellEffects {
    pub fn new(core: Arc<AppCore>, target: Arc<dyn ShellTarget>) -> Self {
        Self { core, target }
    }
}

impl EffectRunner for ShellEffects {
    fn run(&self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                // The write-back (a refused or unsupported login item) reaches the webview
                // through the `SettingsChanged` the update emits after the effects ran.
                Effect::SetAutostart(desired) => {
                    sync_login_item(&self.core, desired);
                }
                Effect::SetTrayVisible(visible) => self.target.set_tray_visible(visible),
                Effect::RegisterHotkeys => self.target.register_hotkeys(),
                Effect::ApplyTheme(theme) => self.target.apply_theme(theme),
                // The language lives in the settings, which the tray model reads: the
                // `RebuildTrayMenu` that always follows is what makes it visible.
                Effect::ApplyLanguage(_) => {}
                Effect::RebuildTrayMenu => self.target.rebuild_tray_menu(),
                Effect::RearmAutoLock => self.core.autolock.settings_changed(),
            }
        }
    }
}

/// Emits the settings a startup reconcile corrected.
pub fn emit_settings<R: tauri::Runtime>(app: &tauri::AppHandle<R>, settings: Settings) {
    use tauri_specta::Event;
    // A window that is gone is not an error.
    let _ = SettingsChanged(settings).emit(app);
}

/// Emits a save-state toast for a failed external check.
pub fn emit_save_state<R: tauri::Runtime>(app: &tauri::AppHandle<R>, status: SaveStatus) {
    use tauri_specta::Event;
    let _ = SaveStateChanged(status).emit(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{settings_effects, FakeWindowOps};
    use crate::testing::{open_world, pw, world};
    use crate::tray::TrayItem;
    use mypass_platform::{LoginItemStatus, PlatformError, SystemEvent};
    use mypass_session::settings::Language;
    use std::cell::RefCell;
    use std::sync::atomic::AtomicUsize;

    // ---- platform events ----

    #[test]
    fn platform_event_wiring_locks_on_screen_lock() {
        let w = open_world();
        let _subscription = attach_platform_events(&w.core, w.core.platform()).unwrap();
        assert_eq!(w.fake.events.subscriber_count(), 1);
        w.fake.events.emit(SystemEvent::ScreenLocked);
        // The sink only scheduled the lock: it never blocks and never locks inline.
        assert!(w.core.session.is_unlocked());
        w.run_scheduled();
        let status = w.core.session.status();
        assert_eq!(status.state, SessionState::Locked);
        assert_eq!(status.lock_reason, Some(LockReason::ScreenLock));
    }

    #[test]
    fn dropping_the_subscription_stops_the_wiring() {
        let w = open_world();
        let subscription = attach_platform_events(&w.core, w.core.platform()).unwrap();
        drop(subscription);
        w.fake.events.emit(SystemEvent::ScreenLocked);
        w.run_scheduled();
        assert!(w.core.session.is_unlocked());
    }

    #[test]
    fn second_instance_shows_then_focuses_main_and_leaves_the_lock_alone() {
        let w = open_world();
        w.core.vault_lock().unwrap();
        let windows = FakeWindowOps::default();
        handle_second_instance(&windows, &["mypass".into(), "/x/Vault.KDBX".into()]);
        let calls: Vec<_> = windows.calls().into_iter().map(|c| (c.0, c.1)).collect();
        assert_eq!(
            calls,
            [
                ("show".to_string(), "main".to_string()),
                ("focus".to_string(), "main".to_string())
            ]
        );
        assert_eq!(w.core.session.status().state, SessionState::Locked);
    }

    #[test]
    fn lock_via_scheduler_defers_and_ignores_a_locked_session() {
        let w = world();
        // No vault: nothing to lock, nothing happens.
        lock_via_scheduler(&w.core, LockReason::Hotkey);
        w.run_scheduled();
        assert_eq!(w.core.session.status().state, SessionState::NoVault);
    }

    // ---- exit ----

    #[derive(Default)]
    struct Recorder {
        log: RefCell<Vec<&'static str>>,
        flush_fails: bool,
    }

    impl QuitSteps for Recorder {
        fn begin_quit(&self) {
            self.log.borrow_mut().push("begin_quit");
        }
        fn flush(&self, timeout: Duration) -> Result<(), SessionError> {
            assert_eq!(timeout, Duration::from_secs(5));
            self.log.borrow_mut().push("flush");
            if self.flush_fails {
                Err(SessionError::Io("disk full".into()))
            } else {
                Ok(())
            }
        }
        fn cancel_quit(&self) {
            self.log.borrow_mut().push("cancel_quit");
        }
    }

    #[test]
    fn exit_requested_begins_quit_before_flush() {
        let steps = Recorder::default();
        let outcome = on_exit_requested(&steps, &|_| panic!("a clean flush must not ask"));
        assert_eq!(outcome, ExitOutcome::Exit);
        assert_eq!(*steps.log.borrow(), ["begin_quit", "flush"]);

        let failing = Recorder {
            flush_fails: true,
            ..Recorder::default()
        };
        let asked = RefCell::new(Vec::new());
        let outcome = on_exit_requested(&failing, &|m| {
            asked.borrow_mut().push(m.to_owned());
            false
        });
        assert_eq!(outcome, ExitOutcome::Stay);
        // The decision comes after the flush, and a cancel re-opens the session last.
        assert_eq!(
            *failing.log.borrow(),
            ["begin_quit", "flush", "cancel_quit"]
        );
        assert_eq!(asked.borrow().len(), 1);
        assert!(asked.borrow()[0].contains("disk full"));
    }

    /// A core whose vault has an unsaved edit and whose writes fail.
    fn dirty_world_with_failing_writes() -> crate::testing::World {
        use mypass_kdbx::test_util::FailingFileIo;
        use mypass_kdbx::PlainFileIo;
        use std::io::ErrorKind;
        let w = open_world();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        w.core.session.set_file_io(io.clone());
        io.fail_write_atomic(ErrorKind::PermissionDenied);
        w.login("Unsaved");
        w
    }

    #[test]
    fn exit_with_failed_flush_prompts_and_cancel_reopens_session() {
        let w = dirty_world_with_failing_writes();
        let asked = AtomicUsize::new(0);
        let outcome = on_exit_requested(&w.core, &|_| {
            asked.fetch_add(1, Ordering::SeqCst);
            false
        });
        assert_eq!(outcome, ExitOutcome::Stay);
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert!(!w.core.is_quitting());
        // The session accepts edits again.
        w.login("After cancel");
    }

    #[test]
    fn exit_quit_anyway_exits_with_gate_still_set() {
        let w = dirty_world_with_failing_writes();
        let outcome = on_exit_requested(&w.core, &|_| true);
        assert_eq!(outcome, ExitOutcome::Exit);
        assert!(w.core.is_quitting());
        // The gate refuses further edits.
        let mut model = mypass_session::editing::new_model(
            mypass_session::dto::EntryTypeDto::Login,
            uuid::Uuid::parse_str(&w.root()).unwrap(),
        );
        model.title = "late".into();
        assert_eq!(
            w.core.entry_save(model).unwrap_err().code,
            mypass_session::ErrorCode::Quitting
        );
    }

    #[test]
    fn exit_with_clean_flush_exits_without_prompt() {
        let w = open_world();
        w.login("Saved by the final flush");
        let outcome = on_exit_requested(&w.core, &|_| panic!("clean flush must not ask"));
        assert_eq!(outcome, ExitOutcome::Exit);
        assert!(w.core.is_quitting());
    }

    #[test]
    fn exit_gate_runs_one_flow_and_lets_a_confirmed_exit_through() {
        let gate = ExitGate::default();
        assert_eq!(gate.request(), ExitRequest::Start);
        assert_eq!(gate.request(), ExitRequest::AlreadyRunning);
        gate.finish(ExitOutcome::Stay);
        assert_eq!(gate.request(), ExitRequest::Start, "re-opened after Cancel");
        gate.finish(ExitOutcome::Exit);
        assert_eq!(gate.request(), ExitRequest::Allow);
        assert_eq!(gate.request(), ExitRequest::Allow);
    }

    // ---- external changes ----

    fn recording_watcher(
        w: &crate::testing::World,
    ) -> (Arc<ExternalWatcher>, Arc<Mutex<Vec<SaveStatus>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let watcher = ExternalWatcher::new(
            Arc::new(w.core.clone()),
            Arc::new(move |s| sink.lock().unwrap().push(s)),
        );
        (watcher, seen)
    }

    #[test]
    fn external_check_is_a_no_op_while_locked_and_quiet_when_nothing_changed() {
        let w = open_world();
        let (watcher, seen) = recording_watcher(&w);
        watcher.check();
        w.core.vault_lock().unwrap();
        watcher.check();
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn an_unreadable_external_change_is_reported_once_as_a_failed_save_state() {
        let w = open_world();
        let (watcher, seen) = recording_watcher(&w);
        std::fs::write(&w.path, b"this is no vault").unwrap();
        watcher.check();
        watcher.check();
        let reports = seen.lock().unwrap().clone();
        assert_eq!(reports.len(), 1, "the same failure is not repeated");
        assert_eq!(reports[0].state, SaveState::Failed);
        assert!(reports[0].message.is_some());
    }

    #[test]
    fn the_poll_runs_every_thirty_seconds_and_re_arms_itself() {
        let w = open_world();
        let (watcher, seen) = recording_watcher(&w);
        start_external_poll(watcher, w.core.scheduler(), EXTERNAL_POLL);
        std::fs::write(&w.path, b"this is no vault").unwrap();
        w.clock.advance(Duration::from_secs(29));
        assert!(seen.lock().unwrap().is_empty());
        w.clock.advance(Duration::from_secs(1));
        assert_eq!(seen.lock().unwrap().len(), 1);
        // Re-armed: a different failure 30 s later is reported again.
        std::fs::write(&w.path, b"").unwrap();
        w.clock.advance(Duration::from_secs(30));
        assert!(seen.lock().unwrap().len() >= 2);
    }

    // ---- launch at login ----

    #[test]
    fn startup_reconcile_writes_the_refused_login_item_back_off() {
        let w = world();
        let fake = w.fake.launch_at_login.clone();
        w.core
            .settings_update(SettingsPatch {
                launch_at_login: Some(true),
                ..SettingsPatch::default()
            })
            .unwrap();
        fake.fail_next(PlatformError::Failed("denied".into()));
        let corrected = sync_login_item(&w.core, true).unwrap();
        assert!(!corrected.launch_at_login);
        assert!(!w.core.settings.get().launch_at_login);
    }

    #[test]
    fn startup_reconcile_leaves_matching_state_alone() {
        let w = world();
        w.fake.launch_at_login.set_status(LoginItemStatus::Disabled);
        assert_eq!(sync_login_item(&w.core, false), None);
        assert_eq!(w.fake.launch_at_login.set_calls(), 0);
        // Desired on and applied: the setting stays on.
        w.core
            .settings_update(SettingsPatch {
                launch_at_login: Some(true),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(sync_login_item(&w.core, true), None);
        assert!(w.core.settings.get().launch_at_login);
    }

    // ---- tray refresh and effects ----

    #[test]
    fn tray_model_follows_the_session_state() {
        let w = open_world();
        w.login("One");
        let unlocked = current_tray_model(&w.core, None);
        assert!(unlocked.items.contains(&TrayItem::Lock));
        assert!(matches!(
            unlocked.items.first(),
            Some(TrayItem::Recent { .. })
        ));
        w.core.vault_lock().unwrap();
        let locked = current_tray_model(&w.core, None);
        assert!(locked.items.contains(&TrayItem::Unlock));
        assert!(!locked
            .items
            .iter()
            .any(|i| matches!(i, TrayItem::Recent { .. })));
        w.core.vault_unlock(pw("pw")).unwrap();
        assert!(current_tray_model(&w.core, Some("2.0"))
            .items
            .contains(&TrayItem::UpdateAvailable("2.0".into())));
    }

    #[test]
    fn tray_refresh_coalesces_a_burst_into_one_build() {
        let w = open_world();
        let built = Arc::new(Mutex::new(Vec::new()));
        let sink = built.clone();
        let refresher = TrayRefresher::new(
            Arc::new(w.core.clone()),
            Arc::new(move |m| sink.lock().unwrap().push(m)),
        );
        for _ in 0..5 {
            refresher.request(TRAY_REFRESH_DELAY);
        }
        w.clock.advance(Duration::from_millis(249));
        assert!(built.lock().unwrap().is_empty());
        w.clock.advance(Duration::from_millis(1));
        assert_eq!(built.lock().unwrap().len(), 1);
        refresher.request(TRAY_REFRESH_DELAY);
        w.clock.advance(TRAY_REFRESH_DELAY);
        assert_eq!(built.lock().unwrap().len(), 2);
    }

    #[derive(Default)]
    struct RecordingTarget {
        calls: Mutex<Vec<String>>,
        refresher: Mutex<Option<Arc<TrayRefresher>>>,
    }

    impl ShellTarget for RecordingTarget {
        fn set_tray_visible(&self, visible: bool) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("tray_visible {visible}"));
        }
        fn rebuild_tray_menu(&self) {
            self.calls.lock().unwrap().push("rebuild_tray_menu".into());
            if let Some(refresher) = self.refresher.lock().unwrap().as_ref() {
                refresher.request(Duration::ZERO);
            }
        }
        fn register_hotkeys(&self) {
            self.calls.lock().unwrap().push("register_hotkeys".into());
        }
        fn apply_theme(&self, theme: Theme) {
            self.calls.lock().unwrap().push(format!("theme {theme:?}"));
        }
    }

    #[test]
    fn language_change_rebuilds_tray_menu() {
        let w = open_world();
        let built = Arc::new(Mutex::new(Vec::<TrayMenuModel>::new()));
        let sink = built.clone();
        let refresher = TrayRefresher::new(
            Arc::new(w.core.clone()),
            Arc::new(move |m| sink.lock().unwrap().push(m)),
        );
        let target = Arc::new(RecordingTarget::default());
        *target.refresher.lock().unwrap() = Some(refresher);
        let effects = ShellEffects::new(Arc::new(w.core.clone()), target.clone());

        // The update the command would make: settings first, then the effects of the diff.
        let old = w.core.settings_get().unwrap();
        let new = w
            .core
            .settings_update(SettingsPatch {
                language: Some(Language::Uk),
                ..SettingsPatch::default()
            })
            .unwrap();
        effects.run(settings_effects(&old, &new));
        w.run_scheduled();

        assert_eq!(*target.calls.lock().unwrap(), ["rebuild_tray_menu"]);
        let models = built.lock().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].language, Language::Uk);
        let uk_lock = crate::i18n::lookup(crate::i18n::Language::Uk, "Lock").unwrap();
        assert!(models[0].labels().iter().any(|l| l == uk_lock));
    }

    #[test]
    fn effects_reach_the_matching_target_calls() {
        let w = open_world();
        let target = Arc::new(RecordingTarget::default());
        let effects = ShellEffects::new(Arc::new(w.core.clone()), target.clone());
        effects.run(vec![
            Effect::ApplyTheme(Theme::Dark),
            Effect::SetTrayVisible(false),
            Effect::RegisterHotkeys,
            Effect::ApplyLanguage(Language::Ru),
            Effect::RearmAutoLock,
        ]);
        assert_eq!(
            *target.calls.lock().unwrap(),
            ["theme Dark", "tray_visible false", "register_hotkeys"]
        );
    }

    #[test]
    fn the_autostart_effect_writes_back_an_unsupported_state_without_the_settings_gate() {
        let w = open_world();
        // The facade stores `true`, the OS refuses: the effect runner corrects the store.
        w.core
            .settings_update(SettingsPatch {
                launch_at_login: Some(true),
                ..SettingsPatch::default()
            })
            .unwrap();
        w.fake
            .launch_at_login
            .fail_next(PlatformError::Unsupported("no login item"));
        let effects = ShellEffects::new(
            Arc::new(w.core.clone()),
            Arc::new(RecordingTarget::default()),
        );
        effects.run(vec![Effect::SetAutostart(true)]);
        assert!(!w.core.settings.get().launch_at_login);
    }
}
