//! The Tauri-runtime glue of the shell: the tray icon and its panel, the effect target, window
//! events and run events. Every decision it takes comes from a plain function (`shell`, `tray`,
//! `hotkeys`, `lifecycle`); what is left here needs a running app and is covered by the manual
//! checklist.
//!
//! Threading rules: the main thread never waits. Work that must run on it (tray, menu, theme,
//! global shortcuts) is posted with `run_on_main_thread`, which returns at once; the model of a
//! menu is built on the calling thread first. The quit flow runs on a worker thread because it
//! flushes to disk and shows a blocking dialog.

use crate::commands::app::{app_open_settings, open_entry, MAIN_WINDOW};
use crate::events::ShortcutConflictEvent;
use crate::hotkeys::{self, ConflictGate};
use crate::i18n;
use crate::lifecycle::{
    attach_platform_events, current_tray_model, emit_save_state, emit_settings, lock_via_scheduler,
    on_exit_requested, start_external_poll, sync_login_item, ExitGate, ExitOutcome, ExitRequest,
    ExternalWatcher, QuitPort, ShellTarget, TrayRefresher, EXTERNAL_POLL,
};
use crate::shell::{close_action, current_os, panel_position, show_main, CloseAction, PanelRect};
use crate::state::{AppState, ShellState};
use crate::tray::{
    action_for_id, separator_before, TrayAction, TrayMenuModel, PANEL_WINDOW, TRAY_ID,
};
use mypass_platform::Subscription;
use mypass_session::settings::{Language, Settings, Theme};
use mypass_session::{AppCore, LockReason};
use std::sync::{Arc, Mutex, PoisonError};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, RunEvent, Runtime, Window, WindowEvent, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_specta::Event;

/// Keeps what must live as long as the app: the platform event subscription.
#[derive(Default)]
pub struct KeepAlive(Mutex<Vec<Subscription>>);

impl KeepAlive {
    fn push(&self, subscription: Subscription) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(subscription);
    }
}

// ---------------------------------------------------------------------------------------------
// Tray

fn build_menu(app: &AppHandle, model: &TrayMenuModel) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let mut previous = None;
    for item in &model.items {
        if previous.is_some_and(|prev| separator_before(prev, item)) {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        let row = MenuItem::with_id(
            app,
            item.id(),
            item.label(model.language),
            true,
            None::<&str>,
        )?;
        menu.append(&row)?;
        previous = Some(item);
    }
    Ok(menu)
}

/// Creates the tray icon (a template image on macOS) with `model`'s menu. Main thread only.
fn create_tray(app: &AppHandle, model: &TrayMenuModel) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return apply_menu(app, model);
    }
    let menu = build_menu(app, model)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .icon_as_template(true)
        .on_menu_event(|app, event| {
            if let Some(action) = action_for_id(event.id().as_ref()) {
                run_tray_action(app, action);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_panel(tray.app_handle(), rect);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Replaces the menu of the tray, if there is one. Main thread only.
fn apply_menu(app: &AppHandle, model: &TrayMenuModel) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(app, model)?))?;
    }
    Ok(())
}

/// Removes the tray icon and the panel it opened. Main thread only.
fn remove_tray(app: &AppHandle) {
    let _ = app.remove_tray_by_id(TRAY_ID);
    if let Some(panel) = app.get_webview_window(PANEL_WINDOW) {
        let _ = panel.hide();
    }
}

fn tray_visible(app: &AppHandle) -> bool {
    app.tray_by_id(TRAY_ID).is_some()
}

fn run_tray_action(app: &AppHandle, action: TrayAction) {
    let (Some(core), Some(shell)) = (app.try_state::<AppState>(), app.try_state::<ShellState>())
    else {
        return;
    };
    match action {
        TrayAction::ShowMain => {
            let _ = show_main(shell.windows.as_ref());
        }
        TrayAction::OpenEntry(id) => {
            if let Ok(Some(event)) = open_entry(&core.0, shell.windows.as_ref(), &id, false) {
                let _ = event.emit(app);
            }
        }
        // A menu callback runs on the main thread: the lock goes through the scheduler.
        TrayAction::Lock => lock_via_scheduler(&core.0, LockReason::Manual),
        // Creating a window on the main thread from an event callback can deadlock on Windows.
        TrayAction::OpenSettings => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = app_open_settings(app).await;
            });
        }
        TrayAction::Quit => app.exit(0),
    }
}

