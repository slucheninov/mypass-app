//! `generator_*`: the password generator and its history.
//!
//! `generator_generate` returns the new password and `generator_history_reveal` returns one
//! history value after an explicit click; those are the only secrets this file hands out.

use crate::state::AppState;
use mypass_session::dto::{GenerateRequest, GeneratedPassword, GeneratorHistoryItem, RulesSummary};
use mypass_session::ApiError;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn generator_generate(
    state: State<'_, AppState>,
    req: GenerateRequest,
) -> Result<GeneratedPassword, ApiError> {
    state.0.generator_generate(req)
}

#[tauri::command]
#[specta::specta]
pub fn generator_rules_summary(
    state: State<'_, AppState>,
    host: String,
) -> Result<Option<RulesSummary>, ApiError> {
    state.0.generator_rules_summary(&host)
}

#[tauri::command]
#[specta::specta]
pub fn generator_history_list(
    state: State<'_, AppState>,
) -> Result<Vec<GeneratorHistoryItem>, ApiError> {
    state.0.generator_history_list()
}

#[tauri::command]
#[specta::specta]
pub fn generator_copy(
    state: State<'_, AppState>,
    value: String,
    record_url: Option<String>,
) -> Result<(), ApiError> {
    state.0.generator_copy(&value, record_url.as_deref())
}

#[tauri::command]
#[specta::specta]
pub fn generator_history_copy(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    state.0.generator_history_copy(&id)
}

#[tauri::command]
#[specta::specta]
pub fn generator_history_reveal(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, ApiError> {
    state.0.generator_history_reveal(&id)
}

#[tauri::command]
#[specta::specta]
pub fn generator_history_delete(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    state.0.generator_history_delete(&id)
}
