//! `platform_*`.

use crate::state::AppState;
use mypass_session::ApiError;
use tauri::State;

/// The current keyboard layout's identifier, when the platform reports one.
#[tauri::command]
#[specta::specta]
pub fn platform_keyboard_layout(state: State<'_, AppState>) -> Result<Option<String>, ApiError> {
    state.0.platform_keyboard_layout()
}
