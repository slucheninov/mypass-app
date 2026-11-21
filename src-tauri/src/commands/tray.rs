//! `tray_*`: what the tray panel may read, and the rule that keeps it out of the recycle bin.
//!
//! The tray panel is a compact, always-at-hand window: it lists the recent items and searches the
//! live ones, then shows a user name, a password, a one-time code and a website. It has no
//! business with deleted entries or with any other field, so:
//!
//! - it searches through [`tray_search`] (a fixed scope: never the recycle bin, at most
//!   [`TRAY_SEARCH_LIMIT`] results), never through the general `entry_list`, which takes any
//!   query including the Deleted category;
//! - `entry_get`, `entry_reveal`, `entry_copy` and `entry_totp` call [`tray_access`] with the
//!   label of the calling webview (set by the runtime, not by the page), which refuses an entry in
//!   the recycle bin and a field the panel never shows when that caller is the tray panel.

use crate::state::AppState;
use crate::tray::PANEL_WINDOW;
use mypass_session::dto::{
    Category, EntryQuery, EntrySummary, FieldRef, Scope, SortKey, SortOrder,
};
use mypass_session::{ApiError, AppCore, ErrorCode};
use tauri::State;

/// A tray search lists at most this many matches; the main window has the full list.
pub const TRAY_SEARCH_LIMIT: usize = 50;

/// The tray panel's search: the main window's matching (title, user name, websites and the other
/// visible fields) over every live entry, sorted by title, never the recycle bin. An empty (or
/// blank) text lists nothing.
pub fn search(core: &AppCore, text: &str) -> Result<Vec<EntrySummary>, ApiError> {
    let text = text.trim();
    if text.is_empty() {
        // Still refuse a locked session like every other read.
        core.entry_recent(0)?;
        return Ok(Vec::new());
    }
    let mut list = core.entry_list(EntryQuery {
        scope: Scope::Category {
            category: Category::All,
        },
        search: text.to_owned(),
        sort: SortKey::Title,
        order: SortOrder::Ascending,
    })?;
    list.retain(|item| !item.in_recycle_bin);
    list.truncate(TRAY_SEARCH_LIMIT);
    Ok(list)
}

