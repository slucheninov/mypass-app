//! Entry, group and sidebar DTOs. Everything here crosses into the webview, so nothing secret is
//! ever a field: no password, TOTP secret, notes body, card number, CVV or protected value.
//! Timestamps are `f64` Unix milliseconds and sizes are `f64` bytes (specta rejects 64-bit
//! integers because JavaScript numbers cannot hold them).

use crate::settings::GeneratorOptionsDto;
pub use crate::sign_in::SignInProvider;
use mypass_core::entry_ext::EntryType;
use mypass_kdbx::Timestamp;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// A model timestamp (Unix seconds) as the `f64` Unix milliseconds the webview consumes.
pub(crate) fn timestamp_ms(ts: Timestamp) -> f64 {
    ts.0 as f64 * 1000.0
}

/// One revealed secret value on its way to the webview. The text stays in a wiping buffer until
/// the IPC layer has serialised it, and the buffer is then wiped when this value drops, so the
/// Rust-side source copy of a revealed password never outlives the command (the serialised copy
/// belongs to the IPC layer and is outside our control). It serialises, and appears in the
/// generated bindings, as a plain `string`; `Debug` is redacted.
#[derive(specta::Type)]
#[specta(transparent)]
pub struct SecretText(#[specta(type = String)] Zeroizing<String>);

impl SecretText {
    pub(crate) fn new(value: Zeroizing<String>) -> Self {
        Self(value)
    }

    /// The text, borrowed; tests and in-process consumers read it here.
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl Serialize for SecretText {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretText(<redacted>)")
    }
}

impl PartialEq<str> for SecretText {
    fn eq(&self, other: &str) -> bool {
        self.expose() == other
    }
}

impl PartialEq<&str> for SecretText {
    fn eq(&self, other: &&str) -> bool {
        self.expose() == *other
    }
}

impl PartialEq<String> for SecretText {
    fn eq(&self, other: &String) -> bool {
        self.expose() == other.as_str()
    }
}

#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    All,
    Favorites,
    Codes,
    Passkeys,
    Addresses,
    People,
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
    Person,
    Card,
    Notes,
    Files,
}

impl From<EntryType> for EntryTypeDto {
    fn from(t: EntryType) -> Self {
        match t {
            EntryType::Login => Self::Login,
            EntryType::Address => Self::Address,
            EntryType::Person => Self::Person,
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
    /// KDBX `CreationTime`.
    Created,
    /// KDBX `LastModificationTime`.
    Modified,
    /// "Recently used": KDBX `LastAccessTime`, set when a field is copied or the bridge fills.
    Accessed,
    /// "Frequently used": KDBX `UsageCount`, bumped together with `Accessed`.
    UsageCount,
}

/// The direction of an [`EntryQuery`] sort. Ties always fall back to the title (A to Z, case
/// insensitive) and then the entry id, whatever the direction, so the order is stable.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SortOrder {
    /// A to Z, oldest first, least used first.
    Ascending,
    /// Z to A, newest first, most used first.
    Descending,
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
    pub order: SortOrder,
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
    pub has_username: bool,
    pub has_password: bool,
    pub has_totp: bool,
    pub has_passkey: bool,
    pub attachment_count: u32,
    /// Unix milliseconds.
    pub modified: f64,
    pub group_id: String,
    pub in_recycle_bin: bool,
    pub flags: Vec<SecurityFlag>,
    /// The social sign-in method remembered on the entry (a list badge), absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in_provider: Option<SignInProvider>,
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
    /// The vault's `RecycleBinEnabled` flag: when false a delete is permanent.
    pub recycle_bin_enabled: bool,
}

#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Default)]
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

impl AddressDto {
    /// An empty address with `label`. A plain constructor because the type wipes itself on drop,
    /// so a struct update from a default value cannot move fields out of it.
    pub fn with_label(label: &str) -> Self {
        Self {
            label: label.to_owned(),
            full_name: String::new(),
            given_name: String::new(),
            family_name: String::new(),
            organization: String::new(),
            address_line1: String::new(),
            address_line2: String::new(),
            city: String::new(),
            region: String::new(),
            postal_code: String::new(),
            country_code: String::new(),
            country_name: String::new(),
            phone: String::new(),
            email: String::new(),
        }
    }

