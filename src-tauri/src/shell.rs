//! Pure decision logic of the app shell: which side effects a settings change needs, what closing
//! the window or quitting does, how a launch-at-login request is reconciled with the OS, where the
//! tray panel goes, and what a second launch does.
//!
//! Nothing here needs a running Tauri app or a window server, so every rule is unit-tested in CI.
//! The wiring (commands, tray, hotkeys, exit handling) calls these functions and performs the
//! effects they name. The only Tauri type is [`TauriWindowOps`], the thin production
//! implementation of the [`WindowOps`] seam.

use std::sync::Mutex;

use crate::commands::app::MAIN_WINDOW;
use mypass_platform::{LaunchAtLogin, LoginItemStatus, PlatformError};
use mypass_session::session::SessionError;
use mypass_session::settings::{Language, Settings, Theme};
use tauri::Manager;

/// A side effect the shell must perform after a settings change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Turn launch at login on or off, through `Platform.launch_at_login` (see
    /// [`reconcile_login_item`]).
    SetAutostart(bool),
    /// Show or hide the tray icon.
    SetTrayVisible(bool),
    /// Re-register the global hotkeys from the current settings.
    RegisterHotkeys,
    /// Apply the colour theme to the native windows.
    ApplyTheme(Theme),
    /// Apply the language. Always followed by [`Effect::RebuildTrayMenu`] so the tray is never
    /// stale. The language may still be `System`; the caller resolves it.
    ApplyLanguage(Language),
    /// Rebuild the tray menu (labels follow the language).
    RebuildTrayMenu,
    /// Restart the idle auto-lock timer with the new timeout.
    RearmAutoLock,
}

/// The effects needed to move from `old` to `new` settings: only what changed, in a fixed order
/// (autostart, theme, language, tray visibility, hotkeys, auto-lock timeout).
pub fn settings_effects(old: &Settings, new: &Settings) -> Vec<Effect> {
    let mut effects = Vec::new();
    if old.launch_at_login != new.launch_at_login {
        effects.push(Effect::SetAutostart(new.launch_at_login));
    }
    if old.theme != new.theme {
        effects.push(Effect::ApplyTheme(new.theme));
    }
    if old.language != new.language {
        effects.push(Effect::ApplyLanguage(new.language));
        effects.push(Effect::RebuildTrayMenu);
    }
    if old.show_menu_bar_extra != new.show_menu_bar_extra {
        effects.push(Effect::SetTrayVisible(new.show_menu_bar_extra));
    }
    if old.shortcut_lock != new.shortcut_lock || old.shortcut_show != new.shortcut_show {
        effects.push(Effect::RegisterHotkeys);
    }
    if old.auto_lock_idle_timeout != new.auto_lock_idle_timeout {
        effects.push(Effect::RearmAutoLock);
    }
    effects
}

/// The OS name every OS-dependent shell rule is fed with (`close_action`, the hotkey rendering).
/// The one place that reads `std::env::consts::OS`; macOS is `"macos"`.
pub fn current_os() -> &'static str {
    std::env::consts::OS
}

/// Shows the main window and then focuses it (a hidden window has to be visible to take focus).
/// The one path of the second launch, the tray, the hotkey and the Dock reopen.
pub fn show_main(windows: &dyn WindowOps) -> Result<(), String> {
    windows.show(MAIN_WINDOW)?;
    windows.focus(MAIN_WINDOW)
}

/// What closing the main window does.
#[derive(Debug, PartialEq, Eq)]
pub enum CloseAction {
    HideWindow,
    QuitApp,
}

/// `os` is `std::env::consts::OS`. macOS always hides (Cmd+Q quits); elsewhere the window hides
/// only while the tray icon is visible, otherwise closing it would leave no way back.
pub fn close_action(os: &str, tray_visible: bool) -> CloseAction {
    if os == "macos" || tray_visible {
        CloseAction::HideWindow
    } else {
        CloseAction::QuitApp
    }
}

/// Whether quitting must first ask to discard the editor's unsaved changes: the webview reported
/// a dirty editor. The prompt comes before `begin_quit`, so a Cancel leaves everything untouched.
pub fn discard_edits_prompt_needed(editor_dirty: bool) -> bool {
    editor_dirty
}

/// What to do after the final flush on quit.
#[derive(Debug, PartialEq, Eq)]
pub enum QuitDecision {
    Proceed,
    /// Ask the user (Cancel / Quit Anyway). `error` is the failure text (no secrets); the
    /// dialog builds its localised message around it.
    PromptUser {
        error: String,
    },
}

/// A failed flush must never exit silently: unsaved edits would be lost.
pub fn quit_decision(flush: Result<(), SessionError>) -> QuitDecision {
    match flush {
        Ok(()) => QuitDecision::Proceed,
        Err(error) => QuitDecision::PromptUser {
            error: error.to_string(),
        },
    }
}