/// Left click on the icon: toggle the panel, placed under the icon inside the monitor.
fn toggle_panel(app: &AppHandle, tray: tauri::Rect) {
    let Some(panel) = app.get_webview_window(PANEL_WINDOW) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }
    let scale = panel.scale_factor().unwrap_or(1.0);
    let at = tray.position.to_physical::<f64>(scale);
    let size = tray.size.to_physical::<f64>(scale);
    let monitor = app
        .monitor_from_point(at.x, at.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    if let (Some(monitor), Ok(panel_size)) = (monitor, panel.outer_size()) {
        let (x, y) = panel_position(
            PanelRect {
                x: at.x,
                y: at.y,
                w: size.width,
                h: size.height,
            },
            PanelRect {
                x: f64::from(monitor.position().x),
                y: f64::from(monitor.position().y),
                w: f64::from(monitor.size().width),
                h: f64::from(monitor.size().height),
            },
            f64::from(panel_size.width),
            f64::from(panel_size.height),
        );
        let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }
    let _ = panel.show();
    let _ = panel.set_focus();
}

/// Hides the tray panel (blur, lock).
pub fn hide_panel(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL_WINDOW) {
        let _ = panel.hide();
    }
}

// ---------------------------------------------------------------------------------------------
// The effect target

/// [`ShellTarget`] over the real app. Each method returns at once: the work is posted to the
/// main thread, never awaited.
pub struct TauriTarget {
    app: AppHandle,
    core: Arc<AppCore>,
    refresher: Arc<TrayRefresher>,
    conflicts: Arc<ConflictGate>,
}

impl TauriTarget {
    pub fn new(
        app: AppHandle,
        core: Arc<AppCore>,
        refresher: Arc<TrayRefresher>,
        conflicts: Arc<ConflictGate>,
    ) -> Self {
        Self {
            app,
            core,
            refresher,
            conflicts,
        }
    }

    fn on_main(&self, job: impl FnOnce(&AppHandle) + Send + 'static) {
        let app = self.app.clone();
        // The only failure is an app that is already shutting down.
        let _ = self.app.run_on_main_thread(move || job(&app));
    }
}

impl ShellTarget for TauriTarget {
    fn set_tray_visible(&self, visible: bool) {
        let model = current_tray_model(&self.core, None);
        self.on_main(move |app| {
            if visible {
                let _ = create_tray(app, &model);
            } else {
                remove_tray(app);
            }
        });
    }

    fn rebuild_tray_menu(&self) {
        self.refresher.request(std::time::Duration::ZERO);
    }

    fn register_hotkeys(&self) {
        let settings = self.core.settings.get();
        let gate = self.conflicts.clone();
        self.on_main(move |app| {
            emit_conflicts(app, gate.submit(hotkeys::register_all(app, &settings)));
        });
    }

    fn apply_theme(&self, theme: Theme) {
        self.on_main(move |app| app.set_theme(native_theme(theme)));
    }
}

fn native_theme(theme: Theme) -> Option<tauri::Theme> {
    match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    }
}

fn emit_conflicts(app: &AppHandle, conflicts: Vec<ShortcutConflictEvent>) {
    for conflict in conflicts {
        let _ = conflict.emit(app);
    }
}

/// The tray refresher of the app: the model is built on the scheduler thread, the menu is swapped
/// on the main thread.
pub fn tray_refresher(app: &AppHandle, core: Arc<AppCore>) -> Arc<TrayRefresher> {
    let app = app.clone();
    TrayRefresher::new(
        core,
        Arc::new(move |model: TrayMenuModel| {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                let _ = apply_menu(&handle, &model);
            });
        }),
    )
}

// ---------------------------------------------------------------------------------------------
// Startup

/// Wires everything that needs the running app, once the state is managed: launch-at-login
/// reconcile, platform events, the external-change watcher, theme, tray, hotkeys and the first
/// window. A failure of an optional piece (no platform events on this OS) never stops the app.
pub fn start(app: &tauri::App) {
    let handle = app.handle().clone();
    let core = app.state::<AppState>().0.clone();
    let settings = core.settings.get();

    if let Some(corrected) = sync_login_item(&core, settings.launch_at_login) {
        emit_settings(&handle, corrected);
    }

    let keep = KeepAlive::default();
    // Unsupported until the Windows body lands (stage 6): the app still starts, without
    // auto-lock on sleep and screen lock there.
    if let Ok(subscription) = attach_platform_events(&core, core.platform()) {
        keep.push(subscription);
    }
    app.manage(keep);

    let report_handle = handle.clone();
    let watcher = ExternalWatcher::new(
        core.clone(),
        Arc::new(move |status| emit_save_state(&report_handle, status)),
    );
    start_external_poll(watcher.clone(), core.scheduler(), EXTERNAL_POLL);
    app.manage(watcher);

    app.set_theme(native_theme(settings.theme));
    apply_startup_ui(&handle, &core, &settings);
}