    /// Overwrites every address string with zeros and empties it; also what `Drop` runs. The
    /// address is stored as protected fields, so every copy of it is a secret copy.
    pub(crate) fn wipe(&mut self) {
        self.label.zeroize();
        self.full_name.zeroize();
        self.given_name.zeroize();
        self.family_name.zeroize();
        self.organization.zeroize();
        self.address_line1.zeroize();
        self.address_line2.zeroize();
        self.city.zeroize();
        self.region.zeroize();
        self.postal_code.zeroize();
        self.country_code.zeroize();
        self.country_name.zeroize();
        self.phone.zeroize();
        self.email.zeroize();
    }
}

impl Drop for AddressDto {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl std::fmt::Debug for AddressDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddressDto").finish_non_exhaustive()
    }
}

/// The fields of a Person entry. Every value is stored protected, so every copy is a secret copy:
/// the type wipes itself on drop and its `Debug` prints nothing. `address_ids` are the linked
/// Address entries in order (the first is the default address), as entry ids.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonDto {
    pub label: String,
    pub given_name: String,
    pub family_name: String,
    pub middle_name: String,
    pub email: String,
    /// An ISO `YYYY-MM-DD` date, or empty.
    pub birth_date: String,
    pub address_ids: Vec<String>,
}

impl PersonDto {
    /// An empty person. A plain constructor because the type wipes itself on drop.
    pub fn empty() -> Self {
        Self {
            label: String::new(),
            given_name: String::new(),
            family_name: String::new(),
            middle_name: String::new(),
            email: String::new(),
            birth_date: String::new(),
            address_ids: Vec::new(),
        }
    }

    /// The given, middle and family names joined by single spaces, the empty ones left out.
    pub fn full_name(&self) -> String {
        [
            self.given_name.trim(),
            self.middle_name.trim(),
            self.family_name.trim(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }

    /// Overwrites every string with zeros and empties it; also what `Drop` runs.
    pub(crate) fn wipe(&mut self) {
        self.label.zeroize();
        self.given_name.zeroize();
        self.family_name.zeroize();
        self.middle_name.zeroize();
        self.email.zeroize();
        self.birth_date.zeroize();
        for id in &mut self.address_ids {
            id.zeroize();
        }
        self.address_ids.clear();
    }
}

impl Drop for PersonDto {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl std::fmt::Debug for PersonDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonDto").finish_non_exhaustive()
    }
}

/// A live Address entry a person links to, as the person's detail shows it (title and the list
/// subtitle only, the address values stay in that entry).
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LinkedAddressDto {
    pub id: String,
    pub title: String,
    pub subtitle: String,
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
    /// `false` for a damaged passkey: a private key with a missing or undecodable credential id.
    /// It cannot sign in anywhere and counts nowhere (category, badge, browser); it is shown only
    /// so the user can remove it. `serde(default)` keeps older stored drafts loadable.
    #[serde(default)]
    pub usable: bool,
}

/// The passkey of an entry as shown after an explicit click: [`PasskeyMeta`] plus the credential
/// id, which users compare with the relying party's list of keys. The credential id is public
/// (every assertion sends it); the private key PEM, the user handle and every other piece of key
/// material are never part of it.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyDetails {
    pub relying_party: String,
    pub username: String,
    /// Unix milliseconds; absent for passkeys MyPass did not create.
    pub created: Option<f64>,
    /// Canonical unpadded base64url.
    pub credential_id: String,
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
    pub person: Option<PersonDto>,
    /// The person's linked Address entries that still exist outside the recycle bin, in the
    /// person's order (missing and deleted ones are skipped). Empty for every other type.
    pub person_addresses: Vec<LinkedAddressDto>,
    pub card: Option<CardDto>,
    pub has_notes: bool,
    pub custom_fields: Vec<CustomFieldDto>,
    pub attachments: Vec<AttachmentMeta>,
    pub history_count: u32,
    pub in_recycle_bin: bool,
    /// The social sign-in method remembered on the entry, absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<SignInDto>,
}