/// The result of reconciling the launch-at-login setting with the OS. The OS state wins:
/// every variant except `InSync` and `Applied` tells the caller to write `setting_should_be`
/// back through `SettingsStore::update` and emit `SettingsChanged`.
#[derive(Debug, PartialEq, Eq)]
pub enum LoginItemOutcome {
    /// The OS already matched the desired state; nothing was changed.
    InSync,
    /// The OS was changed and now matches.
    Applied,
    /// Registered but waiting for OS approval. Windows preserves user intent independently
    /// of this effective status; macOS writes the effective state back.
    NeedsApproval { setting_should_be: bool },
    /// The OS refused or ended in another state than requested.
    Failed {
        setting_should_be: bool,
        message: String,
    },
    /// This platform has no login item (macOS until stage 5).
    Unsupported { setting_should_be: bool },
}

/// Bring the OS login item to `desired` (startup and `SetAutostart`). Never panics.
pub fn reconcile_login_item(desired: bool, launcher: &dyn LaunchAtLogin) -> LoginItemOutcome {
    if let Err(error) = launcher.prepare_startup(desired) {
        return LoginItemOutcome::Failed {
            setting_should_be: false,
            message: error.to_string(),
        };
    }
    let status = match launcher.status() {
        Ok(status) => status,
        Err(PlatformError::Unsupported(_)) => {
            return LoginItemOutcome::Unsupported {
                setting_should_be: false,
            }
        }
        Err(error) => {
            return LoginItemOutcome::Failed {
                setting_should_be: false,
                message: error.to_string(),
            }
        }
    };
    match status {
        LoginItemStatus::Unsupported => {
            return LoginItemOutcome::Unsupported {
                setting_should_be: false,
            }
        }
        LoginItemStatus::NotFound if desired => {
            return LoginItemOutcome::Failed {
                setting_should_be: false,
                message: "the system cannot find the app to register".to_string(),
            }
        }
        LoginItemStatus::NotFound => return LoginItemOutcome::InSync,
        // Already registered and only waiting for the user: registering again cannot help.
        LoginItemStatus::RequiresApproval if desired => {
            return LoginItemOutcome::NeedsApproval {
                setting_should_be: launcher.preserve_intent_on_approval(),
            }
        }
        LoginItemStatus::Enabled if desired => return LoginItemOutcome::InSync,
        LoginItemStatus::Disabled if !desired => return LoginItemOutcome::InSync,
        _ => {}
    }
    if let Err(error) = launcher.set_enabled(desired) {
        let actual = launcher.is_enabled().unwrap_or(false);
        return match error {
            PlatformError::Unsupported(_) => LoginItemOutcome::Unsupported {
                setting_should_be: false,
            },
            other => LoginItemOutcome::Failed {
                setting_should_be: actual,
                message: other.to_string(),
            },
        };
    }
    match launcher.status() {
        Ok(LoginItemStatus::Enabled) if desired => LoginItemOutcome::Applied,
        Ok(LoginItemStatus::Disabled) if !desired => LoginItemOutcome::Applied,
        Ok(LoginItemStatus::RequiresApproval) if desired => LoginItemOutcome::NeedsApproval {
            setting_should_be: launcher.preserve_intent_on_approval(),
        },
        Ok(LoginItemStatus::RequiresApproval) => LoginItemOutcome::Applied,
        other => {
            let actual = matches!(other, Ok(LoginItemStatus::Enabled));
            LoginItemOutcome::Failed {
                setting_should_be: actual,
                message: "the login item is not in the requested state".to_string(),
            }
        }
    }
}

/// The window operations the shell needs, behind a seam so policy code is testable and stage 3
/// (pairing raises the main window) can reuse it.
pub trait WindowOps: Send + Sync {
    fn show(&self, label: &str) -> Result<(), String>;
    fn hide(&self, label: &str) -> Result<(), String>;
    fn focus(&self, label: &str) -> Result<(), String>;
    fn set_always_on_top(&self, label: &str, on: bool) -> Result<(), String>;
}

/// [`WindowOps`] over real Tauri windows, looked up by label.
pub struct TauriWindowOps(pub tauri::AppHandle);

impl TauriWindowOps {
    fn window(&self, label: &str) -> Result<tauri::WebviewWindow, String> {
        self.0
            .get_webview_window(label)
            .ok_or_else(|| format!("no window labelled {label}"))
    }
}

impl WindowOps for TauriWindowOps {
    fn show(&self, label: &str) -> Result<(), String> {
        self.window(label)?.show().map_err(|e| e.to_string())
    }
    fn hide(&self, label: &str) -> Result<(), String> {
        self.window(label)?.hide().map_err(|e| e.to_string())
    }
    fn focus(&self, label: &str) -> Result<(), String> {
        self.window(label)?.set_focus().map_err(|e| e.to_string())
    }
    fn set_always_on_top(&self, label: &str, on: bool) -> Result<(), String> {
        self.window(label)?
            .set_always_on_top(on)
            .map_err(|e| e.to_string())
    }
}

