//! `app_*`: app-level commands (version, activity, windows, reveal in the file manager).

use crate::events::OpenEntryEvent;
use crate::shell::WindowOps;
use crate::state::{AppState, ShellState};
use mypass_session::dto::AppInfo;
use mypass_session::{ApiError, AppCore};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_specta::Event;

/// The label of the main window.
pub const MAIN_WINDOW: &str = "main";
/// The label of the settings window (created on first use).
pub const SETTINGS_WINDOW: &str = "settings";

/// The facade's info with the version of THIS app (the facade reports its own crate's).
pub fn info(core: &AppCore) -> AppInfo {
    info_with_version(core, env!("CARGO_PKG_VERSION"))
}

/// [`info`] with the app version given (so a test can tell it from the facade's own).
pub fn info_with_version(core: &AppCore, version: &str) -> AppInfo {
    AppInfo {
        version: version.to_owned(),
        ..core.app_info()
    }
}

/// Asks the facade to open an entry; when it says yes, shows and focuses the main window and
/// returns the event to emit. `None` (locked, or the entry is gone) leaves every window alone.
pub fn open_entry(
    core: &AppCore,
    windows: &dyn WindowOps,
    id: &str,
    edit: bool,
) -> Result<Option<OpenEntryEvent>, ApiError> {
    let Some(request) = core.open_entry_request(id, edit)? else {
        return Ok(None);
    };
    for result in [windows.show(MAIN_WINDOW), windows.focus(MAIN_WINDOW)] {
        result.map_err(super::internal)?;
    }
    Ok(Some(request.into()))
}

#[tauri::command]
#[specta::specta]
pub fn app_info(state: State<'_, AppState>) -> Result<AppInfo, ApiError> {
    Ok(info(&state.0))
}

/// User activity in a window (resets the idle timer); a no-op while locked.
#[tauri::command]
#[specta::specta]
pub fn app_record_activity(state: State<'_, AppState>) -> Result<(), ApiError> {
    state.0.app_record_activity()
}

#[tauri::command]
#[specta::specta]
pub fn app_open_entry(
    app: AppHandle,
    state: State<'_, AppState>,
    shell: State<'_, ShellState>,
    id: String,
    edit: bool,
) -> Result<(), ApiError> {
    if let Some(event) = open_entry(&state.0, shell.windows.as_ref(), &id, edit)? {
        // A window that is gone is not an error of the request.
        let _ = event.emit(&app);
    }
    Ok(())
}

/// Shows and focuses the main window (the tray panel's Open MyPass). The same path as the tray
/// menu, the hotkey and the second launch.
pub fn show_main_window(windows: &dyn WindowOps) -> Result<(), ApiError> {
    crate::shell::show_main(windows).map_err(super::internal)
}

/// Asks the runtime to exit. This only RAISES `ExitRequested`: the quit flow (`begin_quit`, the
/// final flush, the data-loss prompt, `ExitGate`) decides whether the process really goes, exactly
/// as for the tray menu's Quit, so nothing here can bypass it.
pub fn request_quit<R: tauri::Runtime>(app: &AppHandle<R>) {
    app.exit(0);
}

#[tauri::command]
#[specta::specta]
pub fn app_show_main(shell: State<'_, ShellState>) -> Result<(), ApiError> {
    show_main_window(shell.windows.as_ref())
}

#[tauri::command]
#[specta::specta]
pub fn app_quit(app: AppHandle) -> Result<(), ApiError> {
    request_quit(&app);
    Ok(())
}

/// Reveals the vault's backup folder in the file manager. Called from Rust, so the webview needs
/// no opener capability for it.
#[tauri::command]
#[specta::specta]
pub fn app_reveal_backups(state: State<'_, AppState>) -> Result<(), ApiError> {
    let folder = state.0.backup_folder()?;
    tauri_plugin_opener::reveal_item_in_dir(folder).map_err(|e| super::internal(e.to_string()))
}

