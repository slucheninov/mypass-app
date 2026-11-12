//! `attachment_*`: adding and exporting attachments through native dialogs opened from Rust.
//!
//! The webview supplies no path: `attachment_pick` hands back tokens (see `crate::files`) and
//! `attachment_export` opens the save dialog itself.

use super::{blocking, ensure_unlocked};
use crate::files::{FileGrants, FilePicker, PickedFile};
use crate::state::{AppState, ShellState};
use mypass_session::{ApiError, AppCore, ErrorCode};
use tauri::State;

/// Windows device names that must not be a file's stem.
const RESERVED_STEMS: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Unicode format characters that can disguise a name (bidi marks and overrides, isolates).
fn is_bidi_format(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The file name offered to the save dialog: the attachment's name is vault content (a hostile
/// vault may carry `..\\..\\x.bat`), so only a safe last component survives. It splits on both
/// `/` and `\`, drops control and bidi format characters, strips a Windows drive prefix, cuts at
/// the first `:` (an NTFS stream name), trims dots and whitespace at both ends (Windows drops
/// trailing ones, and a leading dot would hide the file), prefixes `_` to a reserved device name
/// such as `CON` or `LPT1`, and falls back to `attachment` when nothing is left.
pub fn sanitize_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_format(*c))
        .collect();
    let mut chars = cleaned.chars();
    let without_drive = match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => chars.as_str(),
        _ => cleaned.as_str(),
    };
    let before_stream = without_drive.split(':').next().unwrap_or_default();
    let trimmed = before_stream.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return "attachment".to_owned();
    }
    let stem = trimmed.split('.').next().unwrap_or_default().trim_end();
    if RESERVED_STEMS.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        format!("_{trimmed}")
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
    // The attachment must exist before the user is asked where to put it.
    let detail = core.entry_get(entry_id)?;
    if !detail.attachments.iter().any(|a| a.name == name) {
        return Err(ApiError {
            code: ErrorCode::NotFound,
            message: "no such attachment".to_owned(),
        });
    }
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
            (". .", "attachment"),
            (" .hidden", "hidden"),
            ("report.txt. . ", "report.txt"),
            ("a.txt:stream:$DATA", "a.txt"),
            ("\u{202E}gpj.exe", "gpj.exe"),
            ("a\u{200F}b\u{2066}c\u{2069}.txt", "abc.txt"),
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("Lpt1.tar.gz", "_Lpt1.tar.gz"),
            ("COM9", "_COM9"),
            ("COM0", "COM0"),
            ("console.txt", "console.txt"),
            ("dir\\NUL", "_NUL"),
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
        // A bidi override is a legal file name character on Unix and on Windows, so the
        // fixture file can be created on both (a backslash is a path separator on Windows).
        let id = entry_with_attachment_named(&w, "\u{202E}evil.bat");
        let picker = FakeFilePicker::default();
        assert!(!export(&w.core, &picker, &id, "\u{202E}evil.bat").unwrap());
        assert_eq!(*picker.save_names.lock().unwrap(), vec!["evil.bat"]);
    }

    #[test]
    fn export_of_an_unknown_attachment_opens_no_dialog() {
        let w = open_world();
        let id = entry_with_attachment(&w);
        let picker = FakeFilePicker::default();
        let err = export(&w.core, &picker, &id, "nope.txt").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(picker.dialog_count(), 0);
        let err = export(
            &w.core,
            &picker,
            &Uuid::new_v4().simple().to_string(),
            "in.txt",
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(picker.dialog_count(), 0);
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
        entry_with_attachment_named(w, "in.txt")
    }

    fn entry_with_attachment_named(w: &crate::testing::World, name: &str) -> String {
        let mut m = new_model(EntryTypeDto::Login, Uuid::parse_str(&w.root()).unwrap());
        m.title = "E".into();
        // The facade names the attachment after the file (a backslash is a legal file name
        // character on Unix, so a hostile name can be made here).
        let path = w.file(name, b"payload");
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
