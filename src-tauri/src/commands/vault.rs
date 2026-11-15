//! `vault_*`: open, create, unlock and lock the vault, and find vault files.

use super::{blocking, into_secret};
use crate::state::AppState;
use mypass_session::dto::{FoundVaultFileDto, OpenWarning, RecentVaultDto};
use mypass_session::{ApiError, SessionStatus, StorageStatus};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
#[specta::specta]
pub fn vault_status(state: State<'_, AppState>) -> Result<SessionStatus, ApiError> {
    state.0.vault_status()
}

/// Where the open vault is stored (local or iCloud Drive) and whether it is served from the
/// offline copy or still downloading.
#[tauri::command]
#[specta::specta]
pub fn vault_storage_status(state: State<'_, AppState>) -> Result<StorageStatus, ApiError> {
    state.0.vault_storage_status()
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

/// Probe consumes a grant and returns a fresh grant for the user's strategy decision.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderProbe {
    pub probe: mypass_session::migration::TargetProbe,
    pub folder_token: String,
}

#[tauri::command]
#[specta::specta]
pub async fn vault_location_pick(
    state: State<'_, AppState>,
    shell: State<'_, crate::state::ShellState>,
) -> Result<Option<String>, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let start = state.0.vault_location_start();
    let picker = shell.picker.clone();
    let grants = shell.grants.clone();
    blocking(move || {
        Ok(picker
            .pick_folder(Some(start))
            .map(|p| grants.grant_folder(p)))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_location_probe(
    state: State<'_, AppState>,
    shell: State<'_, crate::state::ShellState>,
    folder_token: String,
) -> Result<FolderProbe, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let folder = shell.grants.take_folder(&folder_token)?;
    let core = state.0.clone();
    let grants = shell.grants.clone();
    blocking(move || {
        let probe = core.vault_location_probe(&folder)?;
        Ok(FolderProbe {
            probe,
            folder_token: grants.grant_folder(folder),
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn vault_location_change(
    state: State<'_, AppState>,
    shell: State<'_, crate::state::ShellState>,
    req: mypass_session::migration::LocationChangeRequest,
) -> Result<SessionStatus, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let folder = shell.grants.take_folder(&req.folder_token)?;
    let core = state.0.clone();
    blocking(move || core.vault_location_change(&folder, req.strategy)).await
}

/// The webview's reason key is advisory only; Rust chooses the catalog reason.
#[tauri::command]
#[specta::specta]
pub async fn vault_unlock_device(
    state: State<'_, AppState>,
    reason_key: String,
) -> Result<SessionStatus, ApiError> {
    let _ = reason_key;
    let core = state.0.clone();
    blocking(move || {
        let reason = crate::i18n::t(core.settings.get().language, "Unlock MyPass", &[]);
        core.device_unlock.unlock_with_device(&reason)?;
        core.vault_status()
    })
    .await
}
