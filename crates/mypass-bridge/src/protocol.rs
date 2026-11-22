//! The wire protocol between the extension (through the native host) and the app.
//!
//! Every frame is one JSON object tagged by `type`. The shapes are byte-compatible with the earlier
//! Swift app, so the extension keeps working unchanged: camelCase names, optional fields omitted
//! (never `null`), and the Swift decoding defaults kept as `serde(default)`.
//!
//! Secrets never reach `Debug` output: `Debug` for the requests and responses prints the message tag
//! only, and the address and history item types hide their values.

use std::{borrow::Cow, fmt};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The wire `type` of every [`Request`] variant, in declaration order.
pub const REQUEST_TYPES: [&str; 35] = [
    "getCredentials",
    "getFavorites",
    "listEntries",
    "searchEntries",
    "createEntry",
    "fillAddress",
    "fillPerson",
    "fillCredential",
    "getTotp",
    "proposeSaveLogin",
    "addEntryIdentifier",
    "proposeSaveTotp",
    "startPairing",
    "completePairing",
    "requestUnlock",
    "lockVault",
    "openApp",
    "openBrowserSettings",
    "reportBrowserDiagnostics",
    "getPasskeyProxyStatus",
    "setPasskeyProxyEnabled",
    "passkeyCreate",
    "passkeyCreateConfirm",
    "passkeyGet",
    "passkeyGetApprove",
    "passkeyCancel",
    "generatePassword",
    "recordGeneratedPassword",
    "getGeneratedPasswordHistory",
    "deleteGeneratedPassword",
    "unlockWithPassword",
    "unlockWithDevice",
    "getUnlockOptions",
    "recordSocialSignIn",
    "getSocialSignIn",
];

/// Inspect the tag without deserialising the other fields into serde's enum buffer.
pub(crate) fn message_type(frame: &[u8]) -> Option<Cow<'_, str>> {
    #[derive(Deserialize)]
    struct MessageType<'a> {
        #[serde(borrow, rename = "type")]
        kind: Cow<'a, str>,
    }

    serde_json::from_slice::<MessageType<'_>>(frame)
        .ok()
        .map(|tag| tag.kind)
}

/// Master password received from the extension. Its backing string is wiped on drop.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct WirePassword(Zeroizing<String>);

impl WirePassword {
    pub fn new(password: String) -> Self {
        Self(Zeroizing::new(password))
    }

    pub fn char_len(&self) -> usize {
        self.0.chars().count()
    }

    #[cfg(feature = "server")]
    pub fn into_secret(self) -> secrecy::SecretString {
        // Copy into the final secret and let the Zeroizing string erase its allocation.
        secrecy::SecretString::from(self.0.as_str())
    }
}

impl fmt::Debug for WirePassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WirePassword(<redacted>)")
    }
}

