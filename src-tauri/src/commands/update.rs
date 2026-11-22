//! Update URLs are selected and validated in Rust, never supplied by the webview.
use super::blocking;
use crate::state::AppState;
use mypass_session::{update::UpdateStatus, ApiError};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
#[tauri::command]
#[specta::specta]
pub fn update_status(state: State<'_, AppState>) -> UpdateStatus {
    state.0.update.status()
}
#[tauri::command]
#[specta::specta]
pub async fn update_check_now(state: State<'_, AppState>) -> Result<UpdateStatus, ApiError> {
    super::ensure_unlocked(&state.0)?;
    let service = state.0.update.clone();
    blocking(move || Ok(service.check(true))).await
}
#[tauri::command]
#[specta::specta]
pub async fn update_dismiss(state: State<'_, AppState>) -> Result<(), ApiError> {
    let service = state.0.update.clone();
    blocking(move || service.dismiss()).await
}
pub fn open_release(app: &AppHandle, core: &mypass_session::AppCore) -> Result<(), ApiError> {
    let url = core
        .update
        .release_url()
        .ok_or_else(|| super::internal("no validated release"))?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| super::internal("could not open release"))
}
#[tauri::command]
#[specta::specta]
pub fn update_open_release(app: AppHandle, state: State<'_, AppState>) -> Result<(), ApiError> {
    open_release(&app, &state.0)
}
