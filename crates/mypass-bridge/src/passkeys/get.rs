//! `passkeyGet` (phase one) and `passkeyGetApprove` (phase two). Ports the get half of the Swift
//! `ExtensionBridgeService` passkey handling, tightened by R11 and R13.
//!
//! **Phase one** parses and routes only: it validates the request and its origin, lists the
//! entries holding a passkey for the canonical rpId (narrowed to `allowCredentials` when the page
//! named credentials), and keeps the ceremony in the pending table. It reads no key and signs
//! nothing. An empty `allowCredentials` is a discoverable-credential sign-in: every passkey stored
//! for the rpId is offered (create answers `credProps` with `rk: true`, which relies on this).
//! Unlike create, a login without a key is never offered: there is nothing to sign with.
//!
//! **Phase two** takes the ceremony (single use: every answer consumes it), then runs its checks
//! and the signing in ONE `session.with_db` closure, where the vault cannot change under it: the
//! vault binding first; then the chosen entry must have been offered by phase one AND still hold a
//! passkey for the pending rpId now (not binned, not never-mode, not moved to another relying
//! party: a compromised extension cannot name another site's entry); its stored rpId, in
//! canonical form, must equal the pending rpId (Task 3 M2, an explicit check independent of the
//! lookup, so a lookup bug fails closed); and when the page named credentials, its credential id
//! must be one of them NOW (R11: a replace between get and approve must not sign with a credential
//! the relying party did not ask for). The private key is decoded inside that closure into
//! zeroizing values, used once, and dropped there; only the assertion JSON leaves it.
//!
//! **Last use, not an edit.** A successful approve records the entry's access time and usage count
//! through `VaultSession::touch` (`Database::mark_accessed`), the stage 3 non-dirty path that
//! `fillCredential` uses: signing in does not mark the vault dirty or schedule a save. No counter
//! is stored (R12: the signature counter is always 0). The touch re-checks the vault binding, so a
//! switch that lands after the signing never touches an entry in another vault.
use mypass_core::passkey::{
    parse_get, perform_get, PasskeyError, PasskeyRecord, PasskeySigningKey,
};
use mypass_core::psl::canonical_host;
use mypass_kdbx::Database;
use uuid::Uuid;

use super::candidates::passkey_entries;
use super::create::NO_PENDING;
use super::errors::{get_error, page_error, NOT_ALLOWED};
use super::pending::PendingGet;
use super::{PasskeyContext, PasskeyService};
use crate::protocol::Response;
use crate::views::{parse_wire_id, summary};

/// The answer when no stored passkey matches the request.
pub(crate) const NO_PASSKEY: &str = "No passkey for this site";
/// The answer when the chosen entry is not (or no longer) a passkey for the pending request.
pub(crate) const ENTRY_NOT_FOUND: &str = "Passkey entry not found";
/// The answer when the stored key cannot sign.
pub(crate) const SIGN_FAILED: &str = "Failed to sign assertion";

/// `passkeyGet`. The toggle was checked by the caller (`PasskeyService::handle`).
pub(crate) fn handle_get(
    svc: &PasskeyService,
    ctx: &PasskeyContext<'_>,
    request_id: i64,
    request_details_json: &str,
    origin_url: &str,
) -> Response {
    if !ctx.unlocked {
        return Response::Locked;
    }
    let request = match parse_get(origin_url, request_details_json) {
        Ok(r) => r,
        Err(e) => {
            let (name, message) = page_error(&e);
            return get_error(request_id, name, message);
        }
    };
    // Read the epoch before the vault read (R7, see `create::handle_create`).
    let epoch = svc.epoch();
    let read = ctx.session.with_db(|db| {
        let found = get_candidates(db, request.rp_id(), &request.request().allow_credential_ids);
        if found.is_empty() {
            return None;
        }
        let matches = found.iter().map(|e| summary(e)).collect::<Vec<_>>();
        let ids = found.iter().map(|e| e.uuid).collect();
        Some((matches, PendingGet::new(db, request.clone(), ids)))
    });
    let (matches, pending) = match read {
        Ok(Some(found)) => found,
        // Nothing to offer: no ceremony is kept (R25 residual 3: the page learns it at once).
        Ok(None) => return get_error(request_id, NOT_ALLOWED, NO_PASSKEY),
        Err(_) => return Response::Locked,
    };
    // A repeated request id replaces the earlier ceremony (the Swift dictionary semantics).
    if !svc.put_get(epoch, request_id, pending) {
        return Response::Locked;
    }
    Response::PasskeyGetPending {
        request_id,
        rp_id: request.rp_id().to_owned(),
        matches,
    }
}

/// The entries a `passkeyGet` offers: the passkeys stored for `rp_id`, narrowed to `allow` (as
/// decoded bytes) when it is non-empty.
fn get_candidates<'a>(
    db: &'a Database,
    rp_id: &str,
    allow: &[Vec<u8>],
) -> Vec<&'a mypass_kdbx::Entry> {
    passkey_entries(db, rp_id)
        .into_iter()
        .filter(|e| allowed(allow, PasskeyRecord::credential_id(e).as_deref()))
        .collect()
}

