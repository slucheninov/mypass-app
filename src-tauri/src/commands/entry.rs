//! `entry_*`: list, read, copy, edit and delete entries.
//!
//! Secrets leave Rust only through the commands that say so: `entry_reveal` (one value after an
//! explicit click), `entry_edit_model` (the editor, dropped by the UI on close, save and lock)
//! and `entry_totp` (a display code).

use super::{blocking, ensure_unlocked};
use crate::files::FileGrants;
use crate::state::{AppState, ShellState};
use mypass_session::dto::{
    EntryDetailDto, EntryEditModel, EntryQuery, EntrySummary, EntryTypeDto, FieldRef, HistoryItem,
    TotpDto,
};
use mypass_session::ApiError;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn entry_list(
    state: State<'_, AppState>,
    query: EntryQuery,
) -> Result<Vec<EntrySummary>, ApiError> {
    state.0.entry_list(query)
}

#[tauri::command]
#[specta::specta]
pub fn entry_recent(state: State<'_, AppState>, limit: u32) -> Result<Vec<EntrySummary>, ApiError> {
    state.0.entry_recent(limit)
}

#[tauri::command]
#[specta::specta]
pub fn entry_get(state: State<'_, AppState>, id: String) -> Result<EntryDetailDto, ApiError> {
    state.0.entry_get(&id)
}

#[tauri::command]
#[specta::specta]
pub fn entry_totp(state: State<'_, AppState>, id: String) -> Result<TotpDto, ApiError> {
    state.0.entry_totp(&id)
}

/// Reveal command: returns one secret value after the user's explicit click.
#[tauri::command]
#[specta::specta]
pub fn entry_reveal(
    state: State<'_, AppState>,
    id: String,
    field: FieldRef,
) -> Result<String, ApiError> {
    state.0.entry_reveal(&id, field)
}

#[tauri::command]
#[specta::specta]
pub fn entry_copy(state: State<'_, AppState>, id: String, field: FieldRef) -> Result<(), ApiError> {
    state.0.entry_copy(&id, field)
}

#[tauri::command]
#[specta::specta]
pub fn entry_new_model(
    state: State<'_, AppState>,
    entry_type: EntryTypeDto,
    group_id: Option<String>,
) -> Result<EntryEditModel, ApiError> {
    state.0.entry_new_model(entry_type, group_id.as_deref())
}

/// Reveal command: the editor model carries the secrets of the entry being edited.
#[tauri::command]
#[specta::specta]
pub fn entry_edit_model(
    state: State<'_, AppState>,
    id: String,
) -> Result<EntryEditModel, ApiError> {
    state.0.entry_edit_model(&id)
}

#[tauri::command]
#[specta::specta]
pub fn entry_history(state: State<'_, AppState>, id: String) -> Result<Vec<HistoryItem>, ApiError> {
    state.0.entry_history(&id)
}

/// Saves the model. An `AttachmentOp::Add` carries the token `attachment_pick` returned in its
/// `path`; it is swapped for the real path here.
///
/// No webview string ever reaches the facade as a path: a locked session is refused here with
/// `Locked`, and every `Add` is redeemed afterwards whatever the state is by then (an unlock
/// between the check and the call cannot let a raw path through; an unknown token is
/// `InvalidInput`).
pub fn save(
    core: &mypass_session::AppCore,
    grants: &FileGrants,
    mut model: EntryEditModel,
) -> Result<String, ApiError> {
    ensure_unlocked(core)?;
    let taken = grants.redeem(&mut model.attachment_ops)?;
    let saved = core.entry_save(model);
    if saved.is_err() {
        // The editor still holds the tokens; a failed save must not spend them.
        grants.restore(taken);
    }
    saved
}

#[tauri::command]
#[specta::specta]
pub async fn entry_save(
    state: State<'_, AppState>,
    shell: State<'_, ShellState>,
    model: EntryEditModel,
) -> Result<String, ApiError> {
    let core = state.0.clone();
    let grants = shell.grants.clone();
    blocking(move || save(&core, &grants, model)).await
}

#[tauri::command]
#[specta::specta]
pub async fn entry_delete(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    let core = state.0.clone();
    blocking(move || core.entry_delete(&id)).await
}

#[tauri::command]
#[specta::specta]
pub fn entry_restore(
    state: State<'_, AppState>,
    id: String,
    group_id: Option<String>,
) -> Result<(), ApiError> {
    state.0.entry_restore(&id, group_id.as_deref())
}

