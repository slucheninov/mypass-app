//! `passkeyCreate` (phase one) and `passkeyCreateConfirm` (phase two). Ports the create half of
//! the Swift `ExtensionBridgeService` passkey handling.
//!
//! **Phase one** parses and routes only: it validates the request and its origin, lists the
//! entries the user may save into, and keeps the ceremony in the pending table. It generates no
//! key and writes nothing. `excludeCredentials` is deliberately not checked here: WebAuthn lets
//! an authenticator reveal that it already holds an excluded credential only after user consent,
//! so that answer waits for the Save / Replace click (phase two).
//!
//! **Phase two** takes the ceremony (single use: every answer consumes it), then runs its checks
//! and its write in ONE `session.mutate` (R8): the vault binding, `excludeCredentials` against
//! the vault as it is now, the target against the candidate set recomputed now, the replace
//! consent, then the key generation and the write. Nothing can change between a check and the
//! write, and an error leaves the vault untouched (the clone is dropped). The key is generated
//! inside that closure, written straight into the protected fields and dropped there; only the
//! response JSON (public key, attestation, credential id) leaves it.
//!
//! **Durable before the relying party sees the public key.** After the write, `session.flush()`
//! must succeed before the response JSON is returned (R9). The Swift app answered after an
//! in-memory change only, so a crash inside the save debounce left the site holding a public key
//! whose private half was lost. When the flush fails the page gets `NotAllowedError` "Could not
//! save the passkey" and no response: the edit stays in memory and the autosave retries it; the
//! key it holds is an orphan the site never registered, which is harmless.

use mypass_core::passkey::{parse_create, perform_create, BackupFlags, PasskeyRecord};
use mypass_kdbx::{Database, Entry, Timestamp};
use mypass_session::session::SessionError;
use uuid::Uuid;

use super::candidates::{create_candidates, excluded_credential_exists};
use super::errors::{create_error, page_error, INVALID_STATE, NOT_ALLOWED};
use super::pending::PendingCreate;
use super::{PasskeyContext, PasskeyService};
use crate::protocol::Response;
use crate::views::{parse_wire_id, summary};

/// The answer when there is no ceremony to confirm (never started, expired, consumed, dropped by
/// a lock, a vault switch or the toggle).
pub(crate) const NO_PENDING: &str = "No pending request";
/// The answer when the write or the flush failed.
pub(crate) const COULD_NOT_SAVE: &str = "Could not save the passkey";

/// `passkeyCreate`. The toggle was checked by the caller (`PasskeyService::handle`).
pub(crate) fn handle_create(
    svc: &PasskeyService,
    ctx: &PasskeyContext<'_>,
    request_id: i64,
    request_details_json: &str,
    origin_url: &str,
) -> Response {
    if !ctx.unlocked {
        return Response::Locked;
    }
    let request = match parse_create(origin_url, request_details_json) {
        Ok(r) => r,
        Err(e) => {
            let (name, message) = page_error(&e);
            return create_error(request_id, name, message);
        }
    };
    // Read the epoch before the vault read: a lock or a switch that lands in between makes the
    // put below fail instead of leaving a ceremony for a vault that is gone (R7).
    let epoch = svc.epoch();
    let read = ctx.session.with_db(|db| {
        let found = create_candidates(db, request.origin().as_str(), request.rp_id());
        let matches = found.iter().map(|e| summary(e)).collect::<Vec<_>>();
        let ids = found.iter().map(|e| e.uuid).collect();
        (matches, PendingCreate::new(db, request.clone(), ids))
    });
    let Ok((matches, pending)) = read else {
        return Response::Locked;
    };
    // A repeated request id replaces the earlier ceremony (the Swift dictionary semantics).
    if !svc.put_create(epoch, request_id, pending) {
        return Response::Locked;
    }
    let ceremony = request.request();
    Response::PasskeyCreatePending {
        request_id,
        rp_id: request.rp_id().to_owned(),
        rp_name: if ceremony.rp_name.is_empty() {
            request.rp_id().to_owned()
        } else {
            ceremony.rp_name.clone()
        },
        matches,
    }
}

/// Why confirm refused inside its `mutate`. Each maps to one fixed page error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The ceremony belongs to another vault (a switch landed after the take).
    OtherVault,
    /// An `excludeCredentials` id is already held for this rpId.
    Excluded,
    /// The chosen entry is not a current candidate (forged, binned, moved off the site).
    NotCandidate,
    /// The chosen entry already holds a private key and `replace` was not set.
    HasKey,
    /// The key could not be generated, or the write failed.
    Failed,
}

impl Refusal {
    fn answer(self, request_id: i64) -> Response {
        let (name, message) = match self {
            Self::OtherVault => (NOT_ALLOWED, NO_PENDING),
            Self::Excluded => (INVALID_STATE, "A passkey for this account already exists"),
            Self::NotCandidate => (NOT_ALLOWED, "Entry not found or domain mismatch"),
            Self::HasKey => (INVALID_STATE, "Entry already has a passkey"),
            Self::Failed => (NOT_ALLOWED, COULD_NOT_SAVE),
        };
        create_error(request_id, name, message)
    }
}

/// What the confirm `mutate` closure hands back: the entry written and the response JSON. The
/// private key never leaves the closure.
struct Written {
    entry: Uuid,
    response_json: String,
}

