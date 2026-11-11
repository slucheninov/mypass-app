//! `draft_*`: encrypted note drafts.
//!
//! `draft_versions` is a secret-returning command: it hands the editor the saved notes text of
//! each version (the editor case, like `entry_edit_model`); the UI drops it on close, save and
//! lock. All three commands touch the draft files and `draft_save` may also write the vault, so
//! they run on the blocking pool.

use super::blocking;
use crate::state::AppState;
use mypass_session::{ApiError, DraftVersionDto};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn draft_save(
    state: State<'_, AppState>,
    entry_id: String,
    notes: String,
) -> Result<bool, ApiError> {
    let core = state.0.clone();
    blocking(move || core.draft_save(&entry_id, &notes)).await
}

#[tauri::command]
#[specta::specta]
pub async fn draft_versions(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<Vec<DraftVersionDto>, ApiError> {
    let core = state.0.clone();
    blocking(move || core.draft_versions(&entry_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn draft_clear(state: State<'_, AppState>, entry_id: String) -> Result<(), ApiError> {
    let core = state.0.clone();
    blocking(move || core.draft_clear(&entry_id)).await
}
