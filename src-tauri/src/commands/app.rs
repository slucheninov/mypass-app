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
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
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
        let info = info(&w.core);
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_ne!(
            w.core.app_info().version,
            "",
            "the facade still reports its own"
        );
        assert_eq!(info.os, w.core.app_info().os);
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
    fn open_entry_while_locked_touches_no_window_and_emits_nothing() {
        let w = world();
        let windows = FakeWindowOps::default();
        let event = open_entry(&w.core, &windows, "00000000000000000000000000000000", false);
        assert_eq!(event.unwrap(), None);
        assert!(windows.calls().is_empty());
    }
}