/// The social sign-in method of an entry: the provider and, when the browser recorded it, the
/// last use (Unix milliseconds).
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignInDto {
    pub provider: SignInProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<f64>,
}

/// A TOTP display value (the code is meant to be shown, the secret never leaves Rust).
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TotpDto {
    pub code: String,
    pub seconds_remaining: u32,
    pub period: u32,
}

impl std::fmt::Debug for TotpDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TotpDto")
            .field("code", &"<redacted>")
            .field("seconds_remaining", &self.seconds_remaining)
            .field("period", &self.period)
            .finish()
    }
}

/// The one-time code is short-lived but still a credential: the session-side copy is wiped when
/// the value is dropped (the IPC copy is outside our control).
impl TotpDto {
    pub(crate) fn wipe(&mut self) {
        self.code.zeroize();
    }
}

impl Drop for TotpDto {
    fn drop(&mut self) {
        self.wipe();
    }
}

// ---- the edit model: the one sanctioned place where secrets cross into the webview ----
//
// `entry_edit_model` returns the plain values after the user explicitly opens Edit; the UI drops
// the model on close, save and lock. Because these types carry secrets, their `Debug` impls are
// manual and print structure (counts, field names) but never a value.
//
// The session-side copies are wiped when a model is dropped (`Drop` impls below, via `zeroize`),
// so a model that was built for the editor, cloned for a draft or discarded after a failed save
// does not leave its passwords, notes, card numbers or custom values in freed heap memory. What
// Tauri's IPC layer serialises from these values (JSON text, the webview's own strings) is a copy
// we do not control and cannot wipe; keeping the lifetime of a model short is all we can do.

/// One user-defined field of an entry. `key` is never a reserved key.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldEdit {
    pub key: String,
    pub value: String,
    pub protected: bool,
}

impl CustomFieldEdit {
    /// Overwrites the value with zeros and empties it; also what `Drop` runs.
    pub(crate) fn wipe(&mut self) {
        self.value.zeroize();
    }
}

impl Drop for CustomFieldEdit {
    fn drop(&mut self) {
        self.wipe();
    }
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

impl CardEdit {
    /// Overwrites every card value with zeros and empties it; also what `Drop` runs.
    pub(crate) fn wipe(&mut self) {
        self.holder.zeroize();
        self.number.zeroize();
        self.expiry.zeroize();
        self.cvv.zeroize();
    }
}

impl Drop for CardEdit {
    fn drop(&mut self) {
        self.wipe();
    }
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
    /// `serde(default)` keeps encrypted drafts stored before this field existed loadable.
    #[serde(default)]
    pub person: Option<PersonDto>,
    pub card: Option<CardEdit>,
    pub custom_fields: Vec<CustomFieldEdit>,
    pub attachments: Vec<AttachmentMeta>,
    pub attachment_ops: Vec<AttachmentOp>,
    /// The entry's passkey, for the read-only Passkey row; informational only, ignored on save.
    /// It never carries a `KPEX_PASSKEY_*` secret (no PEM, credential id or user handle).
    /// `serde(default)` keeps encrypted drafts stored before this field existed loadable.
    #[serde(default)]
    pub passkey: Option<PasskeyMeta>,
    /// Remove the passkey when this model is saved (every `KPEX_PASSKEY_*` field); the removal
    /// waits for the save, so cancelling the editor keeps the passkey.
    #[serde(default)]
    pub remove_passkey: bool,
    /// An opaque reference to the passkey this model was built with (not a `KPEX_PASSKEY_*`
    /// value). `remove_passkey` removes the stored passkey only while it still matches, so a
    /// passkey stored after the editor opened (a browser replace) is never removed unseen.
    #[serde(default)]
    pub passkey_ref: Option<String>,
    /// The user explicitly chose to bring this deleted entry back: when it sits in the recycle
    /// bin, a save is refused unless this is set, and then it moves the entry to `group_id`.
    #[serde(default)]
    pub restore: bool,
    /// The social sign-in method the editor shows (`None`: none). Informational only, ignored on
    /// save: `sign_in_edit` says what a save changes. `serde(default)` keeps older stored drafts
    /// loadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in_provider: Option<SignInProvider>,
    /// What a save does to the social sign-in method: `unchanged` (the default) leaves the stored
    /// fields alone, so a method the browser recorded while the editor was open survives a save
    /// of other fields; `set` stores a provider (another one drops the recorded last use, the
    /// same one keeps it); `clear` removes it.
    #[serde(default)]
    pub sign_in_edit: SignInEdit,
}

/// How a save changes the social sign-in method of an entry.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum SignInEdit {
    /// The editor did not touch it: the stored fields are kept as they are on disk.
    #[default]
    Unchanged,
    /// The user picked this provider.
    Set { provider: SignInProvider },
    /// The user picked "none".
    Clear,
}

