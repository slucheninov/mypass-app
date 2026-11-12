//! `import_*`: CSV import. The file is chosen in a native dialog opened from Rust and remembered
//! in app state; the webview never names a path. A locked session is refused before any dialog
//! opens.

use super::{blocking, ensure_unlocked};
use crate::files::{FileGrants, FilePicker};
use crate::state::{AppState, ShellState};
use mypass_session::dto::{CsvPreview, ImportResult};
use mypass_session::{ApiError, AppCore, ErrorCode};
use tauri::State;

/// Opens the CSV dialog and previews the chosen file; `None` when cancelled. The path is
/// remembered for [`commit`] only when the preview succeeded; a failed preview forgets any
/// earlier file.
pub fn preview(
    core: &AppCore,
    picker: &dyn FilePicker,
    grants: &FileGrants,
) -> Result<Option<CsvPreview>, ApiError> {
    ensure_unlocked(core)?;
    let Some(path) = picker.pick_csv() else {
        return Ok(None);
    };
    grants.set_csv(None);
    let preview = core.import_csv_preview(&path.to_string_lossy())?;
    grants.set_csv(Some(path));
    Ok(Some(preview))
}

/// Imports the remembered file into `group_id`. The remembered path is used exactly once, also
/// when the import fails; with none remembered the call is `InvalidInput`.
pub fn commit(
    core: &AppCore,
    grants: &FileGrants,
    group_id: &str,
) -> Result<ImportResult, ApiError> {
    ensure_unlocked(core)?;
    let Some(path) = grants.take_csv() else {
        return Err(ApiError {
            code: ErrorCode::InvalidInput,
            message: "no CSV file was previewed".to_owned(),
        });
    };
    core.import_csv_commit(&path.to_string_lossy(), group_id)
}

#[tauri::command]
#[specta::specta]
pub async fn import_csv_preview(
    state: State<'_, AppState>,
    shell: State<'_, ShellState>,
) -> Result<Option<CsvPreview>, ApiError> {
    let core = state.0.clone();
    let picker = shell.picker.clone();
    let grants = shell.grants.clone();
    blocking(move || preview(&core, picker.as_ref(), &grants)).await
}

#[tauri::command]
#[specta::specta]
pub async fn import_csv_commit(
    state: State<'_, AppState>,
    shell: State<'_, ShellState>,
    group_id: String,
) -> Result<ImportResult, ApiError> {
    let core = state.0.clone();
    let grants = shell.grants.clone();
    blocking(move || commit(&core, &grants, &group_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::FakeFilePicker;
    use crate::testing::{open_world, world};

    const CSV: &[u8] = b"Title,Username,Password,URL\nSite,me,pw1,https://example.com\n";

    fn picker_for(path: std::path::PathBuf) -> FakeFilePicker {
        let picker = FakeFilePicker::default();
        *picker.csv.lock().unwrap() = Some(path);
        picker
    }

    #[test]
    fn cancelled_preview_is_none_and_commit_then_fails() {
        let w = open_world();
        let grants = FileGrants::default();
        let picker = FakeFilePicker::default();
        assert!(preview(&w.core, &picker, &grants).unwrap().is_none());
        let err = commit(&w.core, &grants, &w.root()).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn preview_while_locked_opens_no_dialog() {
        let w = world();
        let picker = FakeFilePicker::default();
        let err = preview(&w.core, &picker, &FileGrants::default()).unwrap_err();
        assert!(matches!(err.code, ErrorCode::NoVault | ErrorCode::Locked));
        assert_eq!(picker.dialog_count(), 0);
    }

    #[test]
    fn commit_without_preview_is_invalid_input() {
        let w = open_world();
        let err = commit(&w.core, &FileGrants::default(), &w.root()).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn preview_then_commit_imports_once() {
        let w = open_world();
        let grants = FileGrants::default();
        let picker = picker_for(w.file("in.csv", CSV));
        let preview = preview(&w.core, &picker, &grants).unwrap().unwrap();
        assert_eq!(preview.total, 1);
        let result = commit(&w.core, &grants, &w.root()).unwrap();
        assert_eq!(result.imported, 1);
        // The remembered path is spent.
        let err = commit(&w.core, &grants, &w.root()).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn a_new_cancelled_preview_keeps_the_earlier_remembered_file() {
        let w = open_world();
        let grants = FileGrants::default();
        let picker = picker_for(w.file("in.csv", CSV));
        preview(&w.core, &picker, &grants).unwrap().unwrap();
        *picker.csv.lock().unwrap() = None;
        assert!(preview(&w.core, &picker, &grants).unwrap().is_none());
        assert!(commit(&w.core, &grants, &w.root()).is_ok());
    }

    #[test]
    fn a_failed_preview_forgets_the_earlier_file() {
        let w = open_world();
        let grants = FileGrants::default();
        let picker = picker_for(w.file("in.csv", CSV));
        preview(&w.core, &picker, &grants).unwrap().unwrap();
        *picker.csv.lock().unwrap() = Some(w.dir.path().join("missing.csv"));
        assert!(preview(&w.core, &picker, &grants).is_err());
        let err = commit(&w.core, &grants, &w.root()).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn commit_while_locked_is_locked_and_keeps_the_file() {
        let w = world();
        let grants = FileGrants::default();
        grants.set_csv(Some(w.file("in.csv", CSV)));
        let err = commit(&w.core, &grants, "x").unwrap_err();
        assert!(matches!(err.code, ErrorCode::NoVault | ErrorCode::Locked));
        assert!(grants.take_csv().is_some());
    }
}
