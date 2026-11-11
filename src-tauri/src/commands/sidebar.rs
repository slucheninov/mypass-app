//! `sidebar_*`.

use crate::state::AppState;
use mypass_session::dto::Sidebar;
use mypass_session::ApiError;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn sidebar_get(state: State<'_, AppState>) -> Result<Sidebar, ApiError> {
    state.0.sidebar_get()
}