/// [`WindowOps`] that records its calls, for tests.
#[derive(Debug, Default)]
pub struct FakeWindowOps {
    calls: Mutex<Vec<(String, String, Option<bool>)>>,
}

impl FakeWindowOps {
    /// The recorded calls as `(op, label, flag)`; `op` is `show`, `hide`, `focus` or `set_always_on_top`.
    pub fn calls(&self) -> Vec<(String, String, Option<bool>)> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn record(&self, op: &str, label: &str, flag: Option<bool>) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((op.to_string(), label.to_string(), flag));
        Ok(())
    }
}

impl WindowOps for FakeWindowOps {
    fn show(&self, label: &str) -> Result<(), String> {
        self.record("show", label, None)
    }
    fn hide(&self, label: &str) -> Result<(), String> {
        self.record("hide", label, None)
    }
    fn focus(&self, label: &str) -> Result<(), String> {
        self.record("focus", label, None)
    }
    fn set_always_on_top(&self, label: &str, on: bool) -> Result<(), String> {
        self.record("set_always_on_top", label, Some(on))
    }
}

/// What a second launch of the app does. One process means never a second writer: the first
/// instance shows and focuses `main` and leaves a locked session locked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecondInstance {
    ShowMainWindow {
        /// An argument ending in `.kdbx`, carried for stage-5 file association; ignored today.
        vault_path: Option<String>,
    },
}

/// Decide what the single-instance callback does with the second launch's `argv`.
pub fn second_instance_action(argv: &[String]) -> SecondInstance {
    let vault_path = argv
        .iter()
        .find(|arg| arg.to_ascii_lowercase().ends_with(".kdbx"))
        .cloned();
    SecondInstance::ShowMainWindow { vault_path }
}

/// A rectangle in physical or logical pixels (the caller keeps one unit throughout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Gap kept between the panel and the monitor edge or the tray icon.
const PANEL_MARGIN: f64 = 8.0;

/// Clamp without the panic of `f64::clamp` on an inverted range (a panel larger than the
/// monitor pins to the lower bound).
fn clamp_to(value: f64, low: f64, high: f64) -> f64 {
    value.min(high).max(low)
}

/// Top-left of the tray panel: centred under the tray icon, clamped inside the monitor with an
/// 8 px margin, and flipped above the icon when the icon is in the bottom half of the monitor
/// (a Windows taskbar).
pub fn panel_position(
    tray: PanelRect,
    monitor: PanelRect,
    panel_w: f64,
    panel_h: f64,
) -> (f64, f64) {
    let centred_x = tray.x + tray.w / 2.0 - panel_w / 2.0;
    let in_bottom_half = tray.y + tray.h / 2.0 > monitor.y + monitor.h / 2.0;
    let y = if in_bottom_half {
        tray.y - panel_h - PANEL_MARGIN
    } else {
        tray.y + tray.h + PANEL_MARGIN
    };
    let x = clamp_to(
        centred_x,
        monitor.x + PANEL_MARGIN,
        monitor.x + monitor.w - panel_w - PANEL_MARGIN,
    );
    let y = clamp_to(
        y,
        monitor.y + PANEL_MARGIN,
        monitor.y + monitor.h - panel_h - PANEL_MARGIN,
    );
    (x, y)
}

/// A monitor as the runtime reports it: physical origin and size, and its own scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorFrame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

impl MonitorFrame {
    /// The monitor in logical points (physical divided by its own scale), the space a macOS
    /// window is positioned in.
    fn logical(&self) -> PanelRect {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        PanelRect {
            x: self.x / scale,
            y: self.y / scale,
            w: self.w / scale,
            h: self.h / scale,
        }
    }

    fn scale(&self) -> f64 {
        if self.scale > 0.0 {
            self.scale
        } else {
            1.0
        }
    }
}

/// The tray icon rectangle exactly as the tray event delivered it, with its unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayRect {
    Physical(PanelRect),
    Logical(PanelRect),
}

fn contains(frame: PanelRect, px: f64, py: f64) -> bool {
    px >= frame.x && px < frame.x + frame.w && py >= frame.y && py < frame.y + frame.h
}

fn scaled(rect: PanelRect, factor: f64) -> PanelRect {
    PanelRect {
        x: rect.x * factor,
        y: rect.y * factor,
        w: rect.w * factor,
        h: rect.h * factor,
    }
}

