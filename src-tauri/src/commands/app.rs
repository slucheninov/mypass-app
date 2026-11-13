//! `app_*`: app-level commands (version, activity, windows, reveal in the file manager).

use crate::events::OpenEntryEvent;
use crate::shell::WindowOps;
use crate::state::{AppState, EditorDirty, ShellState};
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

/// The main window reports whether its editor holds unsaved changes; the quit flow asks before
/// discarding them. Advisory: the flag only decides whether the prompt shows.
#[tauri::command]
#[specta::specta]
pub fn app_set_editor_dirty(dirty: bool, flag: State<'_, EditorDirty>) -> Result<(), ApiError> {
    flag.set(dirty);
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

    /// Every `.exit(` call in the shell, comments and test modules aside: the guard is a source
    /// scan, not a behavioural test, on purpose. Which call site reaches which quit path is wiring
    /// between the tray and window callbacks and the Tauri runtime, and the mock runtime cannot
    /// raise `ExitRequested` (its `request_exit` is unimplemented), so no plain decision function
    /// sees it. The decisions themselves (`close_action`, the tray menu ids) have table tests; this
    /// pins the wiring around them.
    fn exit_call_sites() -> Vec<(String, usize, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&root, &mut files);
        let mut found = Vec::new();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let live = text.split("#[cfg(test)]").next().unwrap_or_default();
            for (n, line) in live.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains(".exit(") {
                    let name = file
                        .strip_prefix(&root)
                        .unwrap_or(&file)
                        .to_string_lossy()
                        .replace('\\', "/");
                    found.push((name, n + 1, line.trim().to_owned()));
                }
            }
        }
        found
    }

    /// Only three places may call `.exit(`: `request_quit` (raises `ExitRequested`), the
    /// `ExitPort::exit` call of the quit worker after a confirmed decision, and the
    /// `TauriQuitPort` that implements it. Any other spelling (`handle.exit(0)`) is a way around
    /// the flush-and-confirm flow.
    #[test]
    fn nothing_but_request_quit_and_the_quit_port_calls_exit() {
        let mut sites: Vec<(String, String)> = exit_call_sites()
            .into_iter()
            .map(|(file, _, line)| (file, line))
            .collect();
        sites.sort();
        assert_eq!(
            sites,
            vec![
                ("commands/app.rs".to_owned(), "app.exit(0);".to_owned()),
                ("shell_runtime.rs".to_owned(), "port.exit();".to_owned()),
                ("shell_runtime.rs".to_owned(), "self.0.exit(0);".to_owned()),
            ]
        );
    }

    /// The tray menu's Quit and the window close's `QuitApp` arms both quit through `request_quit`.
    #[test]
    fn the_tray_quit_and_close_quit_arms_call_request_quit() {
        let runtime = include_str!("../shell_runtime.rs");
        for arm in ["TrayAction::Quit =>", "CloseAction::QuitApp =>"] {
            let line = runtime
                .lines()
                .find(|l| l.contains(arm))
                .unwrap_or_else(|| panic!("no `{arm}` arm in shell_runtime.rs"));
            assert!(
                line.contains("request_quit(app)"),
                "`{arm}` must quit through request_quit: {line}"
            );
        }
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