fn apply_startup_ui(handle: &AppHandle, core: &Arc<AppCore>, settings: &Settings) {
    if settings.show_menu_bar_extra {
        // `setup` runs on the main thread.
        let _ = create_tray(handle, &current_tray_model(core, None));
    }
    // Held until the main page has loaded: nothing listens yet.
    if let Some(gate) = handle.try_state::<Arc<ConflictGate>>() {
        emit_conflicts(handle, gate.submit(hotkeys::register_all(handle, settings)));
    }
    if let Some(shell) = handle.try_state::<ShellState>() {
        let _ = show_main(shell.windows.as_ref());
    }
}

// ---------------------------------------------------------------------------------------------
// Window and run events

/// `on_window_event`: the close rule of the main window, the panel hiding on blur, and the
/// external-change check on focus.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    match (window.label(), event) {
        (MAIN_WINDOW, WindowEvent::CloseRequested { api, .. }) => {
            api.prevent_close();
            match close_action(current_os(), tray_visible(app)) {
                CloseAction::HideWindow => {
                    let _ = window.hide();
                }
                CloseAction::QuitApp => app.exit(0),
            }
        }
        (MAIN_WINDOW, WindowEvent::Focused(true)) => {
            if let Some(watcher) = app.try_state::<Arc<ExternalWatcher>>() {
                let watcher = Arc::clone(&watcher);
                // Disk work: off the event-loop thread.
                tauri::async_runtime::spawn_blocking(move || watcher.check());
            }
        }
        (PANEL_WINDOW, WindowEvent::CloseRequested { api, .. }) => {
            api.prevent_close();
            let _ = window.hide();
        }
        (PANEL_WINDOW, WindowEvent::Focused(false)) => {
            let _ = window.hide();
        }
        _ => {}
    }
}

/// `on_page_load`: when the main window's page has loaded, emits the shortcut conflicts held
/// since startup (once). A Rust hook, not a webview-callable command.
pub fn on_page_load(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() != MAIN_WINDOW || payload.event() != tauri::webview::PageLoadEvent::Finished
    {
        return;
    }
    let app = webview.app_handle();
    if let Some(gate) = app.try_state::<Arc<ConflictGate>>() {
        emit_conflicts(app, gate.main_page_loaded());
    }
}

