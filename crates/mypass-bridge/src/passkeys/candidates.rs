//! Which entries a passkey ceremony may touch. Pure functions over a `&Database`: none of them
//! touches the session, so phase one calls them inside its `with_db` read and confirm calls them
//! again inside its one `mutate`, against the vault as it is at that moment.
//!
//! Domain matching is `matcher::credential_candidates` (https only, PSL-aware, never-mode, bin,
//! generator history, Address and Files excluded); the relying-party lookup applies the same
//! exclusions itself, because it goes by the stored rpId and not through the URL matcher.
//! Relying party ids are compared as `psl::canonical_host` on both sides (R13): KeePassXC and
//! other tools may have stored `Example.COM`.

use mypass_core::entry_ext::{EntryExt, EntryType, UriMatchMode, GENERATOR_HISTORY_GROUP_UUID};
use mypass_core::matcher::credential_candidates;
use mypass_core::passkey::{has_passkey, PasskeyRecord};
use mypass_core::psl::canonical_host;
use mypass_kdbx::{Database, Entry};

/// Whether the rpId stored on `entry` is `rp_id`, both compared in canonical form. An entry
/// without a stored rpId, or with one that does not canonicalise, never matches.
fn stored_rp_is(entry: &Entry, rp_id: &str) -> bool {
    let Some(wanted) = canonical_host(rp_id) else {
        return false;
    };
    PasskeyRecord::relying_party(entry)
        .and_then(canonical_host)
        .is_some_and(|stored| stored == wanted)
}

/// Entries carrying a passkey (the R1 predicate) for `rp_id`. Never offered: Address and Files
/// entries (app-only, even if one somehow carries passkey fields), never-mode entries (the user
/// told MyPass not to offer the entry to the browser), the recycle bin and the generator history
/// group. Document order.
#[must_use]
pub fn passkey_entries<'a>(db: &'a Database, rp_id: &str) -> Vec<&'a Entry> {
    db.entries_excluding(&[GENERATOR_HISTORY_GROUP_UUID])
        .into_iter()
        .filter(|e| !matches!(e.entry_type(), EntryType::Address | EntryType::Files))
        .filter(|e| e.uri_match_mode() != UriMatchMode::Never)
        .filter(|e| has_passkey(e) && stored_rp_is(e, rp_id))
        // Last: the bin walk is the expensive test and few entries reach it.
        .filter(|e| !db.is_in_recycle_bin(e.uuid))
        .collect()
}

/// The entries a `passkeyCreate` may save into: the domain matches for `origin` first (the
/// logins the user knows for the site), then the entries already holding a passkey for `rp_id`
/// that the domain match missed, each entry once. Ports `passkeyCreateCandidates`.
#[must_use]
pub fn create_candidates<'a>(db: &'a Database, origin: &str, rp_id: &str) -> Vec<&'a Entry> {
    let mut out = credential_candidates(db, origin);
    for entry in passkey_entries(db, rp_id) {
        if !out.iter().any(|seen| seen.uuid == entry.uuid) {
            out.push(entry);
        }
    }
    out
}

/// Whether a live entry already holds one of the credentials the page listed in
/// `excludeCredentials` for `rp_id`. An authenticator ownership check, not an autofill match:
/// the URI match mode and the entry type do not matter, only the recycle bin does (a binned
/// passkey is not a live credential). Ids are compared as decoded bytes.
#[must_use]
pub fn excluded_credential_exists(db: &Database, rp_id: &str, exclude: &[Vec<u8>]) -> bool {
    if exclude.is_empty() {
        return false;
    }
    db.entries_excluding(&[]).into_iter().any(|e| {
        has_passkey(e)
            && stored_rp_is(e, rp_id)
            && PasskeyRecord::credential_id(e).is_some_and(|id| exclude.contains(&id))
            && !db.is_in_recycle_bin(e.uuid)
    })
}
