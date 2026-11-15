//! `sync_*`: the secondary-folder sync. `sync_connect_secondary` arrives with the migration
//! (it takes a folder token from a Rust dialog, never a path from the webview).

use super::blocking;
use crate::state::AppState;
use mypass_session::{ApiError, SecondaryStatus};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn sync_status(state: State<'_, AppState>) -> Result<SecondaryStatus, ApiError> {
    state.0.sync_status()
}

#[tauri::command]
#[specta::specta]
pub async fn sync_now(state: State<'_, AppState>) -> Result<SecondaryStatus, ApiError> {
    let core = state.0.clone();
    blocking(move || core.sync_now()).await
}

#[tauri::command]
#[specta::specta]
pub async fn sync_disconnect_secondary(state: State<'_, AppState>) -> Result<(), ApiError> {
    let core = state.0.clone();
    blocking(move || core.sync_disconnect_secondary()).await
}

#[tauri::command]
#[specta::specta]
pub async fn sync_connect_secondary(
    state: State<'_, AppState>,
    shell: State<'_, crate::state::ShellState>,
    folder_token: String,
    strategy: mypass_session::migration::MigrationStrategy,
) -> Result<SecondaryStatus, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let folder = shell.grants.take_folder(&folder_token)?;
    let core = state.0.clone();
    blocking(move || core.sync_connect_secondary(&folder, strategy)).await
}
