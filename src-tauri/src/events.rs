//! The typed events the backend emits to the webview (`tauri-specta`); the names are stable
//! (`session-changed`, `vault-changed`, ...), derived from the struct names.

use mypass_session::{SaveStatus, SessionStatus, Settings};
use serde::{Deserialize, Serialize};

/// Lock state, vault or path changed.
#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SessionChanged(pub SessionStatus);

/// The vault content changed in memory (lists and details are stale).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct VaultChanged;

/// The save state changed.
#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SaveStateChanged(pub SaveStatus);

/// The settings changed (every window shows the same ones).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SettingsChanged(pub Settings);

/// A copied secret was cleared from the clipboard.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct ClipboardClearedEvent;

/// Which global shortcut a conflict is about.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutKind {
    Lock,
    Show,
}

/// A global shortcut could not be registered (already taken, or invalid). Emitted by the hotkey
/// layer (Task 17).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConflictEvent {
    pub kind: ShortcutKind,
    pub shortcut: String,
}

/// The main window should open entry `id` (in the editor when `edit`).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenEntryEvent {
    pub id: String,
    pub edit: bool,
}

impl From<mypass_session::OpenEntryEvent> for OpenEntryEvent {
    fn from(e: mypass_session::OpenEntryEvent) -> Self {
        Self {
            id: e.id,
            edit: e.edit,
        }
    }
}
