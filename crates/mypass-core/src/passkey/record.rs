//! The `KPEX_PASSKEY_*` entry fields (a contract with KeePassXC and the earlier Swift app, so the
//! names never change), the one definition of "this entry carries a usable passkey", and
//! [`PasskeyRecord`], the codec between those fields and a typed passkey.
//!
//! Field conventions (KeePassXC's, kept by the Swift app):
//!
//! | Key | Value | Protected |
//! |---|---|---|
//! | `KPEX_PASSKEY_RELYING_PARTY` | the canonical rpId | no |
//! | `KPEX_PASSKEY_CREDENTIAL_ID` | unpadded base64url | yes |
//! | `KPEX_PASSKEY_PRIVATE_KEY_PEM` | PKCS#8 PEM (`BEGIN PRIVATE KEY`) | yes |
//! | `KPEX_PASSKEY_USER_HANDLE` | unpadded base64url | yes |
//! | `KPEX_PASSKEY_USERNAME` | the account name | no |
//! | `KPEX_PASSKEY_FLAG_BE`, `KPEX_PASSKEY_FLAG_BS` | `"1"` or `"0"` | no |
//! | `KPEX_PASSKEY_CREATED` | `YYYY-MM-DDTHH:MM:SSZ` (MyPass only; KeePassXC ignores it) | no |
//!
//! Reading is lenient (padded or standard-alphabet base64, SEC1 PEM, missing optional fields), so a
//! passkey another tool wrote never silently disappears.

use super::b64url;
use super::ceremony::BackupFlags;
use mypass_kdbx::{Entry, Timestamp};
use std::fmt;
use zeroize::Zeroizing;

/// Prefix shared by every passkey field.
pub const PASSKEY_PREFIX: &str = "KPEX_PASSKEY_";
/// Credential id, base64url.
pub const PASSKEY_CREDENTIAL_ID: &str = "KPEX_PASSKEY_CREDENTIAL_ID";
/// PKCS#8 PEM private key (protected).
pub const PASSKEY_PRIVATE_KEY_PEM: &str = "KPEX_PASSKEY_PRIVATE_KEY_PEM";
/// Relying party id.
pub const PASSKEY_RELYING_PARTY: &str = "KPEX_PASSKEY_RELYING_PARTY";
/// Account user name shown for the passkey.
pub const PASSKEY_USERNAME: &str = "KPEX_PASSKEY_USERNAME";
/// MyPass's own addition (an ISO 8601 timestamp); KeePassXC ignores it.
pub const PASSKEY_CREATED: &str = "KPEX_PASSKEY_CREATED";
/// User handle, base64url.
pub const PASSKEY_USER_HANDLE: &str = "KPEX_PASSKEY_USER_HANDLE";
/// Backup-eligible flag.
pub const PASSKEY_FLAG_BE: &str = "KPEX_PASSKEY_FLAG_BE";
/// Backup-state flag.
pub const PASSKEY_FLAG_BS: &str = "KPEX_PASSKEY_FLAG_BS";

/// Every field [`PasskeyRecord::write`] manages, in the Swift app's order.
pub const KEYS: [&str; 8] = [
    PASSKEY_RELYING_PARTY,
    PASSKEY_CREDENTIAL_ID,
    PASSKEY_PRIVATE_KEY_PEM,
    PASSKEY_USER_HANDLE,
    PASSKEY_USERNAME,
    PASSKEY_FLAG_BE,
    PASSKEY_FLAG_BS,
    PASSKEY_CREATED,
];

/// A usable passkey: the credential id decodes (lenient base64url) to at least one byte and the
/// private key PEM is non-empty. The PEM is deliberately not parsed here, so listing stays cheap
/// and an unreadable key still shows as a passkey the user can inspect or delete.
pub fn has_passkey(entry: &Entry) -> bool {
    PasskeyRecord::credential_id(entry).is_some_and(|id| !id.is_empty())
        && PasskeyRecord::has_private_key(entry)
}

/// A passkey as stored on an entry. `Debug` never prints the private key or the user handle.
#[derive(Clone, PartialEq, Eq)]
pub struct PasskeyRecord {
    /// The canonical rpId (`KPEX_PASSKEY_RELYING_PARTY`); empty when the field is missing.
    pub rp_id: String,
    /// The credential id bytes.
    pub credential_id: Vec<u8>,
    /// The private key PEM, PKCS#8 when MyPass wrote it (SEC1 is accepted on read).
    pub private_key_pem: Zeroizing<String>,
    /// The WebAuthn user handle, when stored and decodable.
    pub user_handle: Option<Vec<u8>>,
    /// The account name shown for the passkey.
    pub username: Option<String>,
    /// When MyPass created the passkey; `None` for one created elsewhere.
    pub created: Option<Timestamp>,
    /// The backup flags; `1/1` when absent or unparsable.
    pub flags: BackupFlags,
}

impl fmt::Debug for PasskeyRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasskeyRecord")
            .field("rp_id", &self.rp_id)
            .field("credential_id_len", &self.credential_id.len())
            .field("private_key_pem", &"<redacted>")
            .field(
                "user_handle",
                &self.user_handle.as_ref().map(|_| "<redacted>"),
            )
            .field("username", &self.username.as_ref().map(|_| "<redacted>"))
            .field("created", &self.created)
            .field("flags", &self.flags)
            .finish()
    }
}

