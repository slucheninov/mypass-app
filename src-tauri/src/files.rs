//! The native file dialogs and the grants they produce.
//!
//! The webview never supplies a file-system path for an attachment, an attachment export or a
//! CSV import: a webview-chosen destination would be an arbitrary file write, and a
//! webview-chosen source an arbitrary file read. The dialogs open from Rust behind the
//! [`FilePicker`] trait, and what the user chose stays in Rust:
//!
//! - picked attachments become single-use random tokens ([`FileGrants::grant_tokens`]) that the
//!   UI hands back inside `AttachmentOp::Add { path }`; [`FileGrants::redeem`] swaps each for the
//!   real path;
//! - the CSV file chosen for a preview is remembered once and consumed by the commit
//!   ([`FileGrants::set_csv`] / [`FileGrants::take_csv`]).
//!
//! Vault open and create still take a path from the UI (the stage's contract); those two flows
//! are the only ones that need the webview's `dialog:allow-open` / `dialog:allow-save`.

use mypass_session::dto::AttachmentOp;
use mypass_session::{ApiError, ErrorCode};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use uuid::Uuid;

/// How many unredeemed attachment tokens are kept; the oldest are forgotten first.
const MAX_TOKENS: usize = 64;

/// The native dialogs. Blocking: callers run it off the async runtime.
pub trait FilePicker: Send + Sync {
    /// The open dialog for attachments (several files); empty when cancelled.
    fn pick_attachments(&self) -> Vec<PathBuf>;
    /// The open dialog for one CSV file; `None` when cancelled.
    fn pick_csv(&self) -> Option<PathBuf>;
    /// The save dialog, suggesting `file_name`; `None` when cancelled.
    fn save_as(&self, file_name: &str) -> Option<PathBuf>;
}

/// [`FilePicker`] over `tauri-plugin-dialog`'s blocking dialogs.
pub struct TauriFilePicker(pub tauri::AppHandle);

impl FilePicker for TauriFilePicker {
    fn pick_attachments(&self) -> Vec<PathBuf> {
        use tauri_plugin_dialog::DialogExt;
        self.0
            .dialog()
            .file()
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.into_path().ok())
            .collect()
    }

    fn pick_csv(&self) -> Option<PathBuf> {
        use tauri_plugin_dialog::DialogExt;
        self.0
            .dialog()
            .file()
            .add_filter("CSV", &["csv"])
            .blocking_pick_file()
            .and_then(|p| p.into_path().ok())
    }

    fn save_as(&self, file_name: &str) -> Option<PathBuf> {
        use tauri_plugin_dialog::DialogExt;
        self.0
            .dialog()
            .file()
            .set_file_name(file_name)
            .blocking_save_file()
            .and_then(|p| p.into_path().ok())
    }
}

/// A [`FilePicker`] with scripted answers, for tests.
#[derive(Debug, Default)]
pub struct FakeFilePicker {
    pub attachments: Mutex<Vec<PathBuf>>,
    pub csv: Mutex<Option<PathBuf>>,
    pub save: Mutex<Option<PathBuf>>,
    /// The file names `save_as` was asked to suggest.
    pub save_names: Mutex<Vec<String>>,
    /// Every dialog opened, by name (`attachments`, `csv`, `save`).
    pub dialogs: Mutex<Vec<&'static str>>,
}

impl FakeFilePicker {
    fn opened(&self, which: &'static str) {
        self.dialogs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(which);
    }

    /// How many dialogs were opened.
    pub fn dialog_count(&self) -> usize {
        self.dialogs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl FilePicker for FakeFilePicker {
    fn pick_attachments(&self) -> Vec<PathBuf> {
        self.opened("attachments");
        self.attachments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn pick_csv(&self) -> Option<PathBuf> {
        self.opened("csv");
        self.csv
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn save_as(&self, file_name: &str) -> Option<PathBuf> {
        self.opened("save");
        self.save_names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(file_name.to_owned());
        self.save
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A file the user picked for an attachment: the UI shows `name` and `size` and passes `token`
/// back in `AttachmentOp::Add { path }`.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    pub token: String,
    pub name: String,
    /// Bytes (`f64`: specta rejects 64-bit integers).
    pub size: f64,
}

#[derive(Default)]
struct Grants {
    tokens: VecDeque<(String, PathBuf)>,
    csv: Option<PathBuf>,
}

/// The app state behind the dialog commands (cheap to clone).
#[derive(Clone, Default)]
pub struct FileGrants(Arc<Mutex<Grants>>);

fn invalid(message: &str) -> ApiError {
    ApiError {
        code: ErrorCode::InvalidInput,
        message: message.to_owned(),
    }
}

impl FileGrants {
    fn with<R>(&self, f: impl FnOnce(&mut Grants) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Stores each picked path under a fresh random single-use token.
    pub fn grant_tokens(&self, paths: Vec<PathBuf>) -> Vec<PickedFile> {
        self.with(|g| {
            paths
                .into_iter()
                .map(|path| {
                    let token = Uuid::new_v4().simple().to_string();
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let size = std::fs::metadata(&path).map_or(0.0, |m| m.len() as f64);
                    g.tokens.push_back((token.clone(), path));
                    while g.tokens.len() > MAX_TOKENS {
                        g.tokens.pop_front();
                    }
                    PickedFile { token, name, size }
                })
                .collect()
        })
    }

    /// Replaces the token in every `Add` operation by its real path and consumes the tokens.
    /// An unknown or already used token (also a token repeated inside the list) is
    /// `InvalidInput` and nothing is consumed. Returns the consumed `(token, path)` pairs, which
    /// [`FileGrants::restore`] puts back when the save then fails.
    pub fn redeem(&self, ops: &mut [AttachmentOp]) -> Result<Vec<(String, PathBuf)>, ApiError> {
        self.with(|g| {
            let mut taken: Vec<(String, PathBuf)> = Vec::new();
            for op in ops.iter() {
                if let AttachmentOp::Add { path: token } = op {
                    let found = g
                        .tokens
                        .iter()
                        .find(|(t, _)| t == token)
                        .filter(|_| !taken.iter().any(|(t, _)| t == token));
                    match found {
                        Some(pair) => taken.push(pair.clone()),
                        None => return Err(invalid("unknown or already used file token")),
                    }
                }
            }
            g.tokens.retain(|(t, _)| !taken.iter().any(|(u, _)| u == t));
            for op in ops.iter_mut() {
                if let AttachmentOp::Add { path } = op {
                    if let Some((_, real)) = taken.iter().find(|(t, _)| t == path) {
                        *path = real.to_string_lossy().into_owned();
                    }
                }
            }
            Ok(taken)
        })
    }

    /// Puts redeemed tokens back (the save they were redeemed for failed, so the editor still
    /// holds them).
    pub fn restore(&self, taken: Vec<(String, PathBuf)>) {
        self.with(|g| {
            for pair in taken {
                if !g.tokens.iter().any(|(t, _)| *t == pair.0) {
                    g.tokens.push_back(pair);
                }
            }
            while g.tokens.len() > MAX_TOKENS {
                g.tokens.pop_front();
            }
        });
    }

    /// Forgets every token and the remembered CSV file (the vault locked or was switched).
    pub fn clear(&self) {
        self.with(|g| *g = Grants::default());
    }

    /// Remembers the CSV file of the last preview (replacing an earlier one).
    pub fn set_csv(&self, path: Option<PathBuf>) {
        self.with(|g| g.csv = path);
    }

    /// The remembered CSV file, exactly once.
    pub fn take_csv(&self) -> Option<PathBuf> {
        self.with(|g| g.csv.take())
    }
}