impl EntryEditModel {
    /// Overwrites the model's own secret strings with zeros and empties them; also what `Drop`
    /// runs (the card and the custom fields wipe themselves when they drop with the model).
    pub(crate) fn wipe(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
        self.totp.zeroize();
        self.notes.zeroize();
    }
}

impl Drop for EntryEditModel {
    fn drop(&mut self) {
        self.wipe();
    }
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
            .field("passkey", &self.passkey.is_some())
            .field("remove_passkey", &self.remove_passkey)
            .field("passkey_ref", &self.passkey_ref.is_some())
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
    UrlAt { index: u32 },
    NotesBody,
    CardNumber,
    CardCvv,
    Address { name: String },
    Person { name: String },
    Totp,
    Custom { key: String },
}

/// One earlier version of an entry. `index` 0 is the newest; `changed_fields` names what the next
/// newer version (or the current entry) changed relative to this one.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    /// The position in the list, for restoring. It shifts whenever a version is added or dropped,
    /// so a value is revealed by `revision`, never by this.
    pub index: u32,
    /// An opaque, stable name of this exact version (the same one across reloads while the version
    /// exists, a different one for a different version even with equal title and date). It is
    /// derived with a per-run secret, so it reveals nothing about the values it covers.
    pub revision: String,
    /// Unix milliseconds.
    pub modified: f64,
    pub title: String,
    pub changed_fields: Vec<String>,
    /// The changed fields whose old value this version still holds and that can be revealed, in
    /// the order of `changed_fields`. Reserved values (the code, passkey material, attachment
    /// bytes, the app's own bookkeeping) are never listed.
    pub revealable: Vec<HistoryReveal>,
}