/// Shows the settings window, creating it on first use. `async` because creating a window from a
/// synchronous command deadlocks on Windows.
#[tauri::command]
#[specta::specta]
pub async fn app_open_settings(app: AppHandle) -> Result<(), ApiError> {
    let window = match app.get_webview_window(SETTINGS_WINDOW) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(
            &app,
            SETTINGS_WINDOW,
            WebviewUrl::App("index.html#/settings".into()),
        )
        .title("MyPass")
        .inner_size(640.0, 520.0)
        .visible(false)
        .build()
        .map_err(|e| super::internal(e.to_string()))?,
    };
    window.show().map_err(|e| super::internal(e.to_string()))?;
    window
        .set_focus()
        .map_err(|e| super::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::FakeWindowOps;
    use crate::testing::{open_world, world};

    #[test]
    fn app_info_reports_the_app_version() {
        let w = world();
        let facade = w.core.app_info();
        let info = info_with_version(&w.core, "9.9.9-sentinel");
        assert_eq!(info.version, "9.9.9-sentinel");
        assert_ne!(info.version, facade.version);
        assert_eq!(info.os, facade.os);
        assert_eq!(info.device_unlock_available, facade.device_unlock_available);
        // The command uses this app's own package version.
        assert_eq!(super::info(&w.core).version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn open_entry_shows_and_focuses_main_and_returns_the_event() {
        let w = open_world();
        let id = w.login("Site");
        let windows = FakeWindowOps::default();
        let event = open_entry(&w.core, &windows, &id, true).unwrap().unwrap();
        assert_eq!(
            event,
            OpenEntryEvent {
                id: id.clone(),
                edit: true
            }
        );
        let calls: Vec<_> = windows
            .calls()
            .into_iter()
            .map(|(op, label, _)| (op, label))
            .collect();
        assert_eq!(
            calls,
            vec![
                ("show".to_owned(), "main".to_owned()),
                ("focus".to_owned(), "main".to_owned())
            ]
        );
    }

    #[test]
    fn show_main_window_shows_then_focuses_main() {
        let windows = FakeWindowOps::default();
        show_main_window(&windows).unwrap();
        let calls: Vec<_> = windows
            .calls()
            .into_iter()
            .map(|(op, label, _)| (op, label))
            .collect();
        assert_eq!(
            calls,
            vec![
                ("show".to_owned(), "main".to_owned()),
                ("focus".to_owned(), "main".to_owned())
            ]
        );
    }

    /// The mock runtime cannot raise `ExitRequested` from `exit` (its `request_exit` is
    /// unimplemented), and the gate, the flow and `exit` are covered by the `shell_runtime`
    /// smoke tests. What must hold here is that the panel's Quit has no other way out.
    #[test]
    fn app_quit_only_raises_exit_requested_and_never_exits_the_process_itself() {
        let source = include_str!("app.rs");
        let body = source
            .split("pub fn request_quit")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .unwrap();
        assert!(body.contains("app.exit(0)"));
        for forbidden in ["process::exit", "exit_gate", "ExitGate", "finish("] {
            assert!(
                !body.contains(forbidden),
                "{forbidden} would bypass the quit flow"
            );
        }
    }

    /// The tray menu, the window close and the panel share `request_quit`: no second way in.
    #[test]
    fn the_tray_menu_and_window_close_quit_through_request_quit() {
        let runtime = include_str!("../shell_runtime.rs");
        assert!(!runtime.contains("app.exit(0)"));
        assert!(runtime.matches("request_quit(app)").count() >= 2);
    }

    #[test]
    fn open_entry_while_locked_touches_no_window_and_emits_nothing() {
        let w = world();
        let windows = FakeWindowOps::default();
        let event = open_entry(&w.core, &windows, "00000000000000000000000000000000", false);
        assert_eq!(event.unwrap(), None);
        assert!(windows.calls().is_empty());
    }
}
