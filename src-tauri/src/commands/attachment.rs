//! `attachment_*`: adding and exporting attachments through native dialogs opened from Rust.
//!
//! The webview supplies no path: `attachment_pick` hands back tokens (see `crate::files`) and
//! `attachment_export` opens the save dialog itself.

use super::{blocking, ensure_unlocked};
use crate::files::{FileGrants, FilePicker, PickedFile};
use crate::state::{AppState, ShellState};
use mypass_session::{ApiError, AppCore};
use tauri::State;

/// The file name offered to the save dialog: the attachment's name is vault content (a hostile
/// vault may carry `..\\..\\x.bat`), so only its last component survives. Splits on both `/`
/// and `\`, drops control characters, strips leading dots and a Windows drive prefix, and falls
/// back to `attachment` when nothing is left.
pub fn sanitize_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    // A drive prefix survives the split only as `C:name` (the separator was the colon's
    // neighbour); drop `X:` at the start.
    let mut chars = cleaned.chars();
    let without_drive = match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => chars.as_str(),
        _ => cleaned.as_str(),
    };
    let trimmed = without_drive.trim_start_matches('.').trim();
    if trimmed.is_empty() {
        "attachment".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Opens the attachment dialog and grants a token per chosen file. A cancelled dialog is an
/// empty list. A locked session is refused before the dialog opens.
pub fn pick(
    core: &AppCore,
    picker: &dyn FilePicker,
    grants: &FileGrants,
) -> Result<Vec<PickedFile>, ApiError> {
    ensure_unlocked(core)?;
    Ok(grants.grant_tokens(picker.pick_attachments()))
}

/// Opens the save dialog and writes the attachment there; `false` when the user cancelled.
pub fn export(
    core: &AppCore,
    picker: &dyn FilePicker,
    entry_id: &str,
    name: &str,
) -> Result<bool, ApiError> {
    ensure_unlocked(core)?;
    let Some(dest) = picker.save_as(&sanitize_file_name(name)) else {
        return Ok(false);
    };
    core.attachment_export(entry_id, name, &dest.to_string_lossy())?;
    Ok(true)
}

#[tauri::command]
#[specta::specta]
pub async fn attachment_pick(
    state: State<'_, AppState>,
    shell: State<'_, ShellState>,
) -> Result<Vec<PickedFile>, ApiError> {
    let core = state.0.clone();
    let picker = shell.picker.clone();
    let grants = shell.grants.clone();
    blocking(move || pick(&core, picker.as_ref(), &grants)).await
}

#[tauri::command]
#[specta::specta]
pub async fn attachment_export(
    state: State<'_, AppState>,
    shell: State<'_, ShellState>,
    entry_id: String,
    name: String,
) -> Result<bool, ApiError> {
    let core = state.0.clone();
    let picker = shell.picker.clone();
    blocking(move || export(&core, picker.as_ref(), &entry_id, &name)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::FakeFilePicker;
    use crate::testing::{open_world, world};
    use mypass_session::dto::{AttachmentOp, EntryTypeDto};
    use mypass_session::editing::new_model;
    use mypass_session::ErrorCode;
    use uuid::Uuid;

    #[test]
    fn sanitize_file_name_table() {
        for (input, expected) in [
            ("..\\..\\x.bat", "x.bat"),
            ("/etc/passwd", "passwd"),
            ("C:\\a\\b.txt", "b.txt"),
            ("C:b.txt", "b.txt"),
            ("..", "attachment"),
            ("", "attachment"),
            ("dir/", "attachment"),
            (".hidden", "hidden"),
            ("a\u{0}b\u{7}.txt", "ab.txt"),
            ("plain.txt", "plain.txt"),
            (
                "caf\u{e9} \u{43f}\u{440}.pdf",
                "caf\u{e9} \u{43f}\u{440}.pdf",
            ),
        ] {
            assert_eq!(sanitize_file_name(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn the_save_dialog_gets_the_sanitised_name() {
        let w = open_world();
        let id = entry_with_attachment(&w);
        let picker = FakeFilePicker::default();
        // The attachment is called in.txt; a hostile caller asks for another name: the dialog
        // still only sees the safe last component (the facade then refuses the unknown name).
        let _ = export(&w.core, &picker, &id, "..\\..\\evil.bat");
        assert_eq!(*picker.save_names.lock().unwrap(), vec!["evil.bat"]);
    }

    #[test]
    fn cancelled_pick_returns_nothing_and_grants_nothing() {
        let w = open_world();
        let picker = FakeFilePicker::default();
        let grants = FileGrants::default();
        assert!(pick(&w.core, &picker, &grants).unwrap().is_empty());
    }

    #[test]
    fn pick_while_locked_opens_no_dialog() {
        let w = world();
        let picker = FakeFilePicker::default();
        let err = pick(&w.core, &picker, &FileGrants::default()).unwrap_err();
        assert!(matches!(err.code, ErrorCode::NoVault | ErrorCode::Locked));
        assert_eq!(picker.dialog_count(), 0);
    }

    #[test]
    fn pick_returns_token_name_and_size_without_the_path() {
        let w = open_world();
        let picker = FakeFilePicker::default();
        picker
            .attachments
            .lock()
            .unwrap()
            .push(w.file("report.pdf", b"12345"));
        let grants = FileGrants::default();
        let picked = pick(&w.core, &picker, &grants).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].name, "report.pdf");
        assert!((picked[0].size - 5.0).abs() < f64::EPSILON);
        assert_eq!(picked[0].token.len(), 32);
        assert!(!picked[0].token.contains("report"));
    }

    fn entry_with_attachment(w: &crate::testing::World) -> String {
        let mut m = new_model(EntryTypeDto::Login, Uuid::parse_str(&w.root()).unwrap());
        m.title = "E".into();
        let path = w.file("in.txt", b"payload");
        m.attachment_ops = vec![AttachmentOp::Add {
            path: path.to_string_lossy().into_owned(),
        }];
        w.core.entry_save(m).unwrap()
    }

    #[test]
    fn cancelled_export_writes_nothing_and_returns_false() {
        let w = open_world();
        let id = entry_with_attachment(&w);
        let picker = FakeFilePicker::default();
        assert!(!export(&w.core, &picker, &id, "in.txt").unwrap());
        assert_eq!(*picker.save_names.lock().unwrap(), vec!["in.txt"]);
    }

    #[test]
    fn export_writes_to_the_chosen_destination() {
        let w = open_world();
        let id = entry_with_attachment(&w);
        let dest = w.dir.path().join("out.txt");
        let picker = FakeFilePicker::default();
        *picker.save.lock().unwrap() = Some(dest.clone());
        assert!(export(&w.core, &picker, &id, "in.txt").unwrap());
        assert_eq!(std::fs::read(dest).unwrap(), b"payload");
    }

    #[test]
    fn export_while_locked_opens_no_dialog() {
        let w = world();
        let picker = FakeFilePicker::default();
        let err = export(&w.core, &picker, "x", "n").unwrap_err();
        assert!(matches!(err.code, ErrorCode::NoVault | ErrorCode::Locked));
        assert!(picker.save_names.lock().unwrap().is_empty());
    }
}
