//! `vault_*`: open, create, unlock and lock the vault, and find vault files.

use super::{blocking, into_secret};
use crate::state::AppState;
use mypass_session::dto::{FoundVaultFileDto, OpenWarning, RecentVaultDto};
use mypass_session::{ApiError, SessionStatus};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
#[specta::specta]
pub fn vault_status(state: State<'_, AppState>) -> Result<SessionStatus, ApiError> {
    state.0.vault_status()
}

#[tauri::command]
#[specta::specta]
pub async fn vault_create(
    state: State<'_, AppState>,
    path: String,
    mut password: String,
    name: String,
) -> Result<SessionStatus, ApiError> {
    let password = into_secret(&mut password);
    let core = state.0.clone();
    blocking(move || core.vault_create(&path, password, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_open(
    state: State<'_, AppState>,
    path: String,
    mut password: String,
) -> Result<SessionStatus, ApiError> {
    let password = into_secret(&mut password);
    let core = state.0.clone();
    blocking(move || core.vault_open(&path, password)).await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_unlock(
    state: State<'_, AppState>,
    mut password: String,
) -> Result<SessionStatus, ApiError> {
    let password = into_secret(&mut password);
    let core = state.0.clone();
    blocking(move || core.vault_unlock(password)).await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_lock(state: State<'_, AppState>) -> Result<(), ApiError> {
    // Locking saves first (key derivation and a write), so never on the main thread.
    let core = state.0.clone();
    blocking(move || core.vault_lock()).await
}

/// The folders searched for `.kdbx` files: home, Documents, Desktop and Downloads (each once).
pub fn search_roots(app: &AppHandle) -> Vec<PathBuf> {
    let path = app.path();
    let mut roots: Vec<PathBuf> = Vec::new();
    for root in [
        path.home_dir(),
        path.document_dir(),
        path.desktop_dir(),
        path.download_dir(),
    ]
    .into_iter()
    .flatten()
    {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

#[tauri::command]
#[specta::specta]
pub async fn vault_find_files(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<FoundVaultFileDto>, ApiError> {
    let roots = search_roots(&app);
    let core = state.0.clone();
    blocking(move || core.vault_find_files(roots)).await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_recent(state: State<'_, AppState>) -> Result<Vec<RecentVaultDto>, ApiError> {
    let core = state.0.clone();
    blocking(move || core.vault_recent()).await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_forget(state: State<'_, AppState>, path: String) -> Result<(), ApiError> {
    let core = state.0.clone();
    blocking(move || core.vault_forget(&path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_open_warnings(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<OpenWarning>, ApiError> {
    // Enumerates processes and stats paths.
    let core = state.0.clone();
    blocking(move || core.vault_open_warnings(&path)).await
}