/// Top-left of the tray panel in logical points (the unit macOS positions windows in).
///
/// The tray icon lives on exactly one monitor, and that monitor's scale factor (never the panel
/// window's, which belongs to wherever the panel was last shown) converts the icon between
/// physical pixels and logical points. The monitor is found in global logical points, where the
/// monitors never overlap: by `click`, the click location in unscaled global points (the caller
/// derives it from the cursor, which sits on the icon at click time), or, for an icon that is
/// already logical, by the icon's own center. A physical icon rectangle is never matched against
/// the physical monitor frames: tao places each monitor at its logical origin times that monitor's
/// own scale, so those frames overlap on mixed-DPI desktops and the first match would be wrong.
/// With no way to tell, `fallback` (the primary monitor's index) is used with its scale; `None`
/// only when there is no monitor at all. `panel` is the panel size in logical points.
pub fn tray_panel_origin(
    tray: TrayRect,
    click: Option<(f64, f64)>,
    monitors: &[MonitorFrame],
    fallback: Option<usize>,
    panel: (f64, f64),
) -> Option<(f64, f64)> {
    let point = click.or(match tray {
        TrayRect::Logical(r) => Some((r.x + r.w / 2.0, r.y + r.h / 2.0)),
        TrayRect::Physical(_) => None,
    });
    let holder = point.and_then(|(x, y)| monitors.iter().position(|m| contains(m.logical(), x, y)));
    let index = holder
        .or(fallback)
        .filter(|i| *i < monitors.len())
        .or(if monitors.is_empty() { None } else { Some(0) })?;
    let monitor = monitors.get(index)?;
    let logical_tray = match tray {
        TrayRect::Physical(r) => scaled(r, 1.0 / monitor.scale()),
        TrayRect::Logical(r) => r,
    };
    Some(panel_position(
        logical_tray,
        monitor.logical(),
        panel.0,
        panel.1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypass_platform::testing::FakeLaunchAtLogin;
    use mypass_platform::{PlatformError, PlatformResult};
    use mypass_session::settings::{IdleTimeout, Language, Settings, Theme};

    #[test]
    fn effects_empty_when_nothing_changed() {
        let s = Settings::default();
        assert_eq!(settings_effects(&s, &s.clone()), Vec::<Effect>::new());
    }

    #[test]
    fn effects_for_autostart_theme_tray_hotkeys_and_timeout() {
        let old = Settings::default();
        let mut new = old.clone();
        new.launch_at_login = true;
        new.theme = Theme::Dark;
        new.show_menu_bar_extra = false;
        new.shortcut_lock = "CommandOrControl+Shift+K".to_string();
        new.auto_lock_idle_timeout = IdleTimeout::FiveMinutes;
        assert_eq!(
            settings_effects(&old, &new),
            vec![
                Effect::SetAutostart(true),
                Effect::ApplyTheme(Theme::Dark),
                Effect::SetTrayVisible(false),
                Effect::RegisterHotkeys,
                Effect::RearmAutoLock,
            ]
        );
    }

    #[test]
    fn show_shortcut_change_registers_hotkeys() {
        let old = Settings::default();
        let mut new = old.clone();
        new.shortcut_show = "CommandOrControl+Shift+M".to_string();
        assert_eq!(settings_effects(&old, &new), vec![Effect::RegisterHotkeys]);
    }

    #[test]
    fn language_change_emits_apply_language_then_rebuild_tray_menu() {
        let old = Settings::default();
        let mut new = old.clone();
        new.language = Language::Uk;
        assert_eq!(
            settings_effects(&old, &new),
            vec![Effect::ApplyLanguage(Language::Uk), Effect::RebuildTrayMenu]
        );
    }

    #[test]
    fn reconcile_login_item_in_sync_does_nothing() {
        let fake = FakeLaunchAtLogin::new(true);
        assert_eq!(reconcile_login_item(true, &*fake), LoginItemOutcome::InSync);
        assert_eq!(fake.set_calls(), 0);
    }

    #[test]
    fn reconcile_login_item_applies_difference() {
        let fake = FakeLaunchAtLogin::new(false);
        assert_eq!(
            reconcile_login_item(true, &*fake),
            LoginItemOutcome::Applied
        );
        assert_eq!(fake.is_enabled(), Ok(true));
        assert_eq!(
            reconcile_login_item(false, &*fake),
            LoginItemOutcome::Applied
        );
        assert_eq!(fake.is_enabled(), Ok(false));
    }

    #[test]
    fn reconcile_login_item_failure_reports_setting_should_be_actual_state() {
        let fake = FakeLaunchAtLogin::new(false);
        fake.fail_next(PlatformError::Failed("boom".into()));
        match reconcile_login_item(true, &*fake) {
            LoginItemOutcome::Failed {
                setting_should_be,
                message,
            } => {
                assert!(!setting_should_be);
                assert!(message.contains("boom"));
            }
            other => panic!("unexpected {other:?}"),
        }
        let fake = FakeLaunchAtLogin::new(true);
        fake.fail_next(PlatformError::Failed("boom".into()));
        assert!(matches!(
            reconcile_login_item(false, &*fake),
            LoginItemOutcome::Failed {
                setting_should_be: true,
                ..
            }
        ));
    }

    /// A launcher whose status is `Unsupported`, like the macOS fallback until stage 5.
    struct Unsupported;
    impl LaunchAtLogin for Unsupported {
        fn status(&self) -> PlatformResult<LoginItemStatus> {
            Ok(LoginItemStatus::Unsupported)
        }
        fn set_enabled(&self, _: bool) -> PlatformResult<()> {
            Err(PlatformError::Unsupported("launch at login"))
        }
    }

    #[test]
    fn launch_at_login_unsupported_reports_setting_false() {
        assert_eq!(
            reconcile_login_item(true, &Unsupported),
            LoginItemOutcome::Unsupported {
                setting_should_be: false
            }
        );
        assert_eq!(
            reconcile_login_item(false, &Unsupported),
            LoginItemOutcome::Unsupported {
                setting_should_be: false
            }
        );
    }

    #[test]
    fn reconcile_login_item_requires_approval_writes_setting_false_and_reports_needs_approval() {
        let fake = FakeLaunchAtLogin::new(false);
        // The fake follows `set_enabled`, so model the OS answering "approval needed" with a
        // launcher that parks in that status after enabling.
        struct Approval(std::sync::Arc<FakeLaunchAtLogin>);
        impl LaunchAtLogin for Approval {
            fn status(&self) -> PlatformResult<LoginItemStatus> {
                self.0.status()
            }
            fn set_enabled(&self, e: bool) -> PlatformResult<()> {
                self.0.set_enabled(e)?;
                self.0.set_status(LoginItemStatus::RequiresApproval);
                Ok(())
            }
        }
        assert_eq!(
            reconcile_login_item(true, &Approval(fake)),
            LoginItemOutcome::NeedsApproval {
                setting_should_be: false
            }
        );
    }

    #[test]
    fn reconcile_login_item_pending_approval_is_reported_without_set_enabled() {
        let fake = FakeLaunchAtLogin::new(false);
        fake.set_status(LoginItemStatus::RequiresApproval);
        assert_eq!(
            reconcile_login_item(true, &*fake),
            LoginItemOutcome::NeedsApproval {
                setting_should_be: false
            }
        );
        assert_eq!(fake.set_calls(), 0);
    }

    #[test]
    fn reconcile_login_item_not_found_writes_setting_false() {
        let fake = FakeLaunchAtLogin::new(false);
        fake.set_status(LoginItemStatus::NotFound);
        assert!(matches!(
            reconcile_login_item(true, &*fake),
            LoginItemOutcome::Failed {
                setting_should_be: false,
                ..
            }
        ));
        assert_eq!(fake.set_calls(), 0);
    }

    #[test]
    fn fake_window_ops_records_show_focus_and_always_on_top() {
        let ops = FakeWindowOps::default();
        ops.show("main").unwrap();
        ops.focus("main").unwrap();
        ops.set_always_on_top("panel", true).unwrap();
        assert_eq!(
            ops.calls(),
            vec![
                ("show".to_string(), "main".to_string(), None),
                ("focus".to_string(), "main".to_string(), None),
                (
                    "set_always_on_top".to_string(),
                    "panel".to_string(),
                    Some(true)
                ),
            ]
        );
    }

    #[test]
    fn close_action_table() {
        assert_eq!(close_action("macos", true), CloseAction::HideWindow);
        assert_eq!(close_action("macos", false), CloseAction::HideWindow);
        assert_eq!(close_action("windows", true), CloseAction::HideWindow);
        assert_eq!(close_action("windows", false), CloseAction::QuitApp);
    }

    #[test]
    fn quit_with_failed_flush_prompts_instead_of_exiting() {
        match quit_decision(Err(SessionError::Io("disk full".into()))) {
            QuitDecision::PromptUser { error } => assert!(error.contains("disk full")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn quit_with_ok_flush_proceeds() {
        assert_eq!(quit_decision(Ok(())), QuitDecision::Proceed);
    }

    #[test]
    fn discard_prompt_is_needed_only_for_a_dirty_editor() {
        assert!(discard_edits_prompt_needed(true));
        assert!(!discard_edits_prompt_needed(false));
    }

    #[test]
    fn second_instance_always_shows_main() {
        assert_eq!(
            second_instance_action(&["mypass".to_string()]),
            SecondInstance::ShowMainWindow { vault_path: None }
        );
        assert_eq!(
            second_instance_action(&["mypass".to_string(), "/tmp/v.KDBX".to_string()]),
            SecondInstance::ShowMainWindow {
                vault_path: Some("/tmp/v.KDBX".to_string())
            }
        );
        assert_eq!(
            second_instance_action(&[]),
            SecondInstance::ShowMainWindow { vault_path: None }
        );
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> PanelRect {
        PanelRect { x, y, w, h }
    }

    #[test]
    fn panel_position_centres_under_tray_icon() {
        let pos = panel_position(
            rect(1000.0, 0.0, 24.0, 24.0),
            rect(0.0, 0.0, 1440.0, 900.0),
            320.0,
            420.0,
        );
        assert_eq!(pos, (852.0, 32.0));
    }

    #[test]
    fn panel_position_clamps_to_right_edge() {
        let pos = panel_position(
            rect(1430.0, 0.0, 24.0, 24.0),
            rect(0.0, 0.0, 1440.0, 900.0),
            320.0,
            420.0,
        );
        assert_eq!(pos.0, 1112.0);
    }

    #[test]
    fn panel_position_clamps_to_left_edge() {
        let pos = panel_position(
            rect(0.0, 0.0, 24.0, 24.0),
            rect(0.0, 0.0, 1440.0, 900.0),
            320.0,
            420.0,
        );
        assert_eq!(pos.0, 8.0);
    }

    fn frame(x: f64, y: f64, w: f64, h: f64, scale: f64) -> MonitorFrame {
        MonitorFrame { x, y, w, h, scale }
    }

    /// A retina laptop (scale 2, 1440x900 points) with a 1920x1080 scale-1 monitor to its right,
    /// the way tao reports them: the origin is the logical origin times the monitor's own scale.
    fn mixed_monitors() -> [MonitorFrame; 2] {
        [
            frame(0.0, 0.0, 2880.0, 1800.0, 2.0),
            frame(1440.0, 0.0, 1920.0, 1080.0, 1.0),
        ]
    }

    const PANEL: (f64, f64) = (320.0, 420.0);

    /// Both orders the OS can list the monitors in; the answer must not depend on it.
    fn both_orders() -> [Vec<MonitorFrame>; 2] {
        let [retina, neighbour] = mixed_monitors();
        [vec![retina, neighbour], vec![neighbour, retina]]
    }

    #[test]
    fn tray_panel_uses_the_scale_of_the_monitor_holding_a_logical_icon() {
        // The icon sits on the scale-1 monitor (logical x 2000); the panel window last lived on
        // the scale-2 one. The icon is already logical, and its monitor decides the frame.
        for monitors in both_orders() {
            let origin = tray_panel_origin(
                TrayRect::Logical(rect(2000.0, 0.0, 24.0, 24.0)),
                None,
                &monitors,
                Some(0),
                PANEL,
            )
            .unwrap();
            assert_eq!(origin, (1852.0, 32.0));
        }
    }

    #[test]
    fn tray_panel_overlapping_physical_frames_are_told_apart_by_the_click_point() {
        // The physical frames overlap (2x primary at physical x 0..2880, 1x neighbour at
        // 1440..3360), so a physical icon at x 2000 is in both. The click location in global
        // points settles it, whichever order the monitors are listed in.
        for monitors in both_orders() {
            // Icon on the 1x neighbour: physical x 2000 is logical 2000, the click is at 2012.
            let neighbour = tray_panel_origin(
                TrayRect::Physical(rect(2000.0, 0.0, 24.0, 24.0)),
                Some((2012.0, 12.0)),
                &monitors,
                Some(0),
                PANEL,
            )
            .unwrap();
            assert_eq!(neighbour, (1852.0, 32.0), "neighbour, {monitors:?}");
            // Icon on the 2x primary: physical x 2000 is logical 1000, the click is at 1012.
            let primary = tray_panel_origin(
                TrayRect::Physical(rect(2000.0, 0.0, 48.0, 48.0)),
                Some((1012.0, 12.0)),
                &monitors,
                Some(1),
                PANEL,
            )
            .unwrap();
            assert_eq!(primary, (852.0, 32.0), "primary, {monitors:?}");
        }
    }

    #[test]
    fn tray_panel_clamps_inside_the_holding_monitor_not_the_whole_desktop() {
        // An icon at the far right of the scale-2 monitor stays inside that monitor's 1440 points.
        for monitors in both_orders() {
            let (x, _) = tray_panel_origin(
                TrayRect::Physical(rect(2820.0, 0.0, 48.0, 48.0)),
                Some((1430.0, 12.0)),
                &monitors,
                Some(0),
                PANEL,
            )
            .unwrap();
            assert_eq!(x, 1440.0 - 320.0 - 8.0);
        }
    }

    #[test]
    fn tray_panel_without_a_click_point_uses_the_primary_monitor_and_its_scale() {
        // No click location and a physical icon: never guess from the overlapping frames.
        for monitors in both_orders() {
            let primary = monitors.iter().position(|m| m.scale == 2.0);
            let origin = tray_panel_origin(
                TrayRect::Physical(rect(2000.0, 0.0, 48.0, 48.0)),
                None,
                &monitors,
                primary,
                PANEL,
            )
            .unwrap();
            assert_eq!(origin, (852.0, 32.0), "{monitors:?}");
        }
    }

    #[test]
    fn tray_panel_falls_back_to_the_primary_monitor_for_a_click_outside_every_monitor() {
        let origin = tray_panel_origin(
            TrayRect::Physical(rect(5000.0, 0.0, 48.0, 48.0)),
            Some((9000.0, 0.0)),
            &mixed_monitors(),
            Some(0),
            PANEL,
        )
        .unwrap();
        // Converted with the primary monitor's scale 2, then clamped into its 1440 points.
        assert_eq!(origin, (1112.0, 32.0));
        // An out-of-range fallback index uses the first monitor.
        assert!(tray_panel_origin(
            TrayRect::Logical(rect(9999.0, 0.0, 24.0, 24.0)),
            None,
            &mixed_monitors(),
            Some(7),
            PANEL,
        )
        .is_some());
    }

    #[test]
    fn tray_panel_has_no_origin_without_monitors_and_survives_a_zero_scale() {
        assert_eq!(
            tray_panel_origin(
                TrayRect::Logical(rect(0.0, 0.0, 24.0, 24.0)),
                None,
                &[],
                None,
                PANEL
            ),
            None
        );
        let broken = [frame(0.0, 0.0, 1440.0, 900.0, 0.0)];
        let origin = tray_panel_origin(
            TrayRect::Physical(rect(1000.0, 0.0, 24.0, 24.0)),
            Some((1012.0, 12.0)),
            &broken,
            None,
            PANEL,
        )
        .unwrap();
        assert_eq!(origin, (852.0, 32.0));
    }

    #[test]
    fn panel_flips_above_when_tray_in_bottom_half() {
        let pos = panel_position(
            rect(1000.0, 876.0, 24.0, 24.0),
            rect(0.0, 0.0, 1440.0, 900.0),
            320.0,
            420.0,
        );
        assert_eq!(pos.1, 448.0);
    }

    #[test]
    fn current_os_is_the_string_close_action_expects_for_macos() {
        // A typo in the macOS string would fall through to the non-macOS rule (quit on close
        // without a tray icon). Pin both the constant and what the running OS reports.
        #[cfg(target_os = "macos")]
        assert_eq!(current_os(), "macos");
        #[cfg(target_os = "windows")]
        assert_eq!(current_os(), "windows");
        assert_eq!(close_action("macos", false), CloseAction::HideWindow);
        assert_eq!(close_action("windows", false), CloseAction::QuitApp);
    }

    #[test]
    fn show_main_shows_then_focuses_the_main_window() {
        let windows = FakeWindowOps::default();
        show_main(&windows).unwrap();
        let calls: Vec<_> = windows.calls().into_iter().map(|c| (c.0, c.1)).collect();
        assert_eq!(
            calls,
            [
                ("show".to_string(), "main".to_string()),
                ("focus".to_string(), "main".to_string())
            ]
        );
    }
}

#[cfg(test)]
mod windows_batch_f_tests {
    use super::*;
    #[test]
    fn reconcile_login_item_with_win_launch_at_login_round_trips() {
        use mypass_platform::{
            windows_launch_at_login::WinLaunchAtLogin, windows_registry::MemoryRegistry,
        };
        let launcher = WinLaunchAtLogin::new(
            std::sync::Arc::new(MemoryRegistry::default()),
            r"C:\MyPass.exe".into(),
        );
        assert_eq!(
            reconcile_login_item(false, &launcher),
            LoginItemOutcome::InSync
        );
        assert_eq!(
            reconcile_login_item(true, &launcher),
            LoginItemOutcome::Applied
        );
        assert_eq!(
            reconcile_login_item(true, &launcher),
            LoginItemOutcome::InSync
        );
        assert_eq!(
            reconcile_login_item(false, &launcher),
            LoginItemOutcome::Applied
        );
    }

    #[test]
    fn windows_approval_fallback_preserves_intent_across_two_launches() {
        use mypass_platform::{
            windows_launch_at_login::WinLaunchAtLogin,
            windows_registry::{Hive, Key, MemoryRegistry, Read, Registry, Value, ValueType, View},
        };
        let run = Key::new(
            Hive::CurrentUser,
            View::Registry64,
            r"Software\Microsoft\Windows\CurrentVersion\Run",
        )
        .unwrap();
        let approved = Key::new(
            Hive::CurrentUser,
            View::Registry64,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
        )
        .unwrap();
        for record in [vec![], vec![2, 0, 0, 0], vec![3, 0, 0, 0], vec![255; 12]] {
            let registry = std::sync::Arc::new(MemoryRegistry::default());
            registry.create(&run).unwrap();
            registry
                .set(
                    &run,
                    "MyPass",
                    Value::String(r#""C:\old\MyPass.exe" --autostarted"#.into()),
                )
                .unwrap();
            registry.create(&approved).unwrap();
            registry
                .set(&approved, "MyPass", Value::Binary(record.clone()))
                .unwrap();
            let mut saved_intent = true;
            for _ in 0..2 {
                let launcher = WinLaunchAtLogin::new(registry.clone(), r"C:\new\MyPass.exe".into());
                let outcome = reconcile_login_item(saved_intent, &launcher);
                // Model the setting persisted by lifecycle::sync_login_item_report.
                if let LoginItemOutcome::NeedsApproval { setting_should_be }
                | LoginItemOutcome::Failed {
                    setting_should_be, ..
                } = outcome
                {
                    saved_intent = setting_should_be;
                }
            }
            assert_eq!(
                registry.read(&run, "MyPass", ValueType::String).unwrap(),
                Read::Value(Value::String(r#""C:\new\MyPass.exe" --autostarted"#.into()))
            );
            assert!(saved_intent);
            let launcher = WinLaunchAtLogin::new(registry.clone(), r"C:\new\MyPass.exe".into());
            assert_eq!(
                launcher.status().unwrap(),
                LoginItemStatus::RequiresApproval
            );
            assert_eq!(
                registry
                    .read(&approved, "MyPass", ValueType::Binary)
                    .unwrap(),
                Read::Value(Value::Binary(record))
            );
            assert_eq!(
                reconcile_login_item(false, &launcher),
                LoginItemOutcome::Applied
            );
            assert_eq!(
                registry.read(&run, "MyPass", ValueType::String).unwrap(),
                Read::Missing
            );
            assert_eq!(
                reconcile_login_item(false, &launcher),
                LoginItemOutcome::InSync
            );
            // Explicit enable with a surviving approval record also preserves intent.
            assert_eq!(
                reconcile_login_item(true, &launcher),
                LoginItemOutcome::NeedsApproval {
                    setting_should_be: true
                }
            );
            // Windows removing its restriction makes the existing registration effective.
            registry.delete_value(&approved, "MyPass").unwrap();
            assert_eq!(
                reconcile_login_item(true, &launcher),
                LoginItemOutcome::InSync
            );
        }
    }

    #[test]
    fn panel_position_windows_table() {
        for scale in [1.0, 1.5, 2.0] {
            let work = PanelRect {
                x: -1920.0,
                y: 0.0,
                w: 1920.0,
                h: 1040.0,
            };
            for tray in [
                PanelRect {
                    x: -50.0,
                    y: 1040.0,
                    w: 32.0,
                    h: 40.0,
                },
                PanelRect {
                    x: -50.0,
                    y: -40.0,
                    w: 32.0,
                    h: 40.0,
                },
                PanelRect {
                    x: -1960.0,
                    y: 500.0,
                    w: 40.0,
                    h: 32.0,
                },
                PanelRect {
                    x: 0.0,
                    y: 500.0,
                    w: 40.0,
                    h: 32.0,
                },
            ] {
                let (x, y) =
                    panel_position_windows(tray, work, 360.0 * scale, 420.0 * scale, scale);
                assert!(x >= work.x && y >= work.y);
                assert!(
                    x + 360.0 * scale <= work.x + work.w && y + 420.0 * scale <= work.y + work.h
                );
                if tray.x < work.x {
                    assert_eq!(x, work.x + 8.0 * scale);
                }
                if tray.x >= work.x + work.w {
                    assert_eq!(x + 360.0 * scale, work.x + work.w - 8.0 * scale);
                }
            }
        }
    }
    #[test]
    fn tray_toggle_debounce_suppresses_blur_then_click() {
        let mut gate = TrayToggleDebounce::default();
        assert!(!gate.suppress(std::time::Duration::ZERO));
        gate.blur(std::time::Duration::from_secs(1));
        assert!(gate.suppress(std::time::Duration::from_millis(1249)));
        gate.blur(std::time::Duration::from_secs(1));
        assert!(!gate.suppress(std::time::Duration::from_millis(1250)));
    }
}

/// Physical pixels throughout. Infer the taskbar edge from the icon relative to the work area.
pub fn panel_position_windows(
    tray: PanelRect,
    work: PanelRect,
    panel_w: f64,
    panel_h: f64,
    scale: f64,
) -> (f64, f64) {
    let margin = PANEL_MARGIN * scale;
    let mut x = tray.x + tray.w / 2.0 - panel_w / 2.0;
    let mut y = tray.y + tray.h + margin;
    if tray.x + tray.w <= work.x {
        x = work.x + margin;
        y = tray.y + tray.h / 2.0 - panel_h / 2.0;
    } else if tray.x >= work.x + work.w {
        x = work.x + work.w - panel_w - margin;
        y = tray.y + tray.h / 2.0 - panel_h / 2.0;
    } else if tray.y >= work.y + work.h {
        y = tray.y - panel_h - margin;
    }
    (
        clamp_to(x, work.x + margin, work.x + work.w - panel_w - margin),
        clamp_to(y, work.y + margin, work.y + work.h - panel_h - margin),
    )
}

/// Monotonic timestamps are supplied by the runtime; tests advance them without sleeping.
#[derive(Default)]
pub struct TrayToggleDebounce {
    last_blur: Option<std::time::Duration>,
}
impl TrayToggleDebounce {
    pub fn blur(&mut self, now: std::time::Duration) {
        self.last_blur = Some(now);
    }
    pub fn suppress(&mut self, now: std::time::Duration) -> bool {
        self.last_blur
            .take()
            .and_then(|blur| now.checked_sub(blur))
            .is_some_and(|elapsed| elapsed < std::time::Duration::from_millis(250))
    }
}
