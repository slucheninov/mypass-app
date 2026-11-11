//! Entry, group and sidebar DTOs. Everything here crosses into the webview, so nothing secret is
//! ever a field: no password, TOTP secret, notes body, card number, CVV or protected value.
//! Timestamps are `f64` Unix milliseconds and sizes are `f64` bytes (specta rejects 64-bit
//! integers because JavaScript numbers cannot hold them).

use crate::settings::GeneratorOptionsDto;
use mypass_core::entry_ext::EntryType;
use mypass_kdbx::Timestamp;
use serde::{Deserialize, Serialize};

/// A model timestamp (Unix seconds) as the `f64` Unix milliseconds the webview consumes.
pub(crate) fn timestamp_ms(ts: Timestamp) -> f64 {
    ts.0 as f64 * 1000.0
}

#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    All,
    Favorites,
    Codes,
    Passkeys,
    Addresses,
    Cards,
    Notes,
    Files,
    Security,
    Deleted,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum EntryTypeDto {
    Login,
    Address,
    Card,
    Notes,
    Files,
}

impl From<EntryType> for EntryTypeDto {
    fn from(t: EntryType) -> Self {
        match t {
            EntryType::Login => Self::Login,
            EntryType::Address => Self::Address,
            EntryType::Card => Self::Card,
            EntryType::Notes => Self::Notes,
            EntryType::Files => Self::Files,
        }
    }
}

#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    Title,
    Modified,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Scope {
    Category { category: Category },
    Group { group_id: String },
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EntryQuery {
    pub scope: Scope,
    pub search: String,
    pub sort: SortKey,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SecurityFlag {
    Reused,
    Weak,
    EmptyPassword,
    InsecureUrl,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub entry_type: EntryTypeDto,
    pub favorite: bool,
    pub has_totp: bool,
    pub has_passkey: bool,
    pub attachment_count: u32,
    /// Unix milliseconds.
    pub modified: f64,
    pub group_id: String,
    pub in_recycle_bin: bool,
    pub flags: Vec<SecurityFlag>,
}

/// `count` is `None` for `Security`: the sidebar shows an em dash instead of a number.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCount {
    pub category: Category,
    pub count: Option<u32>,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GroupNode {
    pub id: String,
    pub name: String,
    pub emoji: Option<String>,
    /// Entries in the group's whole subtree.
    pub count: u32,
    pub is_recycle_bin: bool,
    pub children: Vec<GroupNode>,
}

/// `groups` are the children of the root group (whose id is `root_group_id`), the recycle bin
/// last.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Sidebar {
    pub categories: Vec<CategoryCount>,
    pub root_group_id: String,
    pub groups: Vec<GroupNode>,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AddressDto {
    pub label: String,
    pub full_name: String,
    pub given_name: String,
    pub family_name: String,
    pub organization: String,
    pub address_line1: String,
    pub address_line2: String,
    pub city: String,
    pub region: String,
    pub postal_code: String,
    pub country_code: String,
    pub country_name: String,
    pub phone: String,
    pub email: String,
}

/// Never carries the full number or the CVV.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CardDto {
    pub holder: String,
    pub number_last4: String,
    pub expiry: String,
    pub has_cvv: bool,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyMeta {
    pub relying_party: String,
    pub username: String,
    /// Unix milliseconds; absent for passkeys MyPass did not create.
    pub created: Option<f64>,
}

/// `value` is `None` when the field is protected.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldDto {
    pub key: String,
    pub protected: bool,
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    pub name: String,
    /// Bytes.
    pub size: f64,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetailDto {
    pub id: String,
    pub title: String,
    pub entry_type: EntryTypeDto,
    pub group_id: String,
    pub group_name: String,
    pub favorite: bool,
    /// Unix milliseconds.
    pub created: f64,
    /// Unix milliseconds.
    pub modified: f64,
    pub username: String,
    pub url: Option<String>,
    pub urls: Vec<String>,
    pub uri_match_mode: String,
    pub has_password: bool,
    pub totp_period: Option<u32>,
    pub totp_digits: Option<u32>,
    pub passkey: Option<PasskeyMeta>,
    pub address: Option<AddressDto>,
    pub card: Option<CardDto>,
    pub has_notes: bool,
    pub custom_fields: Vec<CustomFieldDto>,
    pub attachments: Vec<AttachmentMeta>,
    pub history_count: u32,
    pub in_recycle_bin: bool,
}

/// A TOTP display value (the code is meant to be shown, the secret never leaves Rust).
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TotpDto {
    pub code: String,
    pub seconds_remaining: u32,
    pub period: u32,
}

// ---- the edit model: the one sanctioned place where secrets cross into the webview ----
//
// `entry_edit_model` returns the plain values after the user explicitly opens Edit; the UI drops
// the model on close, save and lock. Because these types carry secrets, their `Debug` impls are
// manual and print structure (counts, field names) but never a value.

/// One user-defined field of an entry. `key` is never a reserved key.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldEdit {
    pub key: String,
    pub value: String,
    pub protected: bool,
}

impl std::fmt::Debug for CustomFieldEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomFieldEdit")
            .field("key", &self.key)
            .field("protected", &self.protected)
            .finish_non_exhaustive()
    }
}

/// The four card fields as typed by the user.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CardEdit {
    pub holder: String,
    pub number: String,
    pub expiry: String,
    pub cvv: String,
}

impl std::fmt::Debug for CardEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CardEdit").finish_non_exhaustive()
    }
}

/// A pending attachment change, applied in order on save.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AttachmentOp {
    /// Reads the file at `path` (its name and bytes) when the entry is saved.
    Add {
        path: String,
    },
    Remove {
        name: String,
    },
}