/// `passkeyCreateConfirm`. The toggle was re-checked by the caller. `entry_id` absent means a new
/// entry; `replace` is the user's consent to overwrite an existing key.
pub(crate) fn handle_confirm(
    svc: &PasskeyService,
    ctx: &PasskeyContext<'_>,
    request_id: i64,
    entry_id: Option<&str>,
    replace: bool,
) -> Response {
    // Single use: from here on every answer has consumed the ceremony.
    let Some(pending) = svc.take_create(request_id) else {
        return create_error(request_id, NOT_ALLOWED, NO_PENDING);
    };
    // An id the extension sent but that is not an id at all is a target that cannot match.
    let target = match entry_id.map(parse_wire_id) {
        None => None,
        Some(Some(id)) => Some(id),
        Some(None) => return Refusal::NotCandidate.answer(request_id),
    };

    let mut refusal = None;
    let written = ctx.session.mutate(|db, now| {
        confirm_in_vault(db, now, &pending, target, replace).map_err(|r| {
            refusal = Some(r);
            // Aborts the mutate (the draft is dropped); the reason travels in `refusal`.
            SessionError::InvalidInput("passkey confirm refused".into())
        })
    });
    let written = match (written, refusal) {
        (Ok(w), _) => w,
        (Err(_), Some(r)) => return r.answer(request_id),
        (Err(SessionError::Locked | SessionError::NoVault), None) => return Response::Locked,
        // Quitting, or anything else the session refused before the closure ran.
        (Err(_), None) => return Refusal::Failed.answer(request_id),
    };

    // Durable before the relying party sees the public key (see the module docs). `flush` waits
    // for an in-flight background write and is `Ok` only once this edit is on disk.
    if ctx.session.flush().is_err() {
        return Refusal::Failed.answer(request_id);
    }
    svc.ui.credential_filled(written.entry);
    Response::PasskeyCreateComplete {
        request_id,
        response_json: Some(written.response_json),
        error: None,
    }
}

/// The body of the one confirm `mutate` (R8): every check against `db` as it is now, then the key
/// generation and the write. Any `Err` drops the draft.
fn confirm_in_vault(
    db: &mut Database,
    now: Timestamp,
    pending: &PendingCreate,
    target: Option<Uuid>,
    replace: bool,
) -> Result<Written, Refusal> {
    // First, before reading anything: the ceremony must belong to this vault.
    if !pending.vault_matches(db) {
        return Err(Refusal::OtherVault);
    }
    let request = &pending.request;
    let rp_id = request.rp_id();

    // Re-read now: another registration, a sync or the editor may have added the key while the
    // popup was open.
    if excluded_credential_exists(db, rp_id, &request.request().exclude_credential_ids) {
        return Err(Refusal::Excluded);
    }

    // The target must be one the popup was offered AND one that is still a candidate now (not
    // binned, not moved off the site, not switched to never-mode since). Without this a forged
    // or stale id would attach a passkey to an unrelated entry.
    if let Some(id) = target {
        let still_offered = pending.candidates.contains(&id)
            && create_candidates(db, request.origin().as_str(), rp_id)
                .iter()
                .any(|e| e.uuid == id);
        let entry = db.find_entry(id).filter(|_| still_offered);
        let Some(entry) = entry else {
            return Err(Refusal::NotCandidate);
        };
        // `has_private_key` (a non-empty PEM), not `has_passkey`: Swift parity. An entry with a
        // PEM but an undecodable credential id shows "Save" in the popup yet still needs
        // `replace` here; nothing is overwritten without consent, and the user can retry.
        if PasskeyRecord::has_private_key(entry) && !replace {
            return Err(Refusal::HasKey);
        }
    }

    let outcome = perform_create(request).map_err(|_| Refusal::Failed)?;
    let ceremony = request.request();
    let record = PasskeyRecord {
        rp_id: rp_id.to_owned(),
        credential_id: outcome.credential_id.clone(),
        private_key_pem: outcome.private_key_pem.clone(),
        user_handle: Some(ceremony.user_id.clone()),
        username: Some(ceremony.user_name.clone()),
        created: Some(now),
        flags: BackupFlags::default(),
    };

    let entry = match target {
        Some(id) => {
            // `update_entry` keeps the stored version, including any previous private key, in the
            // entry's history. An authenticator cannot know whether the relying party accepted
            // the new credential (Google, for one, may keep asking for the old one), so the old
            // key must stay recoverable.
            db.update_entry(id, now, |e| record.write(e))
                .map_err(|_| Refusal::Failed)?;
            id
        }
        None => {
            let mut e = Entry::new(now);
            let title = if ceremony.rp_name.is_empty() {
                rp_id
            } else {
                &ceremony.rp_name
            };
            e.set_field("Title", title, false);
            e.set_field("UserName", &ceremony.user_name, false);
            e.set_field("URL", request.origin().as_str(), false);
            record.write(&mut e);
            let id = e.uuid;
            // The root group, the same target the stage 3 `createEntry` uses.
            let root = db.root_id();
            db.add_entry(root, e, now).map_err(|_| Refusal::Failed)?;
            id
        }
    };
    Ok(Written {
        entry,
        response_json: outcome.response_json,
    })
}

#[cfg(test)]
mod tests;