/// Whether `credential` may answer a request listing `allow` (empty: any credential of the rpId).
fn allowed(allow: &[Vec<u8>], credential: Option<&[u8]>) -> bool {
    allow.is_empty() || credential.is_some_and(|c| allow.iter().any(|a| a.as_slice() == c))
}

/// Why approve refused inside its `with_db`. Each maps to one fixed page error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The ceremony belongs to another vault (a switch landed after the take).
    OtherVault,
    /// The entry was not offered, or is no longer a passkey for the pending rpId.
    NotFound,
    /// The entry's credential id is not in the page's non-empty `allowCredentials` (R11).
    NotAllowed,
    /// The stored key could not be loaded or could not sign.
    SignFailed,
}

impl Refusal {
    fn answer(self, request_id: i64) -> Response {
        let (name, message) = match self {
            Self::OtherVault => (NOT_ALLOWED, NO_PENDING),
            Self::NotFound => (NOT_ALLOWED, ENTRY_NOT_FOUND),
            Self::NotAllowed => page_error(&PasskeyError::CredentialNotAllowed),
            Self::SignFailed => (NOT_ALLOWED, SIGN_FAILED),
        };
        get_error(request_id, name, message)
    }
}

/// `passkeyGetApprove`. The toggle was re-checked by the caller.
pub(crate) fn handle_approve(
    svc: &PasskeyService,
    ctx: &PasskeyContext<'_>,
    request_id: i64,
    entry_id: &str,
) -> Response {
    // Single use: from here on every answer has consumed the ceremony.
    let Some(pending) = svc.take_get(request_id) else {
        return get_error(request_id, NOT_ALLOWED, NO_PENDING);
    };
    let Some(target) = parse_wire_id(entry_id) else {
        return Refusal::NotFound.answer(request_id);
    };
    let signed = match ctx
        .session
        .with_db(|db| approve_in_vault(db, &pending, target))
    {
        Ok(Ok(json)) => json,
        Ok(Err(refusal)) => return refusal.answer(request_id),
        Err(_) => return Response::Locked,
    };
    // Last use, never dirty (see the module docs). The assertion was made while the vault was
    // unlocked and the user approved it, so a lock landing between the signing and this touch
    // only skips the access time.
    let touched = ctx
        .session
        .touch(|db, now| pending.vault_matches(db) && db.mark_accessed(target, now));
    if touched == Ok(true) {
        svc.ui.credential_filled(target);
    }
    Response::PasskeyGetComplete {
        request_id,
        response_json: Some(signed),
        error: None,
    }
}

/// The body of the one approve `with_db` (R11): every check against `db` as it is now, then the
/// signing. The stored key is decoded here and dropped before the function returns.
pub(crate) fn approve_in_vault(
    db: &Database,
    pending: &PendingGet,
    target: Uuid,
) -> Result<String, Refusal> {
    // First, before reading anything: the ceremony must belong to this vault.
    if !pending.vault_matches(db) {
        return Err(Refusal::OtherVault);
    }
    let request = &pending.request;
    let rp_id = request.rp_id();

    // Offered by phase one AND still a passkey for this rpId now (bin, never-mode, entry type and
    // the stored rpId are all re-read).
    if !pending.candidates.contains(&target) {
        return Err(Refusal::NotFound);
    }
    let entry = passkey_entries(db, rp_id)
        .into_iter()
        .find(|e| e.uuid == target)
        .ok_or(Refusal::NotFound)?;
    // Task 3 M2: the key is bound to the relying party it was stored for. `rp_id()` is canonical
    // (R13); the stored value is canonicalised here (KeePassXC may have written `Example.COM`).
    let stored_rp = PasskeyRecord::relying_party(entry).and_then(canonical_host);
    if stored_rp.as_deref() != Some(rp_id) {
        return Err(Refusal::NotFound);
    }
    // `record.private_key_pem` is `Zeroizing`; the record is dropped at the end of this function.
    let record = PasskeyRecord::read(entry).ok_or(Refusal::NotFound)?;
    // R11: the credential must be one the page asked for, as it is now (a replace between get and
    // approve changes the id).
    if !allowed(
        &request.request().allow_credential_ids,
        Some(&record.credential_id),
    ) {
        return Err(Refusal::NotAllowed);
    }
    // Zeroized on drop, neither `Clone` nor `Debug`.
    let key =
        PasskeySigningKey::from_pem(&record.private_key_pem).map_err(|_| Refusal::SignFailed)?;
    let outcome = perform_get(
        request,
        &key,
        &record.credential_id,
        record.user_handle.as_deref(),
        record.flags,
    )
    .map_err(|e| match e {
        PasskeyError::CredentialNotAllowed => Refusal::NotAllowed,
        PasskeyError::UnsupportedAlgorithm
        | PasskeyError::OriginMismatch
        | PasskeyError::RpIdNotAuthorized
        | PasskeyError::CrossOrigin
        | PasskeyError::InvalidRequest
        | PasskeyError::Crypto => Refusal::SignFailed,
    })?;
    Ok(outcome.response_json)
}

#[cfg(test)]
mod tests;
