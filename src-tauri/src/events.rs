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

/// The main window should open the editor for a new login (the tray panel's "+"). Sent to the main
/// window only; it carries nothing, the main window builds the new item itself.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
pub struct NewEntryEvent;

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

/// The browser extension asked the main window to start a new entry of `entry_type` (only the
/// kinds the bridge cannot create itself: an address or a files item). The main window runs its
/// own "+" menu action, right away when unlocked, otherwise after the next unlock (if that comes
/// soon). Sent to the main window only.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NewEntryEvent {
    pub entry_type: mypass_session::EntryTypeDto,
}

/// A browser asked for an unlock. Payload-free; the unlock screen (stage 5) listens to it.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct UnlockRequestedEvent;

/// The secondary-folder sync status changed.
#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SyncStatusChanged(pub mypass_session::SecondaryStatus);

/// Where the open vault is stored, or how that storage is doing, changed (iCloud Drive: served
/// from the offline copy, edits not in iCloud yet, still downloading).
#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct StorageStatusChanged(pub mypass_session::StorageStatus);

/// The settings window, already open, should select `pane` (for example `browser`).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
pub struct SettingsPaneEvent {
    pub pane: String,
}

#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct UpdateChanged(pub mypass_session::update::UpdateStatus);
