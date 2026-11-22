//! `tray_*`: what the tray panel may read, and the rule that keeps it out of the recycle bin.
//!
//! The tray panel lists recents and copies fields through `entry_copy`. The clipboard path
//! restricts the tray to live entries and the three fields its menu offers.

use crate::tray::{PanelMaterial, PANEL_WINDOW};
use mypass_session::dto::FieldRef;
use mypass_session::{ApiError, AppCore, ErrorCode};

fn panel_caller(label: &str) -> Result<(), ApiError> {
    if label == PANEL_WINDOW {
        Ok(())
    } else {
        Err(ApiError {
            code: ErrorCode::InvalidInput,
            message: "this command is only available to the tray panel".into(),
        })
    }
}

#[tauri::command]
#[specta::specta]
pub fn tray_panel_fit(
    window: tauri::WebviewWindow,
    height: f64,
) -> Result<PanelMaterial, ApiError> {
    panel_caller(window.label())?;
    crate::shell_runtime::fit_panel(&window, height).map_err(|e| super::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub fn tray_panel_hide(window: tauri::WebviewWindow) -> Result<(), ApiError> {
    panel_caller(window.label())?;
    window.hide().map_err(|e| super::internal(e.to_string()))
}

/// The fields the tray panel can copy: the user name, password and one-time code.
pub fn tray_field(field: &FieldRef) -> bool {
    matches!(
        field,
        FieldRef::Username | FieldRef::Password | FieldRef::Totp
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
    use crate::testing::open_world;

    #[test]
    fn panel_commands_refuse_other_windows() {
        assert!(panel_caller(PANEL_WINDOW).is_ok());
        for label in [MAIN_WINDOW, SETTINGS_WINDOW, "unknown"] {
            assert_eq!(
                panel_caller(label).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
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
        for field in [FieldRef::Username, FieldRef::Password, FieldRef::Totp] {
            tray_access(&w.core, PANEL_WINDOW, &id, Some(&field)).unwrap();
        }
        for field in [
            FieldRef::Url,
            FieldRef::UrlAt { index: 1 },
            FieldRef::NotesBody,
            FieldRef::CardNumber,
            FieldRef::CardCvv,
            FieldRef::Address { name: "x".into() },
            FieldRef::Person {
                name: "email".into(),
            },
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
