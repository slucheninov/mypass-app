//! `sync_*`: the secondary-folder sync. `sync_connect_secondary` arrives with the migration
//! (it takes a folder token from a Rust dialog, never a path from the webview).

use super::blocking;
use crate::state::AppState;
use mypass_session::{ApiError, DisplacedCopies, SecondaryStatus};
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn sync_status(state: State<'_, AppState>) -> Result<SecondaryStatus, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let core = state.0.clone();
    blocking(move || core.sync_status()).await
}

#[tauri::command]
#[specta::specta]
pub async fn sync_now(state: State<'_, AppState>) -> Result<SecondaryStatus, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let core = state.0.clone();
    blocking(move || core.sync_now()).await
}

#[tauri::command]
#[specta::specta]
pub async fn sync_disconnect_secondary(state: State<'_, AppState>) -> Result<(), ApiError> {
    super::ensure_unlocked(&state.0)?;
    let core = state.0.clone();
    blocking(move || core.sync_disconnect_secondary()).await
}

/// The kept vault copies, for display. Listing deletes nothing; while the vault is unlocked
/// each copy that may be deleted carries an opaque id for `sync_displaced_delete`.
#[tauri::command]
#[specta::specta]
pub async fn sync_displaced_status(
    state: State<'_, AppState>,
) -> Result<DisplacedCopies, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let core = state.0.clone();
    blocking(move || core.sync_displaced_status()).await
}

/// Deletes the one retained vault copy the user confirmed, by the id of the current listing,
/// after re-verifying it; returns the refreshed list. Refused while locked.
#[tauri::command]
#[specta::specta]
pub async fn sync_displaced_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<DisplacedCopies, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let core = state.0.clone();
    blocking(move || core.sync_displaced_delete(&id)).await
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
    let incarnation = state.0.session.unlocked_incarnation()?;
    let folder = shell.grants.take_folder_for(&folder_token, incarnation)?;
    let core = state.0.clone();
    blocking(move || {
        core.migrate_folder_for(&folder, strategy, false, incarnation)?;
        core.sync_status()
    })
    .await
}
