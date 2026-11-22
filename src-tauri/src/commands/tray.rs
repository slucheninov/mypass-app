//! `tray_*`: what the tray panel may read, and the rule that keeps it out of the recycle bin.
//!
//! The tray panel lists recent entries and restricts reads to live entries and its compact UI's
//! fields. The caller label comes from the runtime and its capability controls each command.

use crate::tray::PANEL_WINDOW;
use mypass_session::dto::FieldRef;
use mypass_session::{ApiError, AppCore, ErrorCode};

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
    use crate::testing::open_world;

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
