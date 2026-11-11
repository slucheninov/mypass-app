//! `backup_*`.

use super::blocking;
use crate::state::AppState;
use mypass_session::{ApiError, BackupStatus};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn backup_list(state: State<'_, AppState>) -> Result<BackupStatus, ApiError> {
    state.0.backup_list()
}

#[tauri::command]
#[specta::specta]
pub async fn backup_restore(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    let core = state.0.clone();
    blocking(move || core.backup_restore(&id)).await
}
