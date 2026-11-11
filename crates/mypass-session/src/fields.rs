//! Field-name conventions of a MyPass entry and the accessors built on them.
//!
//! The names are a contract with the earlier Swift app and with KeePassXC (the `KPEX_PASSKEY_*`
//! keys), so they never change. Protection flags for the app's own fields are pinned where the
//! fields are written (the edit path), not here.

use mypass_core::TotpConfig;
use mypass_kdbx::Entry;

pub const FAVORITE: &str = "Favorite";
pub const OTP: &str = "otp";
pub const CARD_NUMBER: &str = "MyPass.CardNumber";
pub const CARD_HOLDER: &str = "MyPass.CardholderName";
pub const CARD_EXPIRY: &str = "MyPass.ExpiryDate";
pub const CARD_CVV: &str = "MyPass.CVV";
pub const NOTES_BODY: &str = "MyPass.NotesBody";
pub const ADDRESS_PREFIX: &str = "MyPass.Address.";
pub const PASSKEY_PREFIX: &str = "KPEX_PASSKEY_";
pub const PASSKEY_CREDENTIAL_ID: &str = "KPEX_PASSKEY_CREDENTIAL_ID";
pub const PASSKEY_PRIVATE_KEY_PEM: &str = "KPEX_PASSKEY_PRIVATE_KEY_PEM";
pub const PASSKEY_RELYING_PARTY: &str = "KPEX_PASSKEY_RELYING_PARTY";
pub const PASSKEY_USERNAME: &str = "KPEX_PASSKEY_USERNAME";
/// MyPass's own addition (an ISO 8601 timestamp); KeePassXC ignores it.
pub const PASSKEY_CREATED: &str = "KPEX_PASSKEY_CREATED";
/// Group custom-data key holding a group's emoji.
pub const GROUP_EMOJI: &str = "MyPass.GroupEmoji";
/// Group custom-data key that marks the generator history group (besides its fixed UUID).
pub const GENERATOR_HISTORY_MARKER: &str = "MyPass.GeneratorHistory";

pub const ADDRESS_FIELDS: [&str; 14] = [
    "label",
    "fullName",
    "givenName",
    "familyName",
    "organization",
    "addressLine1",
    "addressLine2",
    "city",
    "region",
    "postalCode",
    "countryCode",
    "countryName",
    "phone",
    "email",
];
pub const ADDRESS_LABELS: [&str; 3] = ["Home", "Work", "Other"];

/// Keys with a dedicated UI that must never appear as generic custom-field rows. The `KPEX_*`
/// keys are matched by prefix, `MyPass.Address.*` by the exact 14 keys, like the Swift set.
const STANDARD_KEYS: [&str; 14] = [
    "Title",
    "UserName",
    "Password",
    "URL",
    "Notes",
    OTP,
    FAVORITE,
    mypass_core::entry_ext::ENTRY_TYPE_FIELD,
    CARD_NUMBER,
    CARD_HOLDER,
    CARD_EXPIRY,
    CARD_CVV,
    NOTES_BODY,
    mypass_core::entry_ext::URI_MATCH_MODE_FIELD,
];

/// MyPass field conventions on a KeePass entry.
pub trait EntryFields {
    fn is_favorite(&self) -> bool;
    /// Sets the marker (`Favorite=1`, unprotected) or removes the field.
    fn set_favorite(&mut self, v: bool);
    /// The non-empty `otp` field: a base32 secret or an `otpauth://` URL.
    fn otp_source(&self) -> Option<&str>;
    /// The parsed TOTP configuration, `None` when absent or unparseable.
    fn totp(&self) -> Option<TotpConfig>;
    /// A usable passkey: both the credential id and the private key are non-empty. This is the
    /// one definition; the app, the bridge and the passkey stage all call it.
    fn has_passkey(&self) -> bool;
    /// Whether `key` has a dedicated UI (or is a managed marker) and so is not a custom field.
    fn is_reserved_key(key: &str) -> bool;
}

impl EntryFields for Entry {
    fn is_favorite(&self) -> bool {
        self.fields.iter().any(|f| f.key == FAVORITE)
    }

    fn set_favorite(&mut self, v: bool) {
        if v {
            self.set_field(FAVORITE, "1", false);
        } else {
            self.remove_field(FAVORITE);
        }
    }

    fn otp_source(&self) -> Option<&str> {
        self.field(OTP).filter(|s| !s.is_empty())
    }

    fn totp(&self) -> Option<TotpConfig> {
        self.otp_source().and_then(TotpConfig::parse)
    }

    fn has_passkey(&self) -> bool {
        let non_empty = |key: &str| self.field(key).is_some_and(|v| !v.is_empty());
        non_empty(PASSKEY_CREDENTIAL_ID) && non_empty(PASSKEY_PRIVATE_KEY_PEM)
    }

    fn is_reserved_key(key: &str) -> bool {
        STANDARD_KEYS.contains(&key)
            || key.starts_with(mypass_core::entry_ext::ADDITIONAL_URLS_KEY)
            || key.starts_with(PASSKEY_PREFIX)
            || key
                .strip_prefix(ADDRESS_PREFIX)
                .is_some_and(|name| ADDRESS_FIELDS.contains(&name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypass_kdbx::Timestamp;

    #[test]
    fn favorite_round_trips_and_is_unprotected() {
        let mut e = Entry::new(Timestamp(0));
        assert!(!e.is_favorite());
        e.set_favorite(true);
        assert!(e.is_favorite());
        assert!(e.fields.iter().all(|f| f.key != FAVORITE || !f.protected));
        e.set_favorite(false);
        assert!(!e.is_favorite());
        assert!(e.field(FAVORITE).is_none());
    }

    #[test]
    fn otp_source_ignores_empty_and_totp_parses() {
        let mut e = Entry::new(Timestamp(0));
        assert!(e.otp_source().is_none() && e.totp().is_none());
        e.set_field(OTP, "", true);
        assert!(e.otp_source().is_none());
        e.set_field(OTP, "JBSWY3DPEHPK3PXP", true);
        assert_eq!(e.otp_source(), Some("JBSWY3DPEHPK3PXP"));
        assert_eq!(e.totp().map(|t| (t.digits, t.period)), Some((6, 30)));
        e.set_field(OTP, "not base32 !!", true);
        assert!(e.otp_source().is_some() && e.totp().is_none());
    }
}
