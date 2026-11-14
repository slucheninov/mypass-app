//! Field-name conventions of a MyPass entry and the accessors built on them.
//!
//! The names are a contract with the earlier Swift app and with KeePassXC (the `KPEX_PASSKEY_*`
//! keys), so they never change. Protection flags for the app's own fields are pinned where the
//! fields are written (the edit path), not here.

use crate::session::SessionError;
use mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID;
use mypass_core::TotpConfig;
use mypass_kdbx::{Database, Entry, Group};
use uuid::Uuid;

pub const FAVORITE: &str = "Favorite";
pub const OTP: &str = "otp";
pub const CARD_NUMBER: &str = "MyPass.CardNumber";
pub const CARD_HOLDER: &str = "MyPass.CardholderName";
pub const CARD_EXPIRY: &str = "MyPass.ExpiryDate";
pub const CARD_CVV: &str = "MyPass.CVV";
pub const NOTES_BODY: &str = "MyPass.NotesBody";
pub const ADDRESS_PREFIX: &str = "MyPass.Address.";
// The passkey field names and the `has_passkey` predicate are owned by `mypass_core::passkey`.
pub use mypass_core::passkey::record::{
    PASSKEY_CREATED, PASSKEY_CREDENTIAL_ID, PASSKEY_FLAG_BE, PASSKEY_FLAG_BS, PASSKEY_PREFIX,
    PASSKEY_PRIVATE_KEY_PEM, PASSKEY_RELYING_PARTY, PASSKEY_USERNAME, PASSKEY_USER_HANDLE,
};
/// Group custom-data key holding a group's emoji.
pub const GROUP_EMOJI: &str = "MyPass.GroupEmoji";
/// Group custom-data key that marks the generator history group (besides its fixed UUID).
pub const GENERATOR_HISTORY_MARKER: &str = "MyPass.GeneratorHistory";

/// Every entry id in the subtree of `group` (the drafts to clear when the group goes away).
pub fn entry_ids_under(db: &Database, group: Uuid) -> Vec<Uuid> {
    let mut ids = Vec::new();
    let mut stack: Vec<&Group> = db.find_group(group).into_iter().collect();
    while let Some(g) = stack.pop() {
        ids.extend(g.entries.iter().map(|e| e.uuid));
        stack.extend(g.groups.iter());
    }
    ids
}

/// The one definition of the recycle bin: `None` when no bin is configured, when its id is the
/// root group (another client, corrupted meta) or when that group does not exist. Mirrors the
/// kdbx layer, so `Database::is_in_recycle_bin`, deletes and the session's views agree.
pub fn bin_id(db: &Database) -> Option<Uuid> {
    db.meta
        .recycle_bin_uuid
        .filter(|id| *id != db.root_id())
        .filter(|id| db.find_group(*id).is_some())
}

/// The one definition of the generator history group: its fixed UUID or the
/// [`GENERATOR_HISTORY_MARKER`] custom-data key.
pub fn is_history_group(group: &Group) -> bool {
    group.uuid == GENERATOR_HISTORY_GROUP_UUID || group.custom(GENERATOR_HISTORY_MARKER).is_some()
}

/// Finds `id` below `group` and reports `(in_bin, in_history)`: the group is, or lies inside, the
/// bin / a history group. The root itself is never either.
fn locate(
    group: &Group,
    id: Uuid,
    bin: Option<Uuid>,
    (in_bin, in_history): (bool, bool),
    is_root: bool,
) -> Option<(bool, bool)> {
    let flags = (
        in_bin || (!is_root && bin == Some(group.uuid)),
        in_history || (!is_root && is_history_group(group)),
    );
    if group.uuid == id {
        return Some(flags);
    }
    group
        .groups
        .iter()
        .find_map(|child| locate(child, id, bin, flags, false))
}

fn classify(db: &Database, id: Uuid) -> Result<(bool, bool), SessionError> {
    locate(&db.root, id, bin_id(db), (false, false), true).ok_or(SessionError::NotFound)
}

/// `id` may receive user-written content (entries, subgroups) without being the generator history
/// tree: it exists (`NotFound` otherwise) and is neither a history group nor inside one
/// (`InvalidInput`). The recycle bin is allowed: moving there is the caller's business.
pub fn require_not_history(db: &Database, id: Uuid) -> Result<(), SessionError> {
    match classify(db, id)? {
        (_, true) => Err(SessionError::InvalidInput(
            "the generator history group cannot be used as a target".into(),
        )),
        _ => Ok(()),
    }
}

/// [`require_not_history`] and additionally not the recycle bin or inside it: a place the user
/// can put new content (CSV import).
pub fn is_valid_user_target(db: &Database, id: Uuid) -> Result<(), SessionError> {
    match classify(db, id)? {
        (false, false) => Ok(()),
        _ => Err(SessionError::InvalidInput(
            "this group cannot receive new content".into(),
        )),
    }
}

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
    /// A usable passkey, delegating to `mypass_core::passkey::has_passkey`, the one definition
    /// the app, the bridge and the passkey stage all share.
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
        mypass_core::passkey::has_passkey(self)
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
