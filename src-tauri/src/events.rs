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

use crate::commands::browser::PairingChallengeDto;

/// The pairing challenge changed: `Some` while a code is waiting for the browser, `None` once it
/// was finished, cancelled, expired or the vault locked. Sent to the main window only (it carries
/// the code the user types into the extension).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
pub struct PairingChallengeEvent(pub Option<PairingChallengeDto>);

/// The paired browsers, their diagnostics or the host registration changed (re-query
/// `browser_status`).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct BridgeChangedEvent;

/// What a save proposal is about.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ProposalKind {
    Login,
    Totp,
}

/// The browser proposed saving a login or a one-time-password secret. It carries no secret: the
/// main window takes the proposal with `browser_proposal_take`. Sent to the main window only.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProposalEvent {
    pub id: String,
    pub kind: ProposalKind,
    pub url: String,
    pub username: Option<String>,
    pub entry_id: Option<String>,
}

/// A browser asked for an unlock. Payload-free; the unlock screen (stage 5) listens to it.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct UnlockRequestedEvent;

/// The settings window, already open, should select `pane` (for example `browser`).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
pub struct SettingsPaneEvent {
    pub pane: String,
}