#[tauri::command]
#[specta::specta]
pub fn tray_search(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<EntrySummary>, ApiError> {
    search(&state.0, &query)
}

/// The fields the tray panel shows and copies: the user name, the password, the first website and
/// the one-time code.
pub fn tray_field(field: &FieldRef) -> bool {
    matches!(
        field,
        FieldRef::Username | FieldRef::Password | FieldRef::Url | FieldRef::Totp
    )
}

/// Refuses a read of the tray panel (`label`) that it has no business with: a field it never shows
/// (`InvalidInput`) or an entry in the recycle bin (`NotFound`, as if it were gone). Any other
/// window passes unchanged; its own capability decides what it may call.
pub fn tray_access(
    core: &AppCore,
    label: &str,
    id: &str,
    field: Option<&FieldRef>,
) -> Result<(), ApiError> {
    if label != PANEL_WINDOW {
        return Ok(());
    }
    if field.is_some_and(|f| !tray_field(f)) {
        return Err(ApiError {
            code: ErrorCode::InvalidInput,
            message: "the tray panel does not read this field".into(),
        });
    }
    if core.entry_get(id)?.in_recycle_bin {
        return Err(not_found());
    }
    Ok(())
}

/// What the tray panel hears about an entry in the recycle bin.
pub fn not_found() -> ApiError {
    ApiError {
        code: ErrorCode::NotFound,
        message: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::app::{MAIN_WINDOW, SETTINGS_WINDOW};
    use crate::testing::{open_world, world};

    #[test]
    fn search_lists_live_matches_by_title_and_never_the_recycle_bin() {
        let w = open_world();
        let b = w.login("Bravo mail");
        let a = w.login("Alpha mail");
        let deleted = w.login("Charlie mail");
        w.login("Other");
        w.core.entry_delete(&deleted).unwrap();
        assert!(w.core.entry_get(&deleted).unwrap().in_recycle_bin);

        let ids: Vec<_> = search(&w.core, "  mail ")
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [a, b]);
        assert!(search(&w.core, "Charlie").unwrap().is_empty());
    }

    #[test]
    fn search_of_an_empty_text_lists_nothing() {
        let w = open_world();
        w.login("Alpha");
        assert!(search(&w.core, "").unwrap().is_empty());
        assert!(search(&w.core, "   ").unwrap().is_empty());
    }

    #[test]
    fn search_is_capped() {
        let w = open_world();
        for i in 0..(TRAY_SEARCH_LIMIT + 3) {
            w.login(&format!("Item {i:03}"));
        }
        let list = search(&w.core, "Item").unwrap();
        assert_eq!(list.len(), TRAY_SEARCH_LIMIT);
        assert_eq!(list[0].title, "Item 000");
    }

    #[test]
    fn search_refuses_a_locked_or_closed_session() {
        let w = open_world();
        w.core.vault_lock().unwrap();
        assert_eq!(search(&w.core, "x").unwrap_err().code, ErrorCode::Locked);
        assert_eq!(search(&w.core, "").unwrap_err().code, ErrorCode::Locked);
        assert!(search(&world().core, "x").is_err());
    }

    #[test]
    fn the_tray_panel_cannot_read_an_entry_in_the_recycle_bin() {
        let w = open_world();
        let live = w.login("Live");
        let deleted = w.login("Deleted");
        w.core.entry_delete(&deleted).unwrap();
        for field in [None, Some(&FieldRef::Password), Some(&FieldRef::Totp)] {
            assert_eq!(
                tray_access(&w.core, PANEL_WINDOW, &deleted, field)
                    .unwrap_err()
                    .code,
                ErrorCode::NotFound
            );
            tray_access(&w.core, PANEL_WINDOW, &live, field).unwrap();
        }
    }

    #[test]
    fn the_tray_panel_reads_only_the_fields_it_shows() {
        let w = open_world();
        let id = w.login("Live");
        for field in [
            FieldRef::Username,
            FieldRef::Password,
            FieldRef::Url,
            FieldRef::Totp,
        ] {
            tray_access(&w.core, PANEL_WINDOW, &id, Some(&field)).unwrap();
        }
        for field in [
            FieldRef::UrlAt { index: 1 },
            FieldRef::NotesBody,
            FieldRef::CardNumber,
            FieldRef::CardCvv,
            FieldRef::Address { name: "x".into() },
            FieldRef::Custom { key: "x".into() },
        ] {
            assert_eq!(
                tray_access(&w.core, PANEL_WINDOW, &id, Some(&field))
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput,
                "{field:?}"
            );
        }
    }

    #[test]
    fn other_windows_keep_their_access_to_the_recycle_bin() {
        let w = open_world();
        let deleted = w.login("Deleted");
        w.core.entry_delete(&deleted).unwrap();
        for label in [MAIN_WINDOW, SETTINGS_WINDOW] {
            tray_access(&w.core, label, &deleted, Some(&FieldRef::NotesBody)).unwrap();
        }
    }

    #[test]
    fn the_tray_panel_check_refuses_a_locked_session_and_a_missing_entry() {
        let w = open_world();
        let id = w.login("Live");
        assert_eq!(
            tray_access(
                &w.core,
                PANEL_WINDOW,
                "00000000-0000-0000-0000-000000000000",
                None
            )
            .unwrap_err()
            .code,
            ErrorCode::NotFound
        );
        w.core.vault_lock().unwrap();
        assert_eq!(
            tray_access(&w.core, PANEL_WINDOW, &id, None)
                .unwrap_err()
                .code,
            ErrorCode::Locked
        );
    }
}
