//! The typed events the backend emits to the webview (`tauri-specta`); the names are stable
//! (`session-changed`, `vault-changed`, ...), derived from the struct names.

use mypass_session::{PublicSettings, SaveStatus, SessionStatus, Settings};
use serde::{Deserialize, Serialize};

/// Lock state, vault or path changed.
#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SessionChanged(pub SessionStatus);

/// The vault content changed in memory (lists and details are stale).
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct VaultChanged;

/// Device unlock attempts changed for the locked vault.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct DeviceUnlockChanged;

/// The save state changed.
#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SaveStateChanged(pub SaveStatus);

/// The settings changed. A broadcast reaches every webview in any session state, so it carries
/// only the public part (theme, language, shortcut hints, the update flag); a window that shows
/// the full settings re-reads them through the gated `settings_get`.
#[derive(Serialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SettingsChanged(pub PublicSettings);

impl SettingsChanged {
    pub fn of(settings: &Settings) -> Self {
        Self(settings.into())
    }
}

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

/// The pairing challenge changed (a code is waiting for the browser, or it was finished,
/// cancelled, expired or dismissed by a lock). Payload-free: Tauri also hands a targeted event to
/// `Any` listeners of other webviews, so the code never travels in it; the main window reads it
/// with `browser_pairing_status`.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
pub struct PairingChallengeEvent;

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

/// A new-entry request is waiting (`AppCore::new_entry_requests`): a login from the tray panel's
/// "+", or a kind the bridge cannot create itself (an address or a files item) from the browser
/// extension. Payload-free, only a nudge: the main window takes the request with
/// `app_take_new_entry` (main window only) and runs its own "+" menu action. Any webview may emit
/// this event, so it must never carry the request itself; a forged one finds nothing to take.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, PartialEq, Debug)]
pub struct NewEntryEvent;

/// A browser asked for an unlock. Payload-free; the unlock screen (stage 5) listens to it.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct UnlockRequestedEvent;

/// The secondary-folder sync status changed. Payload-free, as every webview receives it in any
/// session state: the settings window re-reads the status through the gated `sync_status`.
#[derive(Serialize, Deserialize, specta::Type, tauri_specta::Event, Clone, Debug)]
pub struct SyncStatusChanged;

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// What crosses the webview boundary in a broadcast: the public settings only, and nothing at
    /// all for the sync status and the pairing challenge.
    #[test]
    fn broadcasts_carry_no_settings_sync_state_or_pairing_code() {
        let settings = Settings {
            secondary_folder: Some("/Users/me/Dropbox".into()),
            vault_path: Some("/Users/me/vault.kdbx".into()),
            recent_vaults: vec!["/Users/me/vault.kdbx".into()],
            ..Settings::default()
        };
        let json = serde_json::to_value(SettingsChanged::of(&settings)).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "language",
                "shortcutLock",
                "shortcutShow",
                "theme",
                "updateCheckEnabled"
            ]
        );
        assert!(!json.to_string().contains("/Users/me"));
        assert_eq!(
            serde_json::to_value(SyncStatusChanged).unwrap(),
            serde_json::Value::Null
        );
        assert_eq!(
            serde_json::to_value(PairingChallengeEvent).unwrap(),
            serde_json::Value::Null
        );
    }
}
