//! `draft_*`: encrypted note drafts.

use crate::state::AppState;
use mypass_session::{ApiError, DraftVersionDto};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn draft_save(
    state: State<'_, AppState>,
    entry_id: String,
    notes: String,
) -> Result<bool, ApiError> {
    state.0.draft_save(&entry_id, &notes)
}

#[tauri::command]
#[specta::specta]
pub fn draft_versions(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<Vec<DraftVersionDto>, ApiError> {
    state.0.draft_versions(&entry_id)
}

#[tauri::command]
#[specta::specta]
pub fn draft_clear(state: State<'_, AppState>, entry_id: String) -> Result<(), ApiError> {
    state.0.draft_clear(&entry_id)
}