/// `RunEvent` handler: the quit flow (`ExitRequested`) and the Dock reopen. Generic over the
/// runtime so the tests drive it with the mock runtime.
pub fn on_run_event<R: Runtime>(app: &tauri::AppHandle<R>, event: &RunEvent) {
    match event {
        RunEvent::ExitRequested { api, .. } => {
            let Some(gate) = app.try_state::<ExitGate>() else {
                return;
            };
            match gate.request() {
                ExitRequest::Allow => {}
                ExitRequest::AlreadyRunning => api.prevent_exit(),
                ExitRequest::Start => {
                    api.prevent_exit();
                    start_quit_flow(app.clone());
                }
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => {
            if let Some(shell) = app.try_state::<ShellState>() {
                let _ = show_main(shell.windows.as_ref());
            }
        }
        _ => {}
    }
}

/// Runs the quit flow on a worker (it flushes to disk and shows a blocking dialog). A confirmed
/// exit calls `exit` again, which the gate lets through. If the worker cannot start, the app
/// stays open: nothing is lost.
fn start_quit_flow<R: Runtime>(app: tauri::AppHandle<R>) {
    let worker_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("mypass-quit".into())
        .spawn(move || {
            let app = worker_app;
            let outcome = match (
                app.try_state::<AppState>(),
                app.try_state::<Arc<dyn QuitPort>>(),
            ) {
                (Some(core), Some(port)) => {
                    let core = core.0.clone();
                    let port = Arc::clone(&port);
                    let language = core.settings.get().language;
                    let ask = |error: &str| port.confirm_quit_anyway(language, error);
                    let outcome = on_exit_requested(core.as_ref(), &ask);
                    if outcome == ExitOutcome::Exit {
                        port.exit();
                    }
                    outcome
                }
                _ => ExitOutcome::Stay,
            };
            if let Some(gate) = app.try_state::<ExitGate>() {
                gate.finish(outcome);
            }
        });
    if spawned.is_err() {
        if let Some(gate) = app.try_state::<ExitGate>() {
            gate.finish(ExitOutcome::Stay);
        }
    }
}

/// [`QuitPort`] over the real app: the dialog plugin and `exit`.
pub struct TauriQuitPort(pub AppHandle);

impl QuitPort for TauriQuitPort {
    fn confirm_quit_anyway(&self, language: Language, error: &str) -> bool {
        // The first button is the default one (Return), so it is Cancel, the safe answer: the
        // dialog API has no separate "default button" setting. `blocking_show` is true when the
        // FIRST button was pressed, so Quit Anyway is the negation.
        let cancelled = self
            .0
            .dialog()
            .message(i18n::t(
                language,
                "The vault could not be saved (%@). If you quit anyway, MyPass tries to keep your unsaved changes as a recovery copy in the backups folder.",
                &[error],
            ))
            .title(i18n::t(language, "Quit MyPass", &[]))
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                i18n::t(language, "Cancel", &[]),
                i18n::t(language, "Quit Anyway", &[]),
            ))
            .blocking_show();
        !cancelled
    }

    fn exit(&self) {
        self.0.exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{dirty_world_with_failing_writes, open_world, World};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    /// A quit port with a programmed answer; reports every call on a channel.
    struct FakePort {
        answer: bool,
        calls: mpsc::Sender<String>,
    }

    impl QuitPort for FakePort {
        fn confirm_quit_anyway(&self, _language: Language, error: &str) -> bool {
            let _ = self.calls.send(format!("ask {error}"));
            self.answer
        }
        fn exit(&self) {
            let _ = self.calls.send("exit".into());
        }
    }

    /// Runs the real `ExitRequested` path on the mock runtime: the app's last window closes, the
    /// mock event loop raises `ExitRequested`, and `on_run_event`, `ExitGate`, the quit worker
    /// and `on_exit_requested` run as in production, with only the dialog and the final exit
    /// behind the fake port. The loop keeps running after a prevented exit, so its thread is
    /// left to die with the test process.
    fn drive_exit_requested(world: &World, answer: bool) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel();
        let core = Arc::new(world.core.clone());
        std::thread::spawn(move || {
            let app = mock_builder()
                .build(mock_context(noop_assets()))
                .expect("the mock app builds");
            app.manage(AppState(core));
            app.manage(ExitGate::default());
            app.manage::<Arc<dyn QuitPort>>(Arc::new(FakePort { answer, calls: tx }));
            WebviewWindowBuilder::new(&app, MAIN_WINDOW, WebviewUrl::App("index.html".into()))
                .build()
                .expect("the mock window builds");
            app.run(|app, event| {
                if matches!(event, RunEvent::Ready) {
                    // The last window closing is what makes the mock loop raise `ExitRequested`
                    // (a close before the loop runs is handled inline, so it waits for `Ready`).
                    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                        window.as_ref().window().close().expect("close is queued");
                    }
                }
                on_run_event(app, &event);
            });
        });
        rx
    }

    fn next(rx: &mpsc::Receiver<String>) -> String {
        rx.recv_timeout(Duration::from_secs(15))
            .expect("the quit flow reported in time")
    }

    fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn exit_requested_smoke_clean_quit_exits_without_a_prompt() {
        let w = open_world();
        w.login("Saved by the final flush");
        let rx = drive_exit_requested(&w, false);
        assert_eq!(next(&rx), "exit", "no prompt on a clean flush");
        assert!(w.core.is_quitting());
    }

    #[test]
    fn exit_requested_smoke_failing_flush_and_cancel_stays_and_accepts_edits() {
        let w = dirty_world_with_failing_writes();
        let rx = drive_exit_requested(&w, false);
        assert!(next(&rx).starts_with("ask "), "the user is asked");
        wait_until("the session to re-open", || !w.core.is_quitting());
        // No exit followed the Cancel, and the session takes edits again.
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        w.login("After cancel");
    }

    #[test]
    fn exit_requested_smoke_failing_flush_and_quit_anyway_exits_with_the_gate_set() {
        let w = dirty_world_with_failing_writes();
        let rx = drive_exit_requested(&w, true);
        assert!(next(&rx).starts_with("ask "));
        assert_eq!(next(&rx), "exit");
        assert!(w.core.is_quitting());
    }
}