/// One revealable field of a history version: `key` is the changed-field name from
/// `HistoryItem::changed_fields`, `field` what to pass to `entry_history_reveal`.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryReveal {
    pub key: String,
    pub field: FieldRef,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::new_model;
    use uuid::Uuid;

    #[test]
    fn wipe_clears_every_secret_string_of_the_edit_model() {
        let mut m = new_model(EntryTypeDto::Login, Uuid::new_v4());
        m.username = "user".into();
        m.password = "hunter2".into();
        m.totp = "JBSWY3DPEHPK3PXP".into();
        m.notes = "private notes".into();
        m.wipe();
        assert!(m.username.is_empty() && m.password.is_empty());
        assert!(m.totp.is_empty() && m.notes.is_empty());
        // The non-secret structure is untouched.
        assert_eq!(m.entry_type, EntryTypeDto::Login);
    }

    #[test]
    fn wipe_clears_the_card_values_and_the_custom_value() {
        let mut card = CardEdit {
            holder: "J Doe".into(),
            number: "4111111111111111".into(),
            expiry: "09/28".into(),
            cvv: "7391".into(),
        };
        card.wipe();
        assert_eq!(card, CardEdit::default());
        let mut field = CustomFieldEdit {
            key: "Recovery".into(),
            value: "r-1".into(),
            protected: true,
        };
        field.wipe();
        assert!(field.value.is_empty());
        assert_eq!(field.key, "Recovery");
    }

    #[test]
    fn wipe_clears_the_totp_code() {
        let mut t = TotpDto {
            code: "123456".into(),
            seconds_remaining: 12,
            period: 30,
        };
        t.wipe();
        assert!(t.code.is_empty());
        assert_eq!((t.seconds_remaining, t.period), (12, 30));
    }

    #[test]
    fn secret_bearing_types_still_redact_their_debug_output() {
        let card = CardEdit {
            holder: "J Doe".into(),
            number: "4111111111111111".into(),
            expiry: "09/28".into(),
            cvv: "7391".into(),
        };
        let field = CustomFieldEdit {
            key: "k".into(),
            value: "s3cret-value".into(),
            protected: true,
        };
        let mut m = new_model(EntryTypeDto::Login, Uuid::new_v4());
        m.password = "hunter2-pw".into();
        m.notes = "private notes".into();
        let mut address = AddressDto::with_label("Home");
        address.full_name = "Jane Q Public".into();
        address.address_line1 = "221B Baker Street".into();
        let totp = TotpDto {
            code: "482913".into(),
            seconds_remaining: 12,
            period: 30,
        };
        let secret_text = SecretText::new(Zeroizing::new("reveal-me-pw".to_owned()));
        let shown = format!("{card:?} {field:?} {m:?} {address:?} {totp:?} {secret_text:?}");
        for secret in [
            "4111111111111111",
            "7391",
            "s3cret-value",
            "hunter2-pw",
            "private notes",
            "Jane Q Public",
            "221B Baker Street",
            "482913",
            "reveal-me-pw",
        ] {
            assert!(!shown.contains(secret), "Debug leaked {secret}");
        }
    }

    #[test]
    fn address_wipe_empties_every_field_and_the_type_wipes_on_drop() {
        let mut a = AddressDto::with_label("Home");
        a.full_name = "Jane Q Public".into();
        a.phone = "+1 555 0100".into();
        a.wipe();
        assert_eq!(a, AddressDto::default());
        assert!(
            std::mem::needs_drop::<AddressDto>(),
            "the address must run its wipe when dropped"
        );
    }

    #[test]
    fn person_wipe_empties_every_field_debug_redacts_and_the_type_wipes_on_drop() {
        let mut p = PersonDto::empty();
        p.label = "Me".into();
        p.given_name = "Taras".into();
        p.middle_name = "Hryhorovych".into();
        p.family_name = "Shevchenko".into();
        p.email = "taras@example.test".into();
        p.birth_date = "1814-03-09".into();
        p.address_ids = vec!["0123456789abcdef0123456789abcdef".into()];
        assert_eq!(p.full_name(), "Taras Hryhorovych Shevchenko");
        let shown = format!("{p:?}");
        for secret in [
            "Taras",
            "Shevchenko",
            "taras@example.test",
            "1814-03-09",
            "0123",
        ] {
            assert!(!shown.contains(secret), "Debug leaked {secret}");
        }
        p.wipe();
        assert_eq!(p, PersonDto::default());
        assert!(std::mem::needs_drop::<PersonDto>());
    }

    #[test]
    fn person_full_name_skips_empty_parts() {
        let mut p = PersonDto::empty();
        assert_eq!(p.full_name(), "");
        p.family_name = " Doe ".into();
        assert_eq!(p.full_name(), "Doe");
        p.given_name = "Jane".into();
        assert_eq!(p.full_name(), "Jane Doe");
    }

    #[test]
    fn secret_text_serialises_as_a_plain_string_and_compares_by_value() {
        let t = SecretText::new(Zeroizing::new("Gen#Pass-1".to_owned()));
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"Gen#Pass-1\"");
        assert_eq!(t.expose(), "Gen#Pass-1");
        assert!(t == "Gen#Pass-1");
        assert!(std::mem::needs_drop::<SecretText>());
    }
}