/// Everything the editor shows and saves. `id` is `None` for a new entry. `attachments` is
/// informational (the current files); changes go through `attachment_ops`.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryEditModel {
    pub id: Option<String>,
    pub entry_type: EntryTypeDto,
    pub group_id: String,
    pub title: String,
    pub favorite: bool,
    pub username: String,
    pub password: String,
    pub urls: Vec<String>,
    pub uri_match_mode: String,
    pub totp: String,
    pub notes: String,
    pub address: Option<AddressDto>,
    pub card: Option<CardEdit>,
    pub custom_fields: Vec<CustomFieldEdit>,
    pub attachments: Vec<AttachmentMeta>,
    pub attachment_ops: Vec<AttachmentOp>,
}

impl std::fmt::Debug for EntryEditModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntryEditModel")
            .field("id", &self.id)
            .field("entry_type", &self.entry_type)
            .field("group_id", &self.group_id)
            .field("urls", &self.urls.len())
            .field("custom_fields", &self.custom_fields.len())
            .field("attachments", &self.attachments.len())
            .field("attachment_ops", &self.attachment_ops.len())
            .finish_non_exhaustive()
    }
}

/// A single field to reveal or copy. `Totp` resolves to the current code.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FieldRef {
    Username,
    Password,
    Url,
    NotesBody,
    CardNumber,
    CardCvv,
    Totp,
    Custom { key: String },
}

/// One earlier version of an entry. `index` 0 is the newest; `changed_fields` names what the next
/// newer version (or the current entry) changed relative to this one.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub index: u32,
    /// Unix milliseconds.
    pub modified: f64,
    pub title: String,
    pub changed_fields: Vec<String>,
}

/// Input of the password generator. `host` selects the site rules; `record_url`, when present,
/// also writes the generated value to the generator history.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    pub options: GeneratorOptionsDto,
    pub host: Option<String>,
    pub record_url: Option<String>,
}

/// The site rules that applied, as the generator panel shows them. Class names are `lowercase`,
/// `uppercase`, `digits` and `symbols`.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RulesSummary {
    pub min_length: Option<u32>,
    pub max_length: Option<u32>,
    pub max_consecutive: Option<u32>,
    pub forced_on: Vec<String>,
    pub excluded: Vec<String>,
}

/// A freshly generated password: a display value, like the one a new-password field shows.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedPassword {
    pub value: String,
    pub effective: GeneratorOptionsDto,
    pub rules: Option<RulesSummary>,
    pub history_id: Option<String>,
}

impl std::fmt::Debug for GeneratedPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedPassword")
            .field("value", &"<redacted>")
            .field("effective", &self.effective)
            .field("rules", &self.rules)
            .field("history_id", &self.history_id)
            .finish()
    }
}

/// One generator history row. It never carries the password; reveal it by id.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorHistoryItem {
    pub id: String,
    pub url: String,
    pub title: String,
    /// Unix milliseconds.
    pub created: f64,
}

// ---- CSV import, vault discovery and the open-time guards (Task 13) ----

/// Why a CSV data row was not imported. A stable code: the UI localises it, the backend never
/// emits prose (and never echoes a cell value).
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// The row has a different number of fields than the header.
    FieldCountMismatch,
    /// A quoted field was never closed (the rest of the file belongs to that row).
    UnterminatedQuote,
}

/// A skipped CSV row; `line` is the 1-based physical line on which the row starts.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRow {
    pub line: u32,
    pub reason: SkipReason,
}

/// What the import dialog shows before anything is written: no passwords, only the first titles.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreview {
    pub total: u32,
    pub skipped: Vec<SkippedRow>,
    pub sample_titles: Vec<String>,
}

/// The outcome of an import: rows written and rows skipped by the parser.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: u32,
    pub skipped: u32,
}

/// A `.kdbx` file found by the scanner. `modified` is Unix milliseconds, `size` bytes.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FoundVaultFileDto {
    pub path: String,
    pub modified: f64,
    pub size: f64,
}

/// One row of the recent-vaults list.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecentVaultDto {
    pub path: String,
    pub file_name: String,
    pub exists: bool,
}

/// A reason to ask for confirmation before creating or opening a vault (D6, D7).
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenWarning {
    /// The file lives under `~/Library/Mobile Documents` (iCloud Drive).
    Icloud,
    /// Another `com.mypass.app` process, with a different executable (the old Swift app), runs.
    OldAppRunning { executable: String },
}

// ---- app facade (Task 14) ----

/// What the About pane and the unlock screen need to know about the running app.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    /// `macos`, `windows` or `linux`.
    pub os: String,
    /// Device unlock (Touch ID, account password) can be used on this platform.
    pub device_unlock_available: bool,
}

/// The launch-at-login state, mirrored from `mypass_platform::LoginItemStatus` so the platform
/// crate needs no `specta`. `Unsupported` is a value: the General pane hides the row.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum LoginItemStatus {
    Enabled,
    Disabled,
    RequiresApproval,
    NotFound,
    Unsupported,
}

impl From<mypass_platform::LoginItemStatus> for LoginItemStatus {
    fn from(s: mypass_platform::LoginItemStatus) -> Self {
        use mypass_platform::LoginItemStatus as P;
        match s {
            P::Enabled => Self::Enabled,
            P::Disabled => Self::Disabled,
            P::RequiresApproval => Self::RequiresApproval,
            P::NotFound => Self::NotFound,
            P::Unsupported => Self::Unsupported,
        }
    }
}

/// Asks the main window to open entry `id` (in the editor when `edit`); the Tauri event wraps it.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenEntryEvent {
    pub id: String,
    pub edit: bool,
}