impl PasskeyRecord {
    /// The passkey on `e`, or `None` unless [`has_passkey`] holds. Optional fields that are
    /// missing or unreadable read as `None` (the user handle) or their default (the flags).
    #[must_use]
    pub fn read(e: &Entry) -> Option<PasskeyRecord> {
        if !has_passkey(e) {
            return None;
        }
        Some(PasskeyRecord {
            rp_id: e
                .field(PASSKEY_RELYING_PARTY)
                .unwrap_or_default()
                .to_owned(),
            credential_id: Self::credential_id(e)?,
            private_key_pem: Zeroizing::new(e.field(PASSKEY_PRIVATE_KEY_PEM)?.to_owned()),
            user_handle: e
                .field(PASSKEY_USER_HANDLE)
                .and_then(b64url::decode)
                .filter(|h| !h.is_empty()),
            username: e.field(PASSKEY_USERNAME).map(str::to_owned),
            created: e.field(PASSKEY_CREATED).and_then(parse_created),
            flags: BackupFlags {
                eligible: parse_flag(e.field(PASSKEY_FLAG_BE)),
                state: parse_flag(e.field(PASSKEY_FLAG_BS)),
            },
        })
    }

    /// Store this passkey on `e`. RELYING_PARTY, CREDENTIAL_ID, PRIVATE_KEY_PEM, FLAG_BE and
    /// FLAG_BS are always set; USER_HANDLE, USERNAME and CREATED are set when `Some` and removed
    /// when `None` (so a replace never leaves a stale user handle). CREDENTIAL_ID, PRIVATE_KEY_PEM
    /// and USER_HANDLE are protected, the rest are plain. Other `KPEX_PASSKEY_*` fields, ones a
    /// newer KeePassXC may add, are left as they are.
    pub fn write(&self, e: &mut Entry) {
        e.set_field(PASSKEY_RELYING_PARTY, &self.rp_id, false);
        e.set_field(
            PASSKEY_CREDENTIAL_ID,
            &b64url::encode(&self.credential_id),
            true,
        );
        e.set_field(PASSKEY_PRIVATE_KEY_PEM, &self.private_key_pem, true);
        match &self.user_handle {
            Some(h) => e.set_field(PASSKEY_USER_HANDLE, &b64url::encode(h), true),
            None => e.remove_field(PASSKEY_USER_HANDLE),
        }
        match &self.username {
            Some(name) => e.set_field(PASSKEY_USERNAME, name, false),
            None => e.remove_field(PASSKEY_USERNAME),
        }
        e.set_field(PASSKEY_FLAG_BE, flag(self.flags.eligible), false);
        e.set_field(PASSKEY_FLAG_BS, flag(self.flags.state), false);
        match self.created.and_then(format_created) {
            Some(created) => e.set_field(PASSKEY_CREATED, &created, false),
            None => e.remove_field(PASSKEY_CREATED),
        }
    }

    /// Remove every field whose key starts with `KPEX_PASSKEY_`, not only the eight above, so a
    /// field another tool added is not left orphaned and invisible after the passkey is removed.
    pub fn clear(e: &mut Entry) {
        e.fields.retain(|f| !f.key.starts_with(PASSKEY_PREFIX));
    }

    /// See [`has_passkey`].
    #[must_use]
    pub fn has_passkey(e: &Entry) -> bool {
        has_passkey(e)
    }

    /// The private key PEM is non-empty: the Swift "existing passkey" test that guards a replace.
    ///
    /// This is weaker than [`has_passkey`]: an entry with a PEM but an undecodable credential id
    /// does not count as a passkey (the popup offers "Save"), yet confirm still requires
    /// `replace` before overwriting its key. Kept for Swift parity: the user can retry with
    /// replace, and nothing is overwritten without consent.
    #[must_use]
    pub fn has_private_key(e: &Entry) -> bool {
        e.field(PASSKEY_PRIVATE_KEY_PEM)
            .is_some_and(|v| !v.is_empty())
    }

    /// The stored relying party id, if any.
    #[must_use]
    pub fn relying_party(e: &Entry) -> Option<&str> {
        e.field(PASSKEY_RELYING_PARTY)
    }

    /// The stored credential id, decoded leniently; `None` when absent or not base64.
    #[must_use]
    pub fn credential_id(e: &Entry) -> Option<Vec<u8>> {
        e.field(PASSKEY_CREDENTIAL_ID).and_then(b64url::decode)
    }
}

/// `KPEX_PASSKEY_CREATED` text for `t` (`YYYY-MM-DDTHH:MM:SSZ`, what the Swift app's
/// `ISO8601DateFormatter` wrote); `None` outside years 1 to 9999.
#[must_use]
pub fn format_created(t: Timestamp) -> Option<String> {
    t.to_iso8601()
}

/// Parse `KPEX_PASSKEY_CREATED`: `YYYY-MM-DDTHH:MM:SSZ`, optionally with fractional seconds
/// (dropped) and with `+00:00` in place of `Z`. Anything else, including other offsets and
/// impossible dates, is `None`.
#[must_use]
pub fn parse_created(s: &str) -> Option<Timestamp> {
    let body = s.strip_suffix('Z').or_else(|| s.strip_suffix("+00:00"))?;
    let whole = body.get(..19)?;
    let fraction = body.get(19..)?;
    if let Some(digits) = fraction.strip_prefix('.') {
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    } else if !fraction.is_empty() {
        return None;
    }
    Timestamp::parse_iso8601(&format!("{whole}Z"))
}

/// A stored backup flag: `"1"` / `"true"` and `"0"` / `"false"`; absent or anything else is the
/// default `true`.
fn parse_flag(value: Option<&str>) -> bool {
    match value.map(str::trim) {
        Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
        _ => true,
    }
}

fn flag(on: bool) -> &'static str {
    if on {
        "1"
    } else {
        "0"
    }
}

#[cfg(test)]
mod tests;
