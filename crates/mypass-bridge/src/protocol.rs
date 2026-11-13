//! The wire protocol between the extension (through the native host) and the app.
//!
//! Every frame is one JSON object tagged by `type`. The shapes are byte-compatible with the earlier
//! Swift app, so the extension keeps working unchanged: camelCase names, optional fields omitted
//! (never `null`), and the Swift decoding defaults kept as `serde(default)`.
//!
//! Secrets never reach `Debug` output: `Debug` for the requests, the responses and the history item
//! goes through the JSON form and redacts every secret-bearing key.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The wire `type` of every [`Request`] variant, in declaration order.
pub const REQUEST_TYPES: [&str; 29] = [
    "getCredentials",
    "getFavorites",
    "listEntries",
    "searchEntries",
    "createEntry",
    "fillAddress",
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
];

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
        #[serde(default)]
        title: String,
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        card_number: String,
        #[serde(default)]
        cardholder_name: String,
        #[serde(default)]
        expiry: String,
        #[serde(default)]
        cvv: String,
        #[serde(default)]
        notes: String,
        #[serde(default)]
        favorite: bool,
        #[serde(default, rename = "match")]
        r#match: String,
        #[serde(default)]
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
        #[serde(default)]
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
        #[serde(default)]
        password: String,
        #[serde(default)]
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
        #[serde(default)]
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pairing: Option<String>,
    },
}

/// A response from the app. It never carries the endpoint token.
// The address variant is large, but responses are short-lived values built per request; boxing it
// would change the public shape the handlers construct.
#[allow(clippy::large_enum_variant)]
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
        address: ExtensionAddress,
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
    PasskeyProxyStatus {
        unlocked: bool,
        paired: bool,
        attach_allowed: bool,
        #[serde(default)]
        enabled: bool,
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
    Error {
        message: String,
    },
}

/// One credential offered to the extension. It never carries a password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

/// An address entry as the extension fills it into a form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionScanState {
    Complete,
    Unavailable,
    NotChecked,
}

/// JSON keys whose values are secrets or vault content and never appear in `Debug` output.
fn is_secret_key(key: &str) -> bool {
    matches!(
        key,
        "token"
            | "pairing"
            | "password"
            | "value"
            | "secret"
            | "totp"
            | "code"
            | "cvv"
            | "cardNumber"
            | "notes"
            | "requestDetailsJson"
            | "responseJson"
    ) || key.starts_with("otp")
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Replaces the value of every secret key, at any depth, with a marker string.
fn redact(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_secret_key(key) {
                    *val = serde_json::Value::String("<redacted>".to_owned());
                } else {
                    redact(val);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

/// Prints a serialisable value as `Tag { key: value, .. }` with the secret keys redacted.
fn debug_redacted<T: Serialize>(value: &T, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let Ok(mut json) = serde_json::to_value(value) else {
        return f.write_str("<unprintable>");
    };
    redact(&mut json);
    let serde_json::Value::Object(map) = json else {
        return f.write_str("<unprintable>");
    };
    let tag = map.get("type").and_then(|t| t.as_str()).unwrap_or("Item");
    let mut out = f.debug_struct(tag);
    for (key, val) in &map {
        if key == "type" {
            continue;
        }
        out.field(key, val);
    }
    out.finish()
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_redacted(self, f)
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_redacted(self, f)
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
