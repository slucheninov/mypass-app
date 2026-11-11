//! `group_*`.

use crate::state::AppState;
use mypass_session::ApiError;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn group_create(
    state: State<'_, AppState>,
    parent_id: Option<String>,
    name: String,
    emoji: Option<String>,
) -> Result<String, ApiError> {
    state
        .0
        .group_create(parent_id.as_deref(), &name, emoji.as_deref())
}

#[tauri::command]
#[specta::specta]
pub fn group_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    emoji: Option<String>,
) -> Result<(), ApiError> {
    state.0.group_update(&id, &name, emoji.as_deref())
}

#[tauri::command]
#[specta::specta]
pub fn group_move(
    state: State<'_, AppState>,
    id: String,
    parent_id: String,
) -> Result<(), ApiError> {
    state.0.group_move(&id, &parent_id)
}

#[tauri::command]
#[specta::specta]
pub fn group_delete(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    state.0.group_delete(&id)
}