impl Serialize for WirePassword {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for WirePassword {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// How the extension saw a social sign-in click.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SocialSignInSource {
    /// A trusted click on the button in the page's own top frame.
    Click,
    /// The heuristic for a provider's cross-origin button frame (a trusted pointer over the frame
    /// right before the page lost focus to it). Weaker: it never creates an entry and never
    /// overrides the once-a-day limit.
    #[default]
    Frame,
}

/// A request from the extension. Every variant carries the endpoint `token`; the ones the Swift app
/// accepts a pairing secret on end with `pairing`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Request {
    GetCredentials {
        token: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    GetFavorites {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    ListEntries {
        token: String,
        category: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    SearchEntries {
        token: String,
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    CreateEntry {
        token: String,
        entry_type: String,
        #[serde(default, deserialize_with = "null_as_default")]
        title: String,
        #[serde(default, deserialize_with = "null_as_default")]
        username: String,
        #[serde(default, deserialize_with = "null_as_default")]
        password: String,
        #[serde(default, deserialize_with = "null_as_default")]
        url: String,
        #[serde(default, deserialize_with = "null_as_default")]
        card_number: String,
        #[serde(default, deserialize_with = "null_as_default")]
        cardholder_name: String,
        #[serde(default, deserialize_with = "null_as_default")]
        expiry: String,
        #[serde(default, deserialize_with = "null_as_default")]
        cvv: String,
        #[serde(default, deserialize_with = "null_as_default")]
        notes: String,
        #[serde(default, deserialize_with = "null_as_default")]
        favorite: bool,
        #[serde(default, rename = "match", deserialize_with = "null_as_default")]
        r#match: String,
        #[serde(default, deserialize_with = "null_as_default")]
        otp: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    FillAddress {
        token: String,
        id: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    /// A Person entry and one Address entry for a form: `address_id` names the address the user
    /// picked (any live Address entry); absent, the person's first live linked address is used.
    FillPerson {
        token: String,
        id: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    FillCredential {
        token: String,
        id: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    GetTotp {
        token: String,
        id: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    ProposeSaveLogin {
        token: String,
        url: String,
        username: String,
        password: String,
        /// The other values of a submitted registration form (names, email, phone, birthday,
        /// address parts, other labelled text fields), never a password, a card value or a hidden
        /// field. Absent for a plain login save. Bounded and checked by the handler, which refuses
        /// the whole proposal when a limit is exceeded.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra_fields: Vec<ProposalExtraField>,
        /// The username was typed into the form's email field (the form had no username field), so
        /// it is also the person's email: an Address entry made from the form gets it as its email.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        username_is_email: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    AddEntryIdentifier {
        token: String,
        id: String,
        url: String,
        value: String,
        field_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    ProposeSaveTotp {
        token: String,
        url: String,
        otpauth_or_secret: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    StartPairing {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        browser: Option<String>,
    },
    CompletePairing {
        token: String,
        code: String,
    },
    RequestUnlock {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    LockVault {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    OpenApp {
        token: String,
        /// Also start a new entry of this kind in the main window (absent: only show it). The
        /// kinds the bridge cannot create itself (an address, a files item needing the app's own
        /// file dialog); any other value fails to decode, so the request is refused as malformed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_entry: Option<NewEntryKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    OpenBrowserSettings {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    ReportBrowserDiagnostics {
        token: String,
        report: BrowserDiagnosticsReport,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    GetPasskeyProxyStatus {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    SetPasskeyProxyEnabled {
        token: String,
        enabled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    PasskeyCreate {
        token: String,
        request_id: i64,
        request_details_json: String,
        origin_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    PasskeyCreateConfirm {
        token: String,
        request_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry_id: Option<String>,
        #[serde(default, deserialize_with = "null_as_default")]
        replace: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    PasskeyGet {
        token: String,
        request_id: i64,
        request_details_json: String,
        origin_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    PasskeyGetApprove {
        token: String,
        request_id: i64,
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    PasskeyCancel {
        token: String,
        request_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    GeneratePassword {
        token: String,
        domain: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    RecordGeneratedPassword {
        token: String,
        #[serde(default, deserialize_with = "null_as_default")]
        password: String,
        #[serde(default, deserialize_with = "null_as_default")]
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    GetGeneratedPasswordHistory {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    DeleteGeneratedPassword {
        token: String,
        #[serde(default, deserialize_with = "null_as_default")]
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    UnlockWithPassword {
        token: String,
        password: WirePassword,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    UnlockWithDevice {
        token: String,
        /// The popup started this prompt by itself on opening (Touch ID first).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        automatic: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    GetUnlockOptions {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    /// The user clicked a social sign-in button (`provider`: `google`, `facebook`, `apple`,
    /// `microsoft` or `github`) on the https page `pageUrl`: remember it on the matching login.
    /// `source` says how the extension saw it; a missing value is the weaker `frame`.
    RecordSocialSignIn {
        token: String,
        page_url: String,
        provider: String,
        #[serde(default)]
        source: SocialSignInSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
    /// The social sign-in method remembered for the https page `url`, if any.
    GetSocialSignIn {
        token: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
}

/// A response from the app. It never carries the endpoint token.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Response {
    Locked,
    PairingRequired,
    PairingStarted,
    Paired {
        secret: String,
    },
    CredentialList {
        credentials: Vec<ExtensionCredentialSummary>,
    },
    Address {
        address: Box<ExtensionAddress>,
    },
    /// The person and the address to fill with it (none when the person links no live address
    /// and none was picked).
    Person {
        person: Box<ExtensionPerson>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<Box<ExtensionAddress>>,
    },
    Credential {
        username: String,
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        totp: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        urls: Vec<String>,
    },
    Totp {
        code: String,
    },
    SaveProposalAccepted,
    UnlockPresented,
    UnlockNotNeeded,
    Unlocked,
    WrongPassword,
    UnlockThrottled {
        retry_after_secs: u64,
    },
    DeviceUnlockUnavailable,
    /// The device prompt did not recognize the user; `device_failures` counts such prompts across
    /// the app in this lock spell.
    DeviceNotRecognized {
        device_failures: u32,
    },
    UnlockCancelled,
    UnlockInApp,
    UnlockOptions {
        device_unlock: bool,
        touch_id: bool,
        #[serde(default)]
        device_failures: u32,
        #[serde(default)]
        device_blocked: bool,
        #[serde(default)]
        device_prompt_in_flight: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_secs: Option<u64>,
    },
    PasskeyProxyStatus {
        unlocked: bool,
        paired: bool,
        attach_allowed: bool,
        #[serde(default, deserialize_with = "null_as_default")]
        enabled: bool,
        /// The app's "Remember social sign-in" setting, reported on every status poll while the
        /// vault is unlocked, so the extension notices a change without waiting for a sign-in
        /// lookup. Absent while locked: settings are not served then, and the extension keeps the
        /// value it has cached (absent means unknown, not off).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remember_social_sign_in: Option<bool>,
    },
    PasskeyProxyEnabled {
        enabled: bool,
    },
    PasskeyCreatePending {
        request_id: i64,
        rp_id: String,
        rp_name: String,
        matches: Vec<ExtensionCredentialSummary>,
    },
    PasskeyCreateComplete {
        request_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_json: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<PasskeyErrorDetails>,
    },
    PasskeyGetPending {
        request_id: i64,
        rp_id: String,
        matches: Vec<ExtensionCredentialSummary>,
    },
    PasskeyGetComplete {
        request_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_json: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<PasskeyErrorDetails>,
    },
    PasskeyCancelled {
        request_id: i64,
    },
    GeneratedPassword {
        password: String,
    },
    EntryCreated,
    #[serde(rename = "ok")]
    Acknowledged,
    #[serde(rename = "generatedPasswordHistory")]
    PasswordHistory {
        entries: Vec<ExtensionPasswordHistoryItem>,
    },
    /// The answer to both social sign-in requests. `enabled` mirrors the app's "Remember social
    /// sign-in" setting (when it is off nothing is looked up or stored); `provider` and
    /// `lastUsed` (RFC 3339 UTC) are the method remembered for the page, absent when none is;
    /// `host` is the lowercase host the method was recorded on (absent when it was picked in the
    /// app). The extension rings a page's button only when `host` is that page's own host.
    SocialSignIn {
        enabled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_used: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
    Error {
        message: String,
    },
}

/// One credential offered to the extension. It never carries a password, and a person row never
/// carries a person field other than its linked address ids. `Debug` hides the user name, the URL
/// and the identifiers.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCredentialSummary {
    pub id: String,
    pub title: String,
    pub username: String,
    pub has_totp: bool,
    pub has_passkey: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identifiers: Vec<String>,
    /// A person's linked Address entries as wire ids, in the person's order (the first is the
    /// default). Only person browse rows carry it; the ids may name entries that are gone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub address_ids: Vec<String>,
}

impl std::fmt::Debug for ExtensionCredentialSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionCredentialSummary")
            .field("id", &self.id)
            .field("title", &"<redacted>")
            .field("username", &"<redacted>")
            .field("has_totp", &self.has_totp)
            .field("has_passkey", &self.has_passkey)
            .field("url", &self.url.as_ref().map(|_| "<redacted>"))
            .field("entry_type", &self.entry_type)
            .field("identifiers", &self.identifiers.len())
            .field("address_ids", &self.address_ids.len())
            .finish()
    }
}

/// A person entry as the extension fills it into a form. `full_name` is the given and family
/// names joined (the middle name has its own field). `Debug` hides every value and the values are
/// wiped on drop.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPerson {
    pub id: String,
    pub label: String,
    pub full_name: String,
    pub given_name: String,
    pub middle_name: String,
    pub family_name: String,
    pub email: String,
    /// ISO `YYYY-MM-DD`, or empty.
    pub birth_date: String,
}

/// An address entry as the extension fills it into a form.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionAddress {
    pub id: String,
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

/// One recorded generated password. `Debug` redacts the password.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPasswordHistoryItem {
    pub id: String,
    pub password: String,
    pub url: String,
    pub created_at: f64,
}

/// A WebAuthn error the extension rethrows into the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasskeyErrorDetails {
    pub name: String,
    pub message: String,
}

/// What the extension reports about the browser's own password manager and proxy state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDiagnosticsReport {
    pub proxy_state: ProxyState,
    pub permission_state: PermissionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_saving_enabled: Option<bool>,
    pub extension_scan_state: ExtensionScanState,
    pub enabled_managers: Vec<String>,
}

/// Most manager names a report may carry.
const MAX_REPORTED_MANAGERS: usize = 20;
/// Longest manager name, in characters.
const MAX_MANAGER_NAME_CHARS: usize = 120;

/// Unicode format characters (category Cf): soft hyphen, Arabic number signs, zero-width and
/// bidirectional marks and overrides, word joiner and invisible operators, BOM, interlinear
/// annotation, musical and tag characters. They render as nothing or reorder surrounding text.
fn is_format_char(c: char) -> bool {
    matches!(u32::from(c),
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

/// Control (Cc) or format (Cf) characters, which no name or label shown in the UI may contain.
///
/// `mypass_core::text::is_unsafe_text_char` is the same rule for the app side (the host must not
/// link mypass-core); `handlers::text::tests::host_and_core_unsafe_text_rules_agree_on_every_char`
/// pins that the two agree, so change both together.
#[must_use]
pub fn is_unsafe_text_char(c: char) -> bool {
    c.is_control() || is_format_char(c)
}

impl BrowserDiagnosticsReport {
    /// Bounds and consistency rules of the Swift app: a hostile or buggy extension cannot push
    /// oversized, control-character or self-contradicting reports into the UI.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.enabled_managers.len() <= MAX_REPORTED_MANAGERS
            && self.enabled_managers.iter().all(|name| {
                !name.trim().is_empty()
                    && name.chars().count() <= MAX_MANAGER_NAME_CHARS
                    && !name.chars().any(is_unsafe_text_char)
            })
            && (self.extension_scan_state == ExtensionScanState::Complete
                || self.enabled_managers.is_empty())
            && (self.permission_state == PermissionState::Granted
                || (self.password_saving_enabled.is_none()
                    && self.extension_scan_state == ExtensionScanState::NotChecked))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyState {
    Active,
    Disabled,
    Locked,
    Unpaired,
    Unavailable,
    AttachmentError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionState {
    Granted,
    Unavailable,
    NotGranted,
}

/// The entry kinds `openApp` can ask the main window to start (`newEntry`). Spelled as the
/// extension's entry types (`"Address"`, `"Files"`); deliberately only the kinds that
/// `createEntry` refuses, so the popup creates every other kind itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NewEntryKind {
    Address,
    Files,
}

/// What a value captured from a registration form is. The app names the custom field after the
/// kind (in the UI language); only `Other` uses the label the page gave the field. An unknown kind
/// fails to decode, so the proposal is refused as malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtraFieldKind {
    Name,
    GivenName,
    FamilyName,
    Email,
    Phone,
    Birthday,
    Username,
    Organization,
    AddressLine1,
    AddressLine2,
    City,
    Region,
    PostalCode,
    Country,
    Other,
}

impl ExtraFieldKind {
    /// A part of a postal address (what makes a form an address form), not a name or contact value.
    #[must_use]
    pub fn is_address_part(self) -> bool {
        matches!(
            self,
            Self::AddressLine1
                | Self::AddressLine2
                | Self::City
                | Self::Region
                | Self::PostalCode
                | Self::Country
        )
    }
}

/// One extra value of a registration form, next to a proposal's username and password. `Debug`
/// hides the label and the value; the value is wiped on drop.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalExtraField {
    pub kind: ExtraFieldKind,
    /// The field's readable label on the page (for `Other`, the name it gets in the entry).
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionScanState {
    Complete,
    Unavailable,
    NotChecked,
}

/// Swift decodes the defaulted fields with `decodeIfPresent ?? default`, so an explicit JSON `null`
/// is the default too; serde's plain `default` would reject it.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// The message tag in `PascalCase` (`fillCredential` -> `FillCredential`).
fn message_tag<T: Serialize>(value: &T) -> String {
    let tag = serde_json::to_value(value)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str().map(str::to_owned)))
        .unwrap_or_default();
    pascal_case(&tag)
}

fn pascal_case(tag: &str) -> String {
    let mut chars = tag.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Unknown".to_owned(),
    }
}

// `Debug` is an allow-list of exactly one thing, the message tag: no field value is ever printed, so
// a new field can never leak through a log line.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Request::{} {{ .. }}", pascal_case(self.name()))
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Response::{} {{ .. }}", message_tag(self))
    }
}

impl fmt::Debug for ProposalExtraField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProposalExtraField")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl Drop for ProposalExtraField {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.value);
    }
}

impl fmt::Debug for ExtensionPerson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionPerson").finish_non_exhaustive()
    }
}

impl Drop for ExtensionPerson {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.label.zeroize();
        self.full_name.zeroize();
        self.given_name.zeroize();
        self.middle_name.zeroize();
        self.family_name.zeroize();
        self.email.zeroize();
        self.birth_date.zeroize();
    }
}

impl fmt::Debug for ExtensionAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionAddress").finish_non_exhaustive()
    }
}

/// The history password is wiped when the item drops, so the buffer the handler moved out of its
/// wiping wrapper is still wiped once the response has been serialised and discarded.
impl Drop for ExtensionPasswordHistoryItem {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.password);
    }
}

impl fmt::Debug for ExtensionPasswordHistoryItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionPasswordHistoryItem")
            .field("id", &self.id)
            .field("password", &Redacted)
            .field("url", &self.url)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)] // test-only
mod unlock_protocol_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unlock_requests_round_trip_with_camel_case_names() {
        for name in ["unlockWithPassword", "unlockWithDevice", "getUnlockOptions"] {
            let mut wire = json!({"type": name, "token": "t", "pairing": "p"});
            if name == "unlockWithPassword" {
                wire["password"] = json!("p\u{e9}");
            }
            let request: Request = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(request.name(), name);
            assert_eq!(serde_json::to_value(request).unwrap(), wire);
        }
    }

    #[test]
    fn unlock_with_device_reads_the_optional_automatic_flag() {
        let explicit: Request =
            serde_json::from_value(json!({"type": "unlockWithDevice", "token": "t"})).unwrap();
        assert!(matches!(
            explicit,
            Request::UnlockWithDevice {
                automatic: false,
                ..
            }
        ));
        let wire = json!({"type": "unlockWithDevice", "token": "t", "automatic": true});
        let automatic: Request = serde_json::from_value(wire.clone()).unwrap();
        assert!(matches!(
            automatic,
            Request::UnlockWithDevice {
                automatic: true,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(automatic).unwrap(), wire);
    }

    #[test]
    fn debug_of_unlock_with_password_never_contains_the_password() {
        let sentinel = "hunter2-\u{1f511}";
        let request = Request::UnlockWithPassword {
            token: "t".into(),
            password: WirePassword::new(sentinel.into()),
            pairing: Some("pairing secret".into()),
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("UnlockWithPassword"));
        assert!(!debug.contains(sentinel));
        assert!(!debug.contains("pairing secret"));
    }

    #[test]
    fn wire_password_debug_is_redacted() {
        assert_eq!(
            format!("{:?}", WirePassword::new("test secret".into())),
            "WirePassword(<redacted>)"
        );
    }

    #[test]
    fn unlock_responses_serialise_to_the_spec_shapes() {
        let cases = [
            (Response::Unlocked, json!({"type": "unlocked"})),
            (Response::WrongPassword, json!({"type": "wrongPassword"})),
            (
                Response::UnlockThrottled {
                    retry_after_secs: 30,
                },
                json!({"type": "unlockThrottled", "retryAfterSecs": 30}),
            ),
            (
                Response::DeviceUnlockUnavailable,
                json!({"type": "deviceUnlockUnavailable"}),
            ),
            (
                Response::DeviceNotRecognized { device_failures: 2 },
                json!({"type": "deviceNotRecognized", "deviceFailures": 2}),
            ),
            (
                Response::UnlockCancelled,
                json!({"type": "unlockCancelled"}),
            ),
            (Response::UnlockInApp, json!({"type": "unlockInApp"})),
            (
                Response::UnlockOptions {
                    device_unlock: true,
                    touch_id: true,
                    device_failures: 1,
                    device_blocked: true,
                    device_prompt_in_flight: true,
                    retry_after_secs: None,
                },
                json!({"type": "unlockOptions", "deviceUnlock": true, "touchId": true,
                    "deviceFailures": 1, "deviceBlocked": true, "devicePromptInFlight": true}),
            ),
        ];
        for (response, wire) in cases {
            assert_eq!(serde_json::to_value(response).unwrap(), wire);
        }
    }

    #[test]
    fn request_types_lists_34_names_ending_with_the_social_sign_in_requests() {
        assert_eq!(REQUEST_TYPES.len(), 35);
        assert!(REQUEST_TYPES.ends_with(&[
            "unlockWithPassword",
            "unlockWithDevice",
            "getUnlockOptions",
            "recordSocialSignIn",
            "getSocialSignIn",
        ]));
    }

    #[test]
    fn social_sign_in_messages_use_the_spec_shapes() {
        let wire = json!({"type": "recordSocialSignIn", "token": "t",
            "pageUrl": "https://example.com/login", "provider": "google", "source": "click",
            "pairing": "p"});
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(request.name(), "recordSocialSignIn");
        assert_eq!(serde_json::to_value(request).unwrap(), wire);
        // A missing source is the weaker frame heuristic; an unknown one is malformed.
        let request: Request = serde_json::from_value(json!({"type": "recordSocialSignIn",
            "token": "t", "pageUrl": "https://example.com/", "provider": "google"}))
        .unwrap();
        assert!(matches!(
            request,
            Request::RecordSocialSignIn {
                source: SocialSignInSource::Frame,
                ..
            }
        ));
        assert!(
            serde_json::from_value::<Request>(json!({"type": "recordSocialSignIn",
            "token": "t", "pageUrl": "https://example.com/", "provider": "google",
            "source": "script"}))
            .is_err()
        );
        let wire = json!({"type": "getSocialSignIn", "token": "t", "url": "https://example.com/"});
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), wire);
        assert_eq!(
            serde_json::to_value(Response::SocialSignIn {
                enabled: true,
                provider: Some("github".into()),
                last_used: Some("2026-11-22T10:00:00Z".into()),
                host: Some("github.com".into()),
            })
            .unwrap(),
            json!({"type": "socialSignIn", "enabled": true, "provider": "github",
                "lastUsed": "2026-11-22T10:00:00Z", "host": "github.com"})
        );
        assert_eq!(
            serde_json::to_value(Response::SocialSignIn {
                enabled: false,
                provider: None,
                last_used: None,
                host: None,
            })
            .unwrap(),
            json!({"type": "socialSignIn", "enabled": false})
        );
        // A missing provider is malformed.
        assert!(serde_json::from_value::<Request>(
            json!({"type": "recordSocialSignIn", "token": "t", "pageUrl": "https://a.example"})
        )
        .is_err());
    }
}