#[tauri::command]
#[specta::specta]
pub async fn entry_purge(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    let core = state.0.clone();
    blocking(move || core.entry_purge(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn entry_empty_bin(state: State<'_, AppState>) -> Result<u32, ApiError> {
    let core = state.0.clone();
    blocking(move || core.entry_empty_bin()).await
}

#[tauri::command]
#[specta::specta]
pub fn entry_set_favorite(
    state: State<'_, AppState>,
    id: String,
    favorite: bool,
) -> Result<(), ApiError> {
    state.0.entry_set_favorite(&id, favorite)
}

#[tauri::command]
#[specta::specta]
pub fn entry_move(
    state: State<'_, AppState>,
    id: String,
    group_id: String,
) -> Result<(), ApiError> {
    state.0.entry_move(&id, &group_id)
}

#[tauri::command]
#[specta::specta]
pub fn entry_touch(state: State<'_, AppState>, id: String) -> Result<(), ApiError> {
    state.0.entry_touch(&id)
}

#[tauri::command]
#[specta::specta]
pub fn entry_history_restore(
    state: State<'_, AppState>,
    id: String,
    index: u32,
) -> Result<(), ApiError> {
    state.0.entry_history_restore(&id, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{open_world, world};
    use mypass_session::dto::AttachmentOp;
    use mypass_session::editing::new_model;
    use mypass_session::ErrorCode;
    use uuid::Uuid;

    fn model_with(w: &crate::testing::World, ops: Vec<AttachmentOp>) -> EntryEditModel {
        let mut m = new_model(EntryTypeDto::Login, Uuid::parse_str(&w.root()).unwrap());
        m.title = "With file".into();
        m.attachment_ops = ops;
        m
    }

    fn add(token: &str) -> AttachmentOp {
        AttachmentOp::Add {
            path: token.to_owned(),
        }
    }

    fn grant(grants: &FileGrants, path: std::path::PathBuf) -> String {
        grants.grant_tokens(vec![path]).remove(0).token
    }

    #[test]
    fn token_is_swapped_for_the_real_path_and_the_attachment_lands() {
        let w = open_world();
        let grants = FileGrants::default();
        let file = w.file("note.txt", b"hello");
        let token = grant(&grants, file);
        let id = save(&w.core, &grants, model_with(&w, vec![add(&token)])).unwrap();
        let detail = w.core.entry_get(&id).unwrap();
        assert_eq!(detail.attachments.len(), 1);
        assert_eq!(detail.attachments[0].name, "note.txt");
    }

    #[test]
    fn unknown_token_is_rejected_and_a_real_path_is_not_a_token() {
        let w = open_world();
        let grants = FileGrants::default();
        let file = w.file("secret.txt", b"x");
        for bad in ["not-a-token", file.to_str().unwrap()] {
            let err = save(&w.core, &grants, model_with(&w, vec![add(bad)])).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput);
        }
        assert!(w.core.entry_list(root_query(&w)).unwrap().is_empty());
    }

    #[test]
    fn token_is_single_use() {
        let w = open_world();
        let grants = FileGrants::default();
        let token = grant(&grants, w.file("a.txt", b"a"));
        save(&w.core, &grants, model_with(&w, vec![add(&token)])).unwrap();
        let err = save(&w.core, &grants, model_with(&w, vec![add(&token)])).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn a_token_repeated_in_one_save_is_rejected_and_nothing_is_consumed() {
        let w = open_world();
        let grants = FileGrants::default();
        let token = grant(&grants, w.file("a.txt", b"a"));
        let err = save(
            &w.core,
            &grants,
            model_with(&w, vec![add(&token), add(&token)]),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        // Still redeemable once.
        save(&w.core, &grants, model_with(&w, vec![add(&token)])).unwrap();
    }

    #[test]
    fn a_failed_save_does_not_spend_the_token() {
        let w = open_world();
        let grants = FileGrants::default();
        let token = grant(&grants, w.file("a.txt", b"a"));
        // A group that does not exist makes the facade refuse the save.
        let mut bad = model_with(&w, vec![add(&token)]);
        bad.group_id = Uuid::new_v4().simple().to_string();
        assert!(save(&w.core, &grants, bad).is_err());
        save(&w.core, &grants, model_with(&w, vec![add(&token)])).unwrap();
    }

    #[test]
    fn locked_vault_answers_locked_even_for_an_unknown_token() {
        let w = world();
        let grants = FileGrants::default();
        let m = new_model(EntryTypeDto::Login, Uuid::new_v4());
        let mut m = m;
        m.attachment_ops = vec![add("nope")];
        let err = save(&w.core, &grants, m).unwrap_err();
        assert!(matches!(err.code, ErrorCode::NoVault | ErrorCode::Locked));
    }

    #[test]
    fn locked_session_with_a_real_path_never_reaches_the_facade_with_it() {
        let w = open_world();
        let grants = FileGrants::default();
        let secret_file = w.file("private.txt", b"do not read me");
        let model = model_with(&w, vec![add(secret_file.to_str().unwrap())]);
        w.core.vault_lock().unwrap();
        let err = save(&w.core, &grants, model.clone()).unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        // Once unlocked the same raw path is still no token: nothing was attached or saved.
        w.core.vault_unlock(crate::testing::pw("pw")).unwrap();
        let err = save(&w.core, &grants, model).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(w.core.entry_list(root_query(&w)).unwrap().is_empty());
    }

    fn root_query(w: &crate::testing::World) -> EntryQuery {
        EntryQuery {
            scope: mypass_session::dto::Scope::Group { group_id: w.root() },
            search: String::new(),
            sort: mypass_session::dto::SortKey::Title,
        }
    }
}
