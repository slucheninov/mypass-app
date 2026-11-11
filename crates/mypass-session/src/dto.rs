//! Entry, group and sidebar DTOs. Everything here crosses into the webview, so nothing secret is
//! ever a field: no password, TOTP secret, notes body, card number, CVV or protected value.
//! Timestamps are `f64` Unix milliseconds and sizes are `f64` bytes (specta rejects 64-bit
//! integers because JavaScript numbers cannot hold them).

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
