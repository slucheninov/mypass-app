//! Lossless entry editing, entry and group operations, attachments and history restore.
//!
//! The editor works on an [`EntryEditModel`], the plain values of one entry (the one sanctioned
//! place where secrets cross into the webview, see `dto.rs`). Saving applies the model to the
//! stored entry *field by field*, so everything the editor does not own survives: passkey fields,
//! `KP2A_URL` arrays, unknown custom fields, protection flags set by another KeePass client, and
//! attachments beyond today's limits. Every real change goes through `Database::update_entry`, so
//! it snapshots history (trimmed to the vault's `history_max_*`); a save that changes nothing is
//! `Ok` and records nothing.
//!
//! Protection flags of the app's own fields follow the Swift app (the on-disk contract shared with
//! KeePassXC users): password, TOTP, card number, CVV and every address field are protected; user
//! name, URL, notes, the notes body, card holder and expiry are not. An existing field keeps the
//! flag it already has.

use crate::dto::{
    timestamp_ms, AddressDto, AttachmentMeta, AttachmentOp, CardEdit, CustomFieldEdit,
    EntryEditModel, EntryTypeDto, FieldRef, HistoryItem,
};
use crate::fields::{
    bin_id, require_not_history, EntryFields, ADDRESS_FIELDS, ADDRESS_LABELS, ADDRESS_PREFIX,
    CARD_CVV, CARD_EXPIRY, CARD_HOLDER, CARD_NUMBER, GROUP_EMOJI, NOTES_BODY, OTP,
};
use crate::queries::{passkey_fingerprint, passkey_meta, totp_now};
use crate::session::{SessionError, VaultSession};
use mypass_core::entry_ext::{EntryExt, EntryType, UriMatchMode, ENTRY_TYPE_FIELD};
use mypass_core::passkey::PasskeyRecord;
use mypass_core::TotpConfig;
use mypass_kdbx::{Attachment, Database, Entry, Group};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Attachments an entry may hold; only further adds are refused beyond it.
pub const MAX_ATTACHMENTS_PER_ENTRY: usize = 8;
/// The largest attachment file that can be added.
pub const MAX_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;

fn io_error(e: &std::io::Error) -> SessionError {
    SessionError::Io(e.to_string())
}

fn parse_id(text: &str) -> Result<Uuid, SessionError> {
    Uuid::parse_str(text).map_err(|_| SessionError::InvalidInput("malformed id".into()))
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn core_type(kind: EntryTypeDto) -> EntryType {
    match kind {
        EntryTypeDto::Login => EntryType::Login,
        EntryTypeDto::Address => EntryType::Address,
        EntryTypeDto::Card => EntryType::Card,
        EntryTypeDto::Notes => EntryType::Notes,
        EntryTypeDto::Files => EntryType::Files,
    }
}

/// The stored value of the type marker; `None` for a login (the field is absent).
fn type_marker(kind: EntryTypeDto) -> Option<&'static str> {
    match kind {
        EntryTypeDto::Login => None,
        EntryTypeDto::Address => Some("Address"),
        EntryTypeDto::Card => Some("Card"),
        EntryTypeDto::Notes => Some("Notes"),
        EntryTypeDto::Files => Some("Files"),
    }
}

/// The field that holds the free text of an entry: the notes body of a notes entry, `Notes`
/// otherwise.
fn notes_key(kind: EntryType) -> &'static str {
    if kind == EntryType::Notes {
        NOTES_BODY
    } else {
        "Notes"
    }
}

fn find_entry(db: &Database, id: Uuid) -> Result<&Entry, SessionError> {
    db.find_entry(id).ok_or(SessionError::NotFound)
}

fn require_group(db: &Database, id: Uuid) -> Result<&Group, SessionError> {
    db.find_group(id).ok_or(SessionError::NotFound)
}

// ---- the model ----

/// Pure defaults for a new entry of `entry_type` in `group_id`; changes nothing in the vault.
pub fn new_model(entry_type: EntryTypeDto, group_id: Uuid) -> EntryEditModel {
    EntryEditModel {
        id: None,
        entry_type,
        group_id: group_id.simple().to_string(),
        title: String::new(),
        favorite: false,
        username: String::new(),
        password: String::new(),
        urls: Vec::new(),
        uri_match_mode: UriMatchMode::RegistrableDomain.as_str().to_owned(),
        totp: String::new(),
        notes: String::new(),
        address: (entry_type == EntryTypeDto::Address).then(|| AddressDto {
            label: "Home".into(),
            ..AddressDto::default()
        }),
        card: (entry_type == EntryTypeDto::Card).then(CardEdit::default),
        custom_fields: Vec::new(),
        attachments: Vec::new(),
        attachment_ops: Vec::new(),
        passkey: None,
        remove_passkey: false,
        passkey_ref: None,
    }
}

fn field_or_empty(entry: &Entry, key: &str) -> String {
    entry.field(key).unwrap_or("").to_owned()
}

/// The address as stored: no synthesised full name (so editing the parts cannot freeze an old
/// synthesised name), an unknown label folded to `Other`.
fn stored_address(entry: &Entry) -> AddressDto {
    let get = |name: &str| field_or_empty(entry, &format!("{ADDRESS_PREFIX}{name}"));
    let mut label = get("label");
    if !ADDRESS_LABELS.contains(&label.as_str()) {
        label = "Other".into();
    }
    AddressDto {
        label,
        full_name: get("fullName"),
        given_name: get("givenName"),
        family_name: get("familyName"),
        organization: get("organization"),
        address_line1: get("addressLine1"),
        address_line2: get("addressLine2"),
        city: get("city"),
        region: get("region"),
        postal_code: get("postalCode"),
        country_code: get("countryCode"),
        country_name: get("countryName"),
        phone: get("phone"),
        email: get("email"),
    }
}

/// The address values in the order of [`ADDRESS_FIELDS`].
fn address_values(a: &AddressDto) -> [&str; 14] {
    let label = if ADDRESS_LABELS.contains(&a.label.as_str()) {
        a.label.as_str()
    } else {
        "Other"
    };
    [
        label,
        &a.full_name,
        &a.given_name,
        &a.family_name,
        &a.organization,
        &a.address_line1,
        &a.address_line2,
        &a.city,
        &a.region,
        &a.postal_code,
        &a.country_code,
        &a.country_name,
        &a.phone,
        &a.email,
    ]
}

/// The edit model of an existing entry, with the plain values (the user explicitly opened Edit).
pub fn edit_model(db: &Database, id: Uuid) -> Result<EntryEditModel, SessionError> {
    let entry = find_entry(db, id)?;
    let group = db.parent_of_entry(id).ok_or(SessionError::NotFound)?;
    let kind = entry.entry_type();
    let dto_kind = EntryTypeDto::from(kind);
    Ok(EntryEditModel {
        id: Some(id.simple().to_string()),
        entry_type: dto_kind,
        group_id: group.simple().to_string(),
        title: field_or_empty(entry, "Title"),
        favorite: entry.is_favorite(),
        username: field_or_empty(entry, "UserName"),
        password: field_or_empty(entry, "Password"),
        urls: entry.urls(),
        uri_match_mode: entry.uri_match_mode().as_str().to_owned(),
        totp: entry.otp_source().unwrap_or("").to_owned(),
        notes: field_or_empty(entry, notes_key(kind)),
        address: (kind == EntryType::Address).then(|| stored_address(entry)),
        card: (kind == EntryType::Card).then(|| CardEdit {
            holder: field_or_empty(entry, CARD_HOLDER),
            number: field_or_empty(entry, CARD_NUMBER),
            expiry: field_or_empty(entry, CARD_EXPIRY),
            cvv: field_or_empty(entry, CARD_CVV),
        }),
        // Another client can store the same key twice; the first one is the one the editor shows
        // (and `Entry::field` returns), the extras are dropped by the next real save.
        custom_fields: {
            let mut seen = HashSet::new();
            entry
                .fields
                .iter()
                .filter(|f| !Entry::is_reserved_key(&f.key) && seen.insert(f.key.as_str()))
                .map(|f| CustomFieldEdit {
                    key: f.key.clone(),
                    value: f.value.clone(),
                    protected: f.protected,
                })
                .collect()
        },
        attachments: entry
            .attachments
            .iter()
            .map(|a| AttachmentMeta {
                name: a.name.clone(),
                size: a.data.len() as f64,
            })
            .collect(),
        attachment_ops: Vec::new(),
        // Display data only; the `KPEX_PASSKEY_*` values themselves never enter the model.
        passkey: passkey_meta(entry),
        remove_passkey: false,
        passkey_ref: passkey_fingerprint(entry),
    })
}

// ---- saving ----

/// An attachment change with its file already read (reads happen before the session lock).
enum ResolvedOp {
    Add { name: String, data: Vec<u8> },
    Remove { name: String },
}

/// Writes `value` to `key`, keeping the protection flag of an existing field (`protected` is the
/// flag of a new one). An absent field is created only for a non-empty value, or when
/// `create_empty` is set; an unchanged value is not touched at all.
pub(crate) fn put(entry: &mut Entry, key: &str, value: &str, protected: bool, create_empty: bool) {
    match entry.fields.iter().find(|f| f.key == key) {
        Some(f) if f.value == value => {}
        Some(f) => {
            let keep = f.protected;
            entry.set_field(key, value, keep);
        }
        None if create_empty || !value.is_empty() => entry.set_field(key, value, protected),
        None => {}
    }
}

fn validate(model: &EntryEditModel) -> Result<UriMatchMode, SessionError> {
    let mode = UriMatchMode::parse(&model.uri_match_mode)
        .ok_or_else(|| SessionError::InvalidInput("unknown URL match mode".into()))?;
    let totp = model.totp.trim();
    if !totp.is_empty() && TotpConfig::parse(totp).is_none() {
        return Err(SessionError::InvalidInput(
            "the TOTP value is neither a base32 secret nor an otpauth URI".into(),
        ));
    }
    match model.entry_type {
        EntryTypeDto::Address if model.address.is_none() => {
            return Err(SessionError::InvalidInput(
                "an address entry needs its address".into(),
            ));
        }
        EntryTypeDto::Card if model.card.is_none() => {
            return Err(SessionError::InvalidInput(
                "a card entry needs its card".into(),
            ));
        }
        _ => {}
    }
    let mut seen = HashSet::new();
    for field in &model.custom_fields {
        if field.key.is_empty() {
            return Err(SessionError::InvalidInput(
                "a custom field has an empty name".into(),
            ));
        }
        if Entry::is_reserved_key(&field.key) {
            return Err(SessionError::InvalidInput(format!(
                "custom field name is reserved: {}",
                field.key
            )));
        }
        if !seen.insert(field.key.as_str()) {
            return Err(SessionError::InvalidInput(format!(
                "duplicate custom field name: {}",
                field.key
            )));
        }
    }
    Ok(mode)
}

/// The attachment name a file at `path` would get (the same rule as [`read_attachment_file`]).
fn file_name_of(path: &Path) -> Option<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
}

fn resolve_ops(ops: &[AttachmentOp]) -> Result<Vec<ResolvedOp>, SessionError> {
    // Cheap checks first, so nothing is read from disk for a model that is refused anyway.
    let mut added = HashSet::new();
    let mut adds = 0usize;
    for op in ops {
        if let AttachmentOp::Add { path } = op {
            adds += 1;
            if adds > MAX_ATTACHMENTS_PER_ENTRY {
                return Err(SessionError::LimitExceeded(format!(
                    "an entry holds at most {MAX_ATTACHMENTS_PER_ENTRY} attachments"
                )));
            }
            if let Some(name) = file_name_of(Path::new(path)) {
                if !added.insert(name.clone()) {
                    return Err(SessionError::InvalidInput(format!(
                        "two attachments are named {name}"
                    )));
                }
            }
        }
    }
    ops.iter()
        .map(|op| match op {
            AttachmentOp::Add { path } => read_attachment_file(Path::new(path))
                .map(|(name, data)| ResolvedOp::Add { name, data }),
            AttachmentOp::Remove { name } => Ok(ResolvedOp::Remove { name: name.clone() }),
        })
        .collect()
}

/// Removes the later duplicates of every custom key (the first one wins, like `Entry::field`).
fn dedupe_custom_fields(entry: &mut Entry) {
    let mut seen: HashSet<String> = HashSet::new();
    entry
        .fields
        .retain(|f| Entry::is_reserved_key(&f.key) || seen.insert(f.key.clone()));
}

/// Applies a validated model to an entry. Errors (attachment limits, a removal of a file that is
/// not there) leave the entry half edited; the caller discards it by returning the error from
/// `VaultSession::mutate`, which rolls back.
fn apply(
    entry: &mut Entry,
    model: &EntryEditModel,
    mode: UriMatchMode,
    ops: Vec<ResolvedOp>,
) -> Result<(), SessionError> {
    let old_kind = entry.entry_type();
    let new_kind = core_type(model.entry_type);
    if new_kind != old_kind {
        match type_marker(model.entry_type) {
            Some(marker) => entry.set_field(ENTRY_TYPE_FIELD, marker, false),
            None => entry.remove_field(ENTRY_TYPE_FIELD),
        }
    }
    let title = match (&model.address, model.entry_type) {
        (Some(a), EntryTypeDto::Address) if model.title.trim().is_empty() => {
            address_values(a)[0].to_owned()
        }
        (_, EntryTypeDto::Address) => model.title.trim().to_owned(),
        _ => model.title.clone(),
    };
    put(entry, "Title", &title, false, true);
    put(entry, "UserName", &model.username, false, false);
    put(entry, "Password", &model.password, true, false);

    let mut urls: Vec<String> = Vec::new();
    for url in model
        .urls
        .iter()
        .map(|u| u.trim())
        .filter(|u| !u.is_empty())
    {
        if !urls.iter().any(|seen| seen == url) {
            urls.push(url.to_owned());
        }
    }
    if urls != entry.urls() {
        entry.set_urls(&urls);
    }
    if mode != entry.uri_match_mode() {
        entry.set_uri_match_mode(mode);
    }

    let totp = model.totp.trim();
    if totp.is_empty() {
        entry.remove_field(OTP);
    } else {
        put(entry, OTP, totp, true, true);
    }
    if model.favorite != entry.is_favorite() {
        entry.set_favorite(model.favorite);
    }
    // The free text lives in `Notes` or `MyPass.NotesBody` by type. On a type change the model's
    // text goes to the new type's key, unless that key already holds a different non-empty text
    // (a value from another client): that one is never overwritten, the text stays in (or is
    // written to) the old type's key instead, so no notes text is lost.
    let target = notes_key(new_kind);
    let foreign = new_kind != old_kind
        && notes_key(old_kind) != target
        && entry
            .field(target)
            .is_some_and(|v| !v.is_empty() && v != model.notes);
    let notes_at = if foreign { notes_key(old_kind) } else { target };
    put(entry, notes_at, &model.notes, false, false);

    if let (EntryTypeDto::Card, Some(card)) = (model.entry_type, &model.card) {
        put(entry, CARD_HOLDER, &card.holder, false, false);
        put(entry, CARD_NUMBER, &card.number, true, false);
        put(entry, CARD_EXPIRY, &card.expiry, false, false);
        put(entry, CARD_CVV, &card.cvv, true, false);
    }
    if let (EntryTypeDto::Address, Some(address)) = (model.entry_type, &model.address) {
        // A new address entry gets all fourteen fields (the Swift layout); an existing one is
        // only touched where a value changed.
        let fresh = !entry
            .fields
            .iter()
            .any(|f| f.key.starts_with(ADDRESS_PREFIX));
        for (name, value) in ADDRESS_FIELDS.iter().zip(address_values(address)) {
            let key = format!("{ADDRESS_PREFIX}{name}");
            put(entry, &key, value, true, fresh);
        }
    }

    // Custom fields: the model lists every non-reserved field, so a missing one was removed.
    let keep: HashSet<&str> = model.custom_fields.iter().map(|f| f.key.as_str()).collect();
    entry
        .fields
        .retain(|f| Entry::is_reserved_key(&f.key) || keep.contains(f.key.as_str()));
    for field in &model.custom_fields {
        // Compare with the first stored field of the key (the one that is shown and written).
        let same = entry
            .fields
            .iter()
            .find(|f| f.key == field.key)
            .is_some_and(|f| f.value == field.value && f.protected == field.protected);
        if !same {
            entry.set_field(&field.key, &field.value, field.protected);
        }
    }

    // The passkey is removed only on an explicit request, after the field rebuild above (which
    // keeps every reserved `KPEX_PASSKEY_*` field), and inside `update_entry`, so the history
    // snapshot taken before this edit still holds the key. `model.passkey` is never an input.
    // Only the passkey the editor showed is removed: when the stored one no longer matches
    // `passkey_ref` (a browser stored a replacement, with the user's consent in the popup, while
    // the editor was open), the removal is skipped and the rest of the edit is saved. The
    // passkey the user asked to remove is already gone in that case, and the new one is not
    // deleted unseen; the detail pane shows it after the save. A missing reference never
    // matches, and the reference only ever selects this entry's own passkey.
    if model.remove_passkey
        && model.passkey_ref.is_some()
        && passkey_fingerprint(entry) == model.passkey_ref
    {
        PasskeyRecord::clear(entry);
    }

    // Removals first, so "remove X then add X" in one model is the supported replace.
    let (removes, adds): (Vec<_>, Vec<_>) = ops
        .into_iter()
        .partition(|op| matches!(op, ResolvedOp::Remove { .. }));
    for op in removes.into_iter().chain(adds) {
        match op {
            ResolvedOp::Remove { name } => {
                let pos = entry
                    .attachments
                    .iter()
                    .position(|a| a.name == name)
                    .ok_or(SessionError::NotFound)?;
                entry.attachments.remove(pos);
            }
            ResolvedOp::Add { name, data } => {
                if entry.attachments.iter().any(|a| a.name == name) {
                    return Err(SessionError::InvalidInput(format!(
                        "an attachment named {name} already exists"
                    )));
                }
                if entry.attachments.len() >= MAX_ATTACHMENTS_PER_ENTRY {
                    // Only adds are limited: an entry that already exceeds it stays editable.
                    return Err(SessionError::LimitExceeded(format!(
                        "an entry holds at most {MAX_ATTACHMENTS_PER_ENTRY} attachments"
                    )));
                }
                entry.attachments.push(Attachment {
                    name,
                    data,
                    protected: false,
                });
            }
        }
    }
    Ok(())
}

/// Creates the entry (`model.id` is `None`) or saves the edit. Attachment files are read before
/// the vault is touched. Returns the entry id.
pub fn save_entry(session: &VaultSession, model: EntryEditModel) -> Result<Uuid, SessionError> {
    let id = model.id.as_deref().map(parse_id).transpose()?;
    let group = parse_id(&model.group_id)?;
    let mode = validate(&model)?;
    let ops = resolve_ops(&model.attachment_ops)?;
    session.mutate_tracked(|db, now| {
        require_not_history(db, group)?;
        match id {
            None => {
                let mut entry = Entry::new(now);
                apply(&mut entry, &model, mode, ops)?;
                let id = entry.uuid;
                db.add_entry(group, entry, now)?;
                Ok((id, true))
            }
            Some(id) => {
                find_entry(db, id)?;
                let mut failure = None;
                let changed = db.update_entry(id, now, |entry| {
                    let before = entry.clone();
                    failure = apply(entry, &model, mode, ops).err();
                    // Duplicate custom keys (another client) are dropped only with a real change;
                    // an unchanged save stays a no-op.
                    if failure.is_none() && !entry.content_eq(&before) {
                        dedupe_custom_fields(entry);
                    }
                })?;
                if let Some(error) = failure {
                    return Err(error);
                }
                let moved = db.parent_of_entry(id) != Some(group);
                if moved {
                    db.move_entry(id, group, now)?;
                }
                Ok((id, changed || moved))
            }
        }
    })
}

// ---- reveal ----

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One plain value of an entry, after an explicit user action. `Totp` is the current code.
pub fn reveal(db: &Database, id: Uuid, field: &FieldRef) -> Result<String, SessionError> {
    reveal_at(db, id, field, unix_now())
}

/// [`reveal`] at an explicit time (Unix seconds), for the TOTP code.
pub fn reveal_at(
    db: &Database,
    id: Uuid,
    field: &FieldRef,
    unix_secs: u64,
) -> Result<String, SessionError> {
    let entry = find_entry(db, id)?;
    let plain = |key: &str| Ok(field_or_empty(entry, key));
    match field {
        FieldRef::Username => plain("UserName"),
        FieldRef::Password => plain("Password"),
        FieldRef::Url => Ok(entry.urls().into_iter().next().unwrap_or_default()),
        FieldRef::NotesBody => plain(notes_key(entry.entry_type())),
        FieldRef::CardNumber => plain(CARD_NUMBER),
        FieldRef::CardCvv => plain(CARD_CVV),
        FieldRef::Totp => totp_now(db, id, unix_secs)
            .map(|t| t.code)
            .ok_or(SessionError::NotFound),
        FieldRef::Custom { key } => {
            if Entry::is_reserved_key(key) {
                return Err(SessionError::InvalidInput(format!(
                    "not a custom field: {key}"
                )));
            }
            entry
                .field(key)
                .map(str::to_owned)
                .ok_or(SessionError::NotFound)
        }
    }
}

// ---- entry operations ----

/// Sets or clears the favorite marker. A raw edit: no history snapshot, `modified` moves so a
/// merge keeps the newer state.
pub fn set_favorite(session: &VaultSession, id: Uuid, value: bool) -> Result<(), SessionError> {
    session.mutate_tracked(|db, now| {
        find_entry(db, id)?;
        let mut changed = false;
        if let Some(entry) = db.find_entry_mut(id) {
            if entry.is_favorite() != value {
                entry.set_favorite(value);
                entry.modified = now;
                changed = true;
            }
        }
        Ok(((), changed))
    })
}

/// Moves an entry to `group` (a no-op when it is already there). The generator history group is
/// not a valid target; the recycle bin is accepted (delete is the usual way in).
pub fn move_entry(session: &VaultSession, id: Uuid, group: Uuid) -> Result<(), SessionError> {
    session.mutate_tracked(|db, now| {
        find_entry(db, id)?;
        require_not_history(db, group)?;
        if db.parent_of_entry(id) == Some(group) {
            return Ok(((), false));
        }
        db.move_entry(id, group, now)?;
        Ok(((), true))
    })
}

/// Moves the entry to the recycle bin (purges it when the bin is disabled or it is already in it).
pub fn delete_entry(session: &VaultSession, id: Uuid) -> Result<(), SessionError> {
    session.mutate(|db, now| {
        find_entry(db, id)?;
        db.delete_entry(id, now)?;
        Ok(())
    })
}

/// Moves an entry out of the recycle bin into `group`.
pub fn restore_entry(session: &VaultSession, id: Uuid, group: Uuid) -> Result<(), SessionError> {
    session.mutate(|db, now| {
        find_entry(db, id)?;
        require_not_history(db, group)?;
        if !db.is_in_recycle_bin(id) {
            return Err(SessionError::InvalidInput(
                "entry is not in the recycle bin".into(),
            ));
        }
        db.restore_entry(id, group, now)?;
        Ok(())
    })
}

/// Permanently removes an entry and tombstones it.
pub fn purge_entry(session: &VaultSession, id: Uuid) -> Result<(), SessionError> {
    session.mutate(|db, now| {
        find_entry(db, id)?;
        db.purge_entry(id, now)?;
        Ok(())
    })
}

fn count_entries(group: &Group) -> usize {
    group.entries.len() + group.groups.iter().map(count_entries).sum::<usize>()
}

/// The recycle bin group (see [`bin_id`]).
fn bin_of(db: &Database) -> Option<&Group> {
    bin_id(db).and_then(|id| db.find_group(id))
}

/// Permanently removes everything in the recycle bin; returns how many entries went.
pub fn empty_bin(session: &VaultSession) -> Result<u32, SessionError> {
    empty_bin_removed(session).map(|(count, _)| count)
}

/// [`empty_bin`], also returning the ids of the entries that were removed (collected inside the
/// same mutation, so they are exactly what went).
pub fn empty_bin_removed(session: &VaultSession) -> Result<(u32, Vec<Uuid>), SessionError> {
    let nothing = session.with_db(|db| {
        bin_of(db).is_none_or(|bin| count_entries(bin) == 0 && bin.groups.is_empty())
    })?;
    if nothing {
        return Ok((0, Vec::new()));
    }
    session.mutate(|db, now| {
        let Some(bin) = bin_id(db) else {
            return Ok((0, Vec::new()));
        };
        let removed = crate::fields::entry_ids_under(db, bin);
        let Some(bin) = bin_of(db) else {
            return Ok((0, Vec::new()));
        };
        let count = count_entries(bin);
        let entries: Vec<Uuid> = bin.entries.iter().map(|e| e.uuid).collect();
        let groups: Vec<Uuid> = bin.groups.iter().map(|g| g.uuid).collect();
        for id in entries {
            db.purge_entry(id, now)?;
        }
        for id in groups {
            db.purge_group(id, now)?;
        }
        Ok((count_u32(count), removed))
    })
}

// ---- history ----

/// Names of what differs between an older snapshot and the version that replaced it.
fn changed_fields(older: &Entry, newer: &Entry) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |key: &str| {
        if !out.iter().any(|k| k == key) {
            out.push(key.to_owned());
        }
    };
    for f in &older.fields {
        let same = newer
            .fields
            .iter()
            .any(|n| n.key == f.key && n.value == f.value && n.protected == f.protected);
        if !same {
            push(&f.key);
        }
    }
    for n in &newer.fields {
        if !older.fields.iter().any(|f| f.key == n.key) {
            push(&n.key);
        }
    }
    for array in older.string_arrays.iter().chain(&newer.string_arrays) {
        let find = |e: &Entry| e.string_arrays.iter().find(|a| a.key == array.key).cloned();
        if find(older) != find(newer) {
            push(&array.key);
        }
    }
    if older.attachments != newer.attachments {
        push("Attachments");
    }
    out
}

/// The earlier versions of an entry, newest first (`index` 0 is the newest).
pub fn history(db: &Database, id: Uuid) -> Result<Vec<HistoryItem>, SessionError> {
    let entry = find_entry(db, id)?;
    let versions = &entry.history;
    Ok(versions
        .iter()
        .enumerate()
        .rev()
        .enumerate()
        .map(|(index, (pos, old))| {
            let newer = versions.get(pos.saturating_add(1)).unwrap_or(entry);
            HistoryItem {
                index: count_u32(index),
                modified: timestamp_ms(old.modified),
                title: old.title().to_owned(),
                changed_fields: changed_fields(old, newer),
            }
        })
        .collect())
}

/// Restores version `index` (newest first, as listed by [`history`]). The current state is
/// snapshotted into history first, so the restore is itself undoable.
pub fn restore_history(session: &VaultSession, id: Uuid, index: usize) -> Result<(), SessionError> {
    session.mutate(|db, now| {
        let entry = find_entry(db, id)?;
        let old = index
            .checked_add(1)
            .and_then(|n| entry.history.len().checked_sub(n))
            .and_then(|pos| entry.history.get(pos))
            .cloned()
            .ok_or(SessionError::NotFound)?;
        db.update_entry(id, now, |entry| {
            entry.fields = old.fields;
            entry.string_arrays = old.string_arrays;
            entry.attachments = old.attachments;
        })?;
        Ok(())
    })
}

// ---- attachments ----

/// Reads a file to attach: its name and bytes. The size is checked from the metadata before
/// reading and again on a read bounded to the limit plus one byte; symlinks are followed,
/// directories and other non-files are refused.
pub fn read_attachment_file(path: &Path) -> Result<(String, Vec<u8>), SessionError> {
    let meta = std::fs::metadata(path).map_err(|e| io_error(&e))?;
    if meta.is_dir() {
        return Err(SessionError::InvalidInput(
            "an attachment cannot be a directory".into(),
        ));
    }
    if !meta.is_file() {
        return Err(SessionError::InvalidInput(
            "an attachment must be a regular file".into(),
        ));
    }
    let too_big = || {
        SessionError::LimitExceeded(format!(
            "an attachment is at most {MAX_ATTACHMENT_BYTES} bytes"
        ))
    };
    if meta.len() > MAX_ATTACHMENT_BYTES as u64 {
        return Err(too_big());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| SessionError::InvalidInput("the path has no file name".into()))?;
    let file = std::fs::File::open(path).map_err(|e| io_error(&e))?;
    let mut data = Vec::new();
    file.take(MAX_ATTACHMENT_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| io_error(&e))?;
    if data.len() > MAX_ATTACHMENT_BYTES {
        return Err(too_big());
    }
    Ok((name, data))
}

/// A copy of an attachment's bytes, taken under the session lock so the (slow) write to disk
/// happens outside it, see [`write_attachment`].
pub fn attachment_bytes(
    db: &Database,
    id: Uuid,
    name: &str,
) -> Result<Zeroizing<Vec<u8>>, SessionError> {
    find_entry(db, id)?
        .attachments
        .iter()
        .find(|a| a.name == name)
        .map(|a| Zeroizing::new(a.data.clone()))
        .ok_or(SessionError::NotFound)
}

/// Writes attachment bytes to `dest` atomically. Needs no session lock. The destination comes
/// from the save dialog, so confirming an overwrite is the dialog's job.
pub fn write_attachment(dest: &Path, bytes: &[u8]) -> Result<(), SessionError> {
    mypass_kdbx::write_atomic(dest, bytes)?;
    Ok(())
}

// ---- groups ----

fn clean_name(name: &str) -> Result<&str, SessionError> {
    let name = name.trim();
    if name.is_empty() {
        Err(SessionError::InvalidInput("a group needs a name".into()))
    } else {
        Ok(name)
    }
}

/// Stores the emoji in the group's custom data, or removes it for `None` or an empty string.
fn set_emoji(group: &mut Group, emoji: Option<&str>) {
    match emoji.filter(|e| !e.is_empty()) {
        Some(e) => group.set_custom(GROUP_EMOJI, e),
        None => group.remove_custom(GROUP_EMOJI),
    }
}

pub fn create_group(
    session: &VaultSession,
    parent: Uuid,
    name: &str,
    emoji: Option<&str>,
) -> Result<Uuid, SessionError> {
    let name = clean_name(name)?;
    session.mutate(|db, now| {
        require_not_history(db, parent)?;
        let mut group = Group::new(name, now);
        set_emoji(&mut group, emoji);
        let id = group.uuid;
        db.add_group(parent, group, now)?;
        Ok(id)
    })
}

/// Renames a group and sets or clears its emoji.
pub fn update_group(
    session: &VaultSession,
    id: Uuid,
    name: &str,
    emoji: Option<&str>,
) -> Result<(), SessionError> {
    let name = clean_name(name)?;
    session.mutate_tracked(|db, now| {
        let group = db.find_group_mut(id).ok_or(SessionError::NotFound)?;
        let before = (
            group.name.clone(),
            group.custom(GROUP_EMOJI).map(str::to_owned),
        );
        group.name = name.to_owned();
        set_emoji(group, emoji);
        let changed = before
            != (
                group.name.clone(),
                group.custom(GROUP_EMOJI).map(str::to_owned),
            );
        if changed {
            group.modified = now;
        }
        Ok(((), changed))
    })
}

/// Moves a group; into itself or a descendant is `InvalidInput`.
pub fn move_group(session: &VaultSession, id: Uuid, parent: Uuid) -> Result<(), SessionError> {
    session.mutate(|db, now| {
        require_group(db, id)?;
        require_not_history(db, parent)?;
        db.move_group(id, parent, now)?;
        Ok(())
    })
}

/// Moves a group (with its entries) to the recycle bin; the root and the bin cannot be deleted.
pub fn delete_group(session: &VaultSession, id: Uuid) -> Result<(), SessionError> {
    delete_group_removed(session, id).map(|_| ())
}

/// [`delete_group`], also returning the ids of the entries that went to the bin with it
/// (collected inside the same mutation).
pub fn delete_group_removed(session: &VaultSession, id: Uuid) -> Result<Vec<Uuid>, SessionError> {
    session.mutate(|db, now| {
        require_group(db, id)?;
        if id == db.root_id() || db.meta.recycle_bin_uuid == Some(id) {
            return Err(SessionError::InvalidInput(
                "the root group and the recycle bin cannot be deleted".into(),
            ));
        }
        let removed = crate::fields::entry_ids_under(db, id);
        db.delete_group(id, now)?;
        Ok(removed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::PasskeyMeta;
    use crate::fields::{
        bin_id, CARD_CVV, CARD_EXPIRY, CARD_HOLDER, CARD_NUMBER, GROUP_EMOJI, NOTES_BODY,
    };
    use crate::fields::{
        PASSKEY_CREDENTIAL_ID, PASSKEY_PREFIX, PASSKEY_PRIVATE_KEY_PEM, PASSKEY_RELYING_PARTY,
        PASSKEY_USER_HANDLE,
    };
    use crate::queries::totp_now;
    use crate::scheduler::ManualScheduler;
    use crate::session::{SaveState, SessionDeps};
    use crate::settings::SettingsStore;
    use crate::testing::fast_database;
    use crate::testing::{
        add_test_passkey, TEST_PASSKEY_CREDENTIAL_ID, TEST_PASSKEY_PEM, TEST_PASSKEY_USER_HANDLE,
    };
    use mypass_core::entry_ext::ENTRY_TYPE_FIELD;
    use mypass_kdbx::{PlainFileIo, StringArray, Timestamp};
    use secrecy::SecretString;
    use std::fs;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;

    const NOW: Timestamp = Timestamp(1_700_000_000);
    const TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";

    struct Fx {
        dir: TempDir,
        session: VaultSession,
        root: Uuid,
    }

    fn fx_with(db: Database) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let session = VaultSession::new(SessionDeps {
            settings: SettingsStore::in_memory(),
            scheduler: ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        let root = db.root_id();
        session
            .create_vault_with(
                &dir.path().join("v.kdbx"),
                SecretString::from("pw".to_owned()),
                db,
            )
            .unwrap();
        Fx { dir, session, root }
    }

    fn fx() -> Fx {
        fx_with(fast_database("T"))
    }

    impl Fx {
        fn db<R>(&self, f: impl FnOnce(&Database) -> R) -> R {
            self.session.with_db(f).unwrap()
        }

        fn entry(&self, id: Uuid) -> Entry {
            self.db(|d| d.find_entry(id).unwrap().clone())
        }

        fn model(&self, id: Uuid) -> EntryEditModel {
            self.db(|d| edit_model(d, id).unwrap())
        }

        fn create_as(&self, kind: EntryTypeDto, edit: impl FnOnce(&mut EntryEditModel)) -> Uuid {
            let mut m = new_model(kind, self.root);
            edit(&mut m);
            save_entry(&self.session, m).unwrap()
        }

        fn create(&self, edit: impl FnOnce(&mut EntryEditModel)) -> Uuid {
            self.create_as(EntryTypeDto::Login, edit)
        }

        fn edit(&self, id: Uuid, edit: impl FnOnce(&mut EntryEditModel)) {
            let mut m = self.model(id);
            edit(&mut m);
            save_entry(&self.session, m).unwrap();
        }

        fn write_file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
            let p = self.dir.path().join(name);
            fs::write(&p, bytes).unwrap();
            p
        }
    }

    fn protected(e: &Entry, key: &str) -> Option<bool> {
        e.fields.iter().find(|f| f.key == key).map(|f| f.protected)
    }

    fn titled(title: &str) -> Entry {
        let mut e = Entry::new(NOW);
        e.set_field("Title", title, false);
        e
    }

    fn db_with(entry: Entry) -> (Database, Uuid) {
        let mut db = fast_database("T");
        let id = entry.uuid;
        let root = db.root_id();
        db.add_entry(root, entry, NOW).unwrap();
        (db, id)
    }

    #[test]
    fn new_login_saves_fields_and_urls() {
        let f = fx();
        let both = vec![
            "https://bank.test".to_owned(),
            "https://m.bank.test".to_owned(),
        ];
        let id = f.create(|m| {
            m.title = "Bank".into();
            m.username = "bob".into();
            m.password = "pw1".into();
            m.urls = both.clone();
        });
        let e = f.entry(id);
        assert_eq!(e.urls(), both);
        assert_eq!(e.field("UserName"), Some("bob"));
        assert_eq!(e.field("Title"), Some("Bank"));
        assert_eq!(protected(&e, "Password"), Some(true));
        assert_eq!(protected(&e, "UserName"), Some(false));
        assert!(e.field(ENTRY_TYPE_FIELD).is_none());
        assert_eq!(f.db(|d| d.parent_of_entry(id)), Some(f.root));
    }

    #[test]
    fn new_card_protects_number_and_cvv() {
        let f = fx();
        let id = f.create_as(EntryTypeDto::Card, |m| {
            m.title = "Visa".into();
            m.card = Some(CardEdit {
                holder: "Jane Doe".into(),
                number: "4111111111111111".into(),
                expiry: "09/28".into(),
                cvv: "123".into(),
            });
        });
        let e = f.entry(id);
        // The flags are the on-disk contract of the Swift app (KdbxEntryConvenienceTests).
        assert_eq!(protected(&e, CARD_NUMBER), Some(true));
        assert_eq!(protected(&e, CARD_CVV), Some(true));
        assert_eq!(protected(&e, CARD_HOLDER), Some(false));
        assert_eq!(protected(&e, CARD_EXPIRY), Some(false));
        assert_eq!(e.field(CARD_NUMBER), Some("4111111111111111"));
        assert_eq!(e.field(ENTRY_TYPE_FIELD), Some("Card"));
    }

    #[test]
    fn new_notes_entry_stores_body_unprotected_and_type_marker() {
        let f = fx();
        let id = f.create_as(EntryTypeDto::Notes, |m| {
            m.title = "Memo".into();
            m.notes = "# Heading\n\nbody".into();
        });
        let e = f.entry(id);
        assert_eq!(e.field(NOTES_BODY), Some("# Heading\n\nbody"));
        assert_eq!(protected(&e, NOTES_BODY), Some(false));
        assert_eq!(e.field(ENTRY_TYPE_FIELD), Some("Notes"));
        assert!(e.field("Notes").is_none());
        assert_eq!(f.model(id).notes, "# Heading\n\nbody");
    }

    #[test]
    fn login_type_removes_entry_type_field() {
        let f = fx();
        let id = f.create_as(EntryTypeDto::Files, |m| m.title = "Files".into());
        assert_eq!(f.entry(id).field(ENTRY_TYPE_FIELD), Some("Files"));
        f.edit(id, |m| m.entry_type = EntryTypeDto::Login);
        assert!(f.entry(id).field(ENTRY_TYPE_FIELD).is_none());
    }

    #[test]
    fn address_empty_title_uses_label() {
        let f = fx();
        let id = f.create_as(EntryTypeDto::Address, |m| {
            m.title = "   ".into();
            m.notes = "Delivery".into();
            if let Some(a) = m.address.as_mut() {
                a.label = "Work".into();
                a.given_name = "Ada".into();
                a.city = "Warsaw".into();
            }
        });
        let e = f.entry(id);
        assert_eq!(e.title(), "Work");
        assert_eq!(e.field(ENTRY_TYPE_FIELD), Some("Address"));
        assert_eq!(e.field("MyPass.Address.city"), Some("Warsaw"));
        assert_eq!(e.field("Notes"), Some("Delivery"));
        for key in ADDRESS_FIELDS {
            assert_eq!(
                protected(&e, &format!("{ADDRESS_PREFIX}{key}")),
                Some(true),
                "{key}"
            );
        }
        // The stored full name is not synthesised from the parts.
        assert_eq!(f.model(id).address.unwrap().full_name, "");
    }

    #[test]
    fn edit_creates_history_snapshot_and_honours_history_max_items() {
        let mut db = fast_database("T");
        db.meta.history_max_items = 3;
        let f = fx_with(db);
        let id = f.create(|m| m.title = "t0".into());
        assert!(f.entry(id).history.is_empty());
        for n in 1..=6 {
            f.edit(id, |m| m.title = format!("t{n}"));
        }
        let e = f.entry(id);
        assert_eq!(e.history.len(), 3);
        assert_eq!(e.title(), "t6");
        assert_eq!(e.history.last().map(Entry::title), Some("t5"));
    }

    #[test]
    fn unchanged_save_records_nothing() {
        let f = fx();
        let id = f.create(|m| {
            m.title = "x".into();
            m.password = "p".into();
        });
        let before = f.entry(id);
        f.session.flush().unwrap();
        f.edit(id, |_| {});
        assert_eq!(f.entry(id), before);
        assert!(f.entry(id).history.is_empty());
        assert_eq!(f.session.status().save.state, SaveState::Idle);
    }

    #[test]
    fn edit_preserves_passkey_and_unknown_custom_fields() {
        let mut e = titled("Site");
        e.set_field("KPEX_PASSKEY_CREDENTIAL_ID", "abc", false);
        e.set_field("KPEX_PASSKEY_PRIVATE_KEY_PEM", "pem", true);
        e.set_field("Recovery codes", "1111 2222", true);
        e.set_field("URL", "https://site.test", false);
        e.string_arrays.push(StringArray {
            key: "KP2A_URL".into(),
            values: vec!["https://a.test".into(), "https://b.test".into()],
        });
        let (db, id) = db_with(e);
        let f = fx_with(db);
        let model = f.model(id);
        assert_eq!(model.custom_fields.len(), 1);
        assert_eq!(model.custom_fields[0].key, "Recovery codes");
        assert!(model.urls.len() == 3);
        f.edit(id, |m| m.title = "Renamed".into());
        let e = f.entry(id);
        assert_eq!(e.title(), "Renamed");
        assert_eq!(e.field("KPEX_PASSKEY_CREDENTIAL_ID"), Some("abc"));
        assert_eq!(e.field("KPEX_PASSKEY_PRIVATE_KEY_PEM"), Some("pem"));
        assert_eq!(protected(&e, "KPEX_PASSKEY_PRIVATE_KEY_PEM"), Some(true));
        assert_eq!(e.field("Recovery codes"), Some("1111 2222"));
        assert_eq!(protected(&e, "Recovery codes"), Some(true));
        assert_eq!(
            e.string_arrays,
            vec![StringArray {
                key: "KP2A_URL".into(),
                values: vec!["https://a.test".into(), "https://b.test".into()],
            }]
        );
        assert_eq!(e.history.len(), 1);
    }

    // ---- passkeys (ports of the Swift EntryEditSaveLogicTests) ----

    fn passkey_fields(e: &Entry) -> Vec<(String, String, bool)> {
        e.fields
            .iter()
            .filter(|f| f.key.starts_with(PASSKEY_PREFIX))
            .map(|f| (f.key.clone(), f.value.clone(), f.protected))
            .collect()
    }

    /// A login with a password, a TOTP secret, a custom field and a full MyPass passkey.
    fn passkey_login() -> (Fx, Uuid) {
        let mut e = titled("Example");
        e.set_field("UserName", "alice", false);
        e.set_field("Password", "hunter2", true);
        e.set_field(OTP, TOTP_SECRET, true);
        e.set_field("Recovery code", "abc", false);
        add_test_passkey(&mut e, true);
        let (db, id) = db_with(e);
        (fx_with(db), id)
    }

    #[test]
    fn edit_preserves_passkey_fields() {
        let (f, id) = passkey_login();
        let before = passkey_fields(&f.entry(id));
        assert_eq!(before.len(), 8);
        f.edit(id, |m| m.title = "Renamed".into());
        let e = f.entry(id);
        assert_eq!(e.title(), "Renamed");
        // Byte-identical values and unchanged protection flags, in the same order.
        assert_eq!(passkey_fields(&e), before);
        for key in [
            PASSKEY_CREDENTIAL_ID,
            PASSKEY_PRIVATE_KEY_PEM,
            PASSKEY_USER_HANDLE,
        ] {
            assert_eq!(protected(&e, key), Some(true), "{key}");
        }
        assert!(e.has_passkey());
        assert_eq!(e.history.len(), 1);
    }

    #[test]
    fn edit_model_never_carries_passkey_secrets() {
        let (f, id) = passkey_login();
        let model = f.model(id);
        let json = serde_json::to_string(&model).unwrap();
        for secret in [
            TEST_PASSKEY_PEM,
            "BEGIN",
            "TestPasskeyBodyXyz",
            TEST_PASSKEY_CREDENTIAL_ID,
            TEST_PASSKEY_USER_HANDLE,
            PASSKEY_PREFIX,
        ] {
            assert!(!json.contains(secret), "{secret}");
        }
        // The read-only meta is there, and nothing asks for a removal.
        let meta = model.passkey.clone().unwrap();
        assert_eq!(meta.relying_party, "example.com");
        assert_eq!(meta.username, "alice@example.com");
        assert_eq!(meta.created, Some(1_700_000_000_000.0));
        assert!(!model.remove_passkey);
        assert!(model
            .custom_fields
            .iter()
            .all(|c| !c.key.starts_with(PASSKEY_PREFIX)));
        // A new entry has neither.
        let fresh = new_model(EntryTypeDto::Login, f.root);
        assert!(fresh.passkey.is_none() && !fresh.remove_passkey);
    }

    #[test]
    fn edit_model_without_new_fields_still_deserialises() {
        // An encrypted draft stored before `passkey` / `removePasskey` existed.
        let f = fx();
        let mut json = serde_json::to_value(new_model(EntryTypeDto::Login, f.root)).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("passkey");
        obj.remove("removePasskey");
        obj.remove("passkeyRef");
        let model: EntryEditModel = serde_json::from_value(json).unwrap();
        assert!(model.passkey.is_none() && !model.remove_passkey && model.passkey_ref.is_none());
        // A draft whose passkey meta predates `usable` loads too (as not usable; display only).
        let mut json = serde_json::to_value(new_model(EntryTypeDto::Login, f.root)).unwrap();
        json["passkey"] =
            serde_json::json!({"relyingParty": "example.com", "username": "a", "created": null});
        let model: EntryEditModel = serde_json::from_value(json).unwrap();
        assert!(!model.passkey.unwrap().usable);
    }

    #[test]
    fn kpex_keys_are_not_offered_as_custom_fields_and_cannot_be_created() {
        let (f, id) = passkey_login();
        let keys: Vec<String> = f
            .model(id)
            .custom_fields
            .into_iter()
            .map(|c| c.key)
            .collect();
        assert_eq!(keys, ["Recovery code"]);
        for key in [
            PASSKEY_PRIVATE_KEY_PEM,
            PASSKEY_CREDENTIAL_ID,
            PASSKEY_USER_HANDLE,
            "KPEX_PASSKEY_SOMETHING_NEW",
        ] {
            let mut m = f.model(id);
            m.custom_fields.push(CustomFieldEdit {
                key: key.into(),
                value: "attacker".into(),
                protected: false,
            });
            let err = save_entry(&f.session, m).unwrap_err();
            assert!(
                matches!(err, SessionError::InvalidInput(_)),
                "{key}: {err:?}"
            );
        }
        assert_eq!(
            f.entry(id).field(PASSKEY_PRIVATE_KEY_PEM),
            Some(TEST_PASSKEY_PEM)
        );
    }

    #[test]
    fn model_passkey_is_ignored_as_input() {
        let (f, id) = passkey_login();
        let before = passkey_fields(&f.entry(id));
        // Clearing or rewriting the display data changes nothing on save.
        f.edit(id, |m| {
            m.title = "Changed".into();
            m.passkey = None;
        });
        f.edit(id, |m| {
            m.passkey = Some(PasskeyMeta {
                relying_party: "evil.test".into(),
                username: "mallory".into(),
                created: None,
                usable: true,
            });
        });
        assert_eq!(passkey_fields(&f.entry(id)), before);
        // And a new entry cannot be given a passkey through it.
        let new_id = f.create(|m| {
            m.title = "Fresh".into();
            m.passkey = Some(PasskeyMeta {
                relying_party: "evil.test".into(),
                username: "mallory".into(),
                created: None,
                usable: true,
            });
        });
        assert!(passkey_fields(&f.entry(new_id)).is_empty());
    }

    #[test]
    fn remove_is_deferred_until_save_and_cancel_keeps_it() {
        let (f, id) = passkey_login();
        let before = passkey_fields(&f.entry(id));
        // The user ticks "remove" and cancels: the model is dropped, nothing is saved.
        let mut cancelled = f.model(id);
        cancelled.remove_passkey = true;
        drop(cancelled);
        assert_eq!(passkey_fields(&f.entry(id)), before);
        // A save without the flag keeps it.
        f.edit(id, |m| m.title = "Kept".into());
        assert_eq!(passkey_fields(&f.entry(id)), before);
        let history_before = f.entry(id).history.len();
        // A save with the flag removes every `KPEX_PASSKEY_*` field, including unknown ones.
        f.session
            .mutate(|db, _| {
                db.find_entry_mut(id)
                    .unwrap()
                    .set_field("KPEX_PASSKEY_FUTURE_FIELD", "x", false);
                Ok(())
            })
            .unwrap();
        f.edit(id, |m| m.remove_passkey = true);
        let e = f.entry(id);
        assert!(passkey_fields(&e).is_empty());
        assert!(!e.has_passkey());
        assert!(f.model(id).passkey.is_none());
        // The history snapshot taken by the save still holds the key.
        assert_eq!(e.history.len(), history_before + 1);
        let last = e.history.last().unwrap();
        assert_eq!(last.field(PASSKEY_PRIVATE_KEY_PEM), Some(TEST_PASSKEY_PEM));
        assert!(last.has_passkey());
    }

    #[test]
    fn remove_leaves_rest_of_entry_intact() {
        let (f, id) = passkey_login();
        f.edit(id, |m| m.remove_passkey = true);
        let e = f.entry(id);
        assert_eq!(e.title(), "Example");
        assert_eq!(e.field("UserName"), Some("alice"));
        assert_eq!(e.field("Password"), Some("hunter2"));
        assert_eq!(e.otp_source(), Some(TOTP_SECRET));
        assert_eq!(e.field("Recovery code"), Some("abc"));
        assert!(passkey_fields(&e).is_empty());
    }

    /// What a browser replace does to the entry (popup consent): a fresh key and credential id.
    fn replace_passkey(f: &Fx, id: Uuid) {
        f.session
            .mutate(|db, _| {
                let e = db.find_entry_mut(id).unwrap();
                e.set_field(PASSKEY_CREDENTIAL_ID, "BAUGBwgJCgs", true);
                e.set_field(PASSKEY_PRIVATE_KEY_PEM, "replacement-key", true);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn remove_skips_a_passkey_stored_after_the_editor_opened() {
        let (f, id) = passkey_login();
        // The editor opens and the user ticks Remove; meanwhile the browser replaces the passkey.
        let mut m = f.model(id);
        m.title = "Edited".into();
        m.remove_passkey = true;
        replace_passkey(&f, id);
        let replaced = passkey_fields(&f.entry(id));
        save_entry(&f.session, m).unwrap();
        let e = f.entry(id);
        // The rest of the edit is saved, the passkey the user never saw is kept.
        assert_eq!(e.title(), "Edited");
        assert_eq!(passkey_fields(&e), replaced);
        assert_eq!(e.field(PASSKEY_PRIVATE_KEY_PEM), Some("replacement-key"));
        // A fresh editor sees the new passkey and can remove that one.
        f.edit(id, |m| m.remove_passkey = true);
        assert!(passkey_fields(&f.entry(id)).is_empty());
    }

    #[test]
    fn remove_needs_the_reference_of_this_entrys_passkey() {
        let (f, id) = passkey_login();
        let before = passkey_fields(&f.entry(id));
        // No reference, or a made-up one: nothing is removed.
        for forged in [None, Some("0".repeat(32)), Some(String::new())] {
            f.edit(id, |m| {
                m.remove_passkey = true;
                m.passkey_ref = forged.clone();
            });
            assert_eq!(passkey_fields(&f.entry(id)), before, "{forged:?}");
        }
        // The reference of another entry's passkey does not match this one either.
        let other = f.create(|m| m.title = "Other".into());
        f.session
            .mutate(|db, _| {
                let e = db.find_entry_mut(other).unwrap();
                add_test_passkey(e, true);
                e.set_field(PASSKEY_CREDENTIAL_ID, "CQoLDA0ODxA", true);
                Ok(())
            })
            .unwrap();
        let foreign = f.model(other).passkey_ref;
        assert!(foreign.is_some());
        f.edit(id, |m| {
            m.remove_passkey = true;
            m.passkey_ref = foreign.clone();
        });
        assert_eq!(passkey_fields(&f.entry(id)), before);
        assert!(f.entry(other).has_passkey());
    }

    #[test]
    fn damaged_passkey_is_in_the_model_and_can_be_removed() {
        let mut e = titled("Broken");
        add_test_passkey(&mut e, true);
        e.set_field(PASSKEY_CREDENTIAL_ID, "not base64!", true);
        let (db, id) = db_with(e);
        let f = fx_with(db);
        let m = f.model(id);
        let meta = m.passkey.clone().unwrap();
        assert!(!meta.usable);
        assert!(m.passkey_ref.is_some());
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("not base64!") && !json.contains(TEST_PASSKEY_PEM));
        f.edit(id, |m| m.remove_passkey = true);
        assert!(passkey_fields(&f.entry(id)).is_empty());
        assert!(f.model(id).passkey.is_none());
    }

    #[test]
    fn remove_on_entry_without_passkey_is_a_no_op() {
        let f = fx();
        let id = f.create(|m| m.title = "Plain".into());
        f.session.flush().unwrap();
        let before = f.entry(id);
        f.edit(id, |m| m.remove_passkey = true);
        assert_eq!(f.entry(id), before);
        assert!(f.entry(id).history.is_empty());
    }

    #[test]
    fn reveal_refuses_passkey_fields() {
        let (f, id) = passkey_login();
        for key in [
            PASSKEY_PRIVATE_KEY_PEM,
            PASSKEY_CREDENTIAL_ID,
            PASSKEY_USER_HANDLE,
            PASSKEY_RELYING_PARTY,
        ] {
            let field = FieldRef::Custom { key: key.into() };
            let err = f.db(|d| reveal_at(d, id, &field, 0)).unwrap_err();
            assert!(
                matches!(err, SessionError::InvalidInput(_)),
                "{key}: {err:?}"
            );
        }
    }

    #[test]
    fn custom_fields_are_replaced_added_and_removed() {
        let f = fx();
        let id = f.create(|m| {
            m.title = "x".into();
            m.custom_fields = vec![
                CustomFieldEdit {
                    key: "a".into(),
                    value: "1".into(),
                    protected: false,
                },
                CustomFieldEdit {
                    key: "b".into(),
                    value: "2".into(),
                    protected: true,
                },
            ];
        });
        f.edit(id, |m| {
            m.custom_fields.remove(0);
            m.custom_fields.push(CustomFieldEdit {
                key: "c".into(),
                value: "3".into(),
                protected: false,
            });
        });
        let e = f.entry(id);
        assert!(e.field("a").is_none());
        assert_eq!(e.field("b"), Some("2"));
        assert_eq!(protected(&e, "b"), Some(true));
        assert_eq!(e.field("c"), Some("3"));
    }

    #[test]
    fn custom_field_with_reserved_key_rejected() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        for key in [
            "Password",
            "KPEX_PASSKEY_CREDENTIAL_ID",
            "KP2A_URL",
            "MyPass.CVV",
        ] {
            let mut m = f.model(id);
            m.custom_fields.push(CustomFieldEdit {
                key: key.into(),
                value: "v".into(),
                protected: false,
            });
            assert!(
                matches!(
                    save_entry(&f.session, m),
                    Err(SessionError::InvalidInput(_))
                ),
                "{key}"
            );
        }
        let mut m = f.model(id);
        let dup = CustomFieldEdit {
            key: "dup".into(),
            value: "v".into(),
            protected: false,
        };
        m.custom_fields = vec![dup.clone(), dup];
        assert!(matches!(
            save_entry(&f.session, m),
            Err(SessionError::InvalidInput(_))
        ));
        let mut m = f.model(id);
        m.custom_fields.push(CustomFieldEdit {
            key: String::new(),
            value: "v".into(),
            protected: false,
        });
        assert!(matches!(
            save_entry(&f.session, m),
            Err(SessionError::InvalidInput(_))
        ));
        assert!(f.entry(id).history.is_empty());
    }

    #[test]
    fn invalid_totp_rejected_valid_otpauth_and_base32_accepted() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        let mut m = f.model(id);
        m.totp = "!!! not a secret".into();
        assert!(matches!(
            save_entry(&f.session, m),
            Err(SessionError::InvalidInput(_))
        ));
        assert!(f.entry(id).field("otp").is_none());

        f.edit(id, |m| m.totp = TOTP_SECRET.into());
        assert_eq!(f.entry(id).field("otp"), Some(TOTP_SECRET));
        assert_eq!(protected(&f.entry(id), "otp"), Some(true));

        let uri = format!("otpauth://totp/Acme:bob?secret={TOTP_SECRET}&digits=8");
        f.edit(id, |m| m.totp = uri.clone());
        assert_eq!(f.entry(id).field("otp"), Some(uri.as_str()));

        f.edit(id, |m| m.totp = String::new());
        assert!(f.entry(id).field("otp").is_none());
    }

    #[test]
    fn adding_ninth_attachment_rejected() {
        let mut e = titled("Full");
        for n in 0..MAX_ATTACHMENTS_PER_ENTRY {
            e.attachments.push(Attachment {
                name: format!("f{n}"),
                data: vec![1],
                protected: false,
            });
        }
        let (db, id) = db_with(e);
        let f = fx_with(db);
        let path = f.write_file("extra.txt", b"x");
        let before = f.entry(id);
        let mut m = f.model(id);
        m.title = "changed".into();
        m.attachment_ops = vec![AttachmentOp::Add {
            path: path.to_string_lossy().into_owned(),
        }];
        assert!(matches!(
            save_entry(&f.session, m),
            Err(SessionError::LimitExceeded(_))
        ));
        assert_eq!(f.entry(id), before);
    }

    #[test]
    fn file_over_5_mib_rejected_5_mib_exactly_accepted() {
        let f = fx();
        let exact = f.write_file("exact.bin", &vec![7u8; MAX_ATTACHMENT_BYTES]);
        let (name, data) = read_attachment_file(&exact).unwrap();
        assert_eq!(name, "exact.bin");
        assert_eq!(data.len(), MAX_ATTACHMENT_BYTES);
        let over = f.write_file("over.bin", &vec![7u8; MAX_ATTACHMENT_BYTES + 1]);
        assert!(matches!(
            read_attachment_file(&over),
            Err(SessionError::LimitExceeded(_))
        ));
    }

    #[test]
    fn oversized_existing_attachments_survive_title_edit() {
        let mut e = titled("Legacy");
        for n in 0..10 {
            e.attachments.push(Attachment {
                name: format!("f{n}"),
                data: vec![n as u8; if n == 3 { 6 * 1024 * 1024 } else { 4 }],
                protected: false,
            });
        }
        let (db, id) = db_with(e);
        let f = fx_with(db);
        let before = f.entry(id).attachments;
        assert_eq!(f.model(id).attachments.len(), 10);
        f.edit(id, |m| m.title = "Renamed".into());
        let e = f.entry(id);
        assert_eq!(e.title(), "Renamed");
        assert_eq!(e.attachments, before);
        assert_eq!(e.attachments.len(), 10);
    }

    #[test]
    fn attachment_add_reads_name_and_bytes_from_path() {
        let f = fx();
        let path = f.write_file("note.txt", b"abc");
        let id = f.create(|m| {
            m.title = "x".into();
            m.attachment_ops = vec![AttachmentOp::Add {
                path: path.to_string_lossy().into_owned(),
            }];
        });
        let e = f.entry(id);
        assert_eq!(e.attachments.len(), 1);
        assert_eq!(e.attachments[0].name, "note.txt");
        assert_eq!(e.attachments[0].data, b"abc");
        let meta = f.model(id).attachments;
        assert_eq!(meta[0].name, "note.txt");
        assert_eq!(meta[0].size, 3.0);
    }

    #[test]
    fn attachment_directory_refused() {
        let f = fx();
        assert!(matches!(
            read_attachment_file(f.dir.path()),
            Err(SessionError::InvalidInput(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn attachment_symlink_to_file_followed() {
        let f = fx();
        let target = f.write_file("target.txt", b"real");
        let link = f.dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let (name, data) = read_attachment_file(&link).unwrap();
        assert_eq!(name, "link.txt");
        assert_eq!(data, b"real");
    }

    #[test]
    fn attachment_missing_file_is_io_error() {
        let f = fx();
        assert!(matches!(
            read_attachment_file(&f.dir.path().join("absent")),
            Err(SessionError::Io(_))
        ));
    }

    #[test]
    fn attachment_remove_by_name() {
        let mut e = titled("Files");
        for name in ["a", "b"] {
            e.attachments.push(Attachment {
                name: name.into(),
                data: vec![1],
                protected: false,
            });
        }
        let (db, id) = db_with(e);
        let f = fx_with(db);
        f.edit(id, |m| {
            m.attachment_ops = vec![AttachmentOp::Remove { name: "a".into() }];
        });
        let names: Vec<String> = f
            .entry(id)
            .attachments
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["b"]);
        let mut m = f.model(id);
        m.attachment_ops = vec![AttachmentOp::Remove { name: "zzz".into() }];
        assert_eq!(save_entry(&f.session, m), Err(SessionError::NotFound));
    }

    #[test]
    fn export_attachment_writes_exact_bytes() {
        let mut e = titled("Files");
        e.attachments.push(Attachment {
            name: "blob".into(),
            data: vec![0, 1, 2, 255],
            protected: false,
        });
        let (db, id) = db_with(e);
        let f = fx_with(db);
        let dest = f.dir.path().join("out.bin");
        let bytes = f.db(|d| attachment_bytes(d, id, "blob")).unwrap();
        write_attachment(&dest, &bytes).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), [0, 1, 2, 255]);
        assert_eq!(
            f.db(|d| attachment_bytes(d, id, "nope"))
                .map(|b| b.to_vec()),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn delete_moves_to_bin_and_restore_returns() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        delete_entry(&f.session, id).unwrap();
        assert!(f.db(|d| d.is_in_recycle_bin(id)));
        restore_entry(&f.session, id, f.root).unwrap();
        assert!(!f.db(|d| d.is_in_recycle_bin(id)));
        assert_eq!(f.db(|d| d.parent_of_entry(id)), Some(f.root));
        assert_eq!(
            restore_entry(&f.session, id, f.root),
            Err(SessionError::InvalidInput(
                "entry is not in the recycle bin".into()
            ))
        );
        assert_eq!(
            delete_entry(&f.session, Uuid::new_v4()),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn purge_tombstones() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        purge_entry(&f.session, id).unwrap();
        assert!(f.db(|d| d.find_entry(id).is_none()));
        assert!(f.db(|d| d.deleted_objects.iter().any(|o| o.uuid == id)));
    }

    #[test]
    fn empty_bin_purges_all_and_returns_count() {
        let f = fx();
        assert_eq!(empty_bin(&f.session), Ok(0));
        let ids: Vec<Uuid> = (0..3)
            .map(|n| f.create(|m| m.title = format!("e{n}")))
            .collect();
        let keep = f.create(|m| m.title = "keep".into());
        let group = create_group(&f.session, f.root, "G", None).unwrap();
        let nested = f.create(|m| {
            m.title = "nested".into();
            m.group_id = group.simple().to_string();
        });
        for id in &ids {
            delete_entry(&f.session, *id).unwrap();
        }
        delete_group(&f.session, group).unwrap();
        assert_eq!(empty_bin(&f.session), Ok(4));
        f.db(|d| {
            for id in ids.iter().chain([&nested]) {
                assert!(d.find_entry(*id).is_none());
            }
            assert!(d.find_entry(keep).is_some());
            assert!(d.find_group(group).is_none());
            let bin = d
                .meta
                .recycle_bin_uuid
                .and_then(|b| d.find_group(b))
                .unwrap();
            assert!(bin.entries.is_empty() && bin.groups.is_empty());
        });
        assert_eq!(empty_bin(&f.session), Ok(0));
    }

    #[test]
    fn move_entry_changes_group() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        let g = create_group(&f.session, f.root, "G", None).unwrap();
        move_entry(&f.session, id, g).unwrap();
        assert_eq!(f.db(|d| d.parent_of_entry(id)), Some(g));
        assert_eq!(
            move_entry(&f.session, id, Uuid::new_v4()),
            Err(SessionError::NotFound)
        );
        // Saving the edit model with another group moves too.
        f.edit(id, |m| m.group_id = f.root.simple().to_string());
        assert_eq!(f.db(|d| d.parent_of_entry(id)), Some(f.root));
    }

    #[test]
    fn set_favorite_toggles_without_history() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        set_favorite(&f.session, id, true).unwrap();
        assert!(f.entry(id).is_favorite());
        assert!(f.model(id).favorite);
        set_favorite(&f.session, id, false).unwrap();
        assert!(!f.entry(id).is_favorite());
        assert!(f.entry(id).history.is_empty());
        assert_eq!(
            set_favorite(&f.session, Uuid::new_v4(), true),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn reveal_returns_each_field_kind() {
        let f = fx();
        let id = f.create(|m| {
            m.title = "Bank".into();
            m.username = "bob".into();
            m.password = "hunter2".into();
            m.urls = vec!["https://bank.test".into()];
            m.totp = TOTP_SECRET.into();
            m.notes = "n".into();
            m.custom_fields = vec![CustomFieldEdit {
                key: "Recovery".into(),
                value: "r-1".into(),
                protected: true,
            }];
        });
        let card = f.create_as(EntryTypeDto::Card, |m| {
            m.card = Some(CardEdit {
                holder: "J".into(),
                number: "4111".into(),
                expiry: "09/28".into(),
                cvv: "7391".into(),
            });
        });
        let note = f.create_as(EntryTypeDto::Notes, |m| m.notes = "secret body".into());
        f.db(|d| {
            let r = |id, field: FieldRef| reveal(d, id, &field).unwrap();
            assert_eq!(r(id, FieldRef::Username), "bob");
            assert_eq!(r(id, FieldRef::Password), "hunter2");
            assert_eq!(r(id, FieldRef::Url), "https://bank.test");
            assert_eq!(r(id, FieldRef::NotesBody), "n");
            assert_eq!(
                r(
                    id,
                    FieldRef::Custom {
                        key: "Recovery".into()
                    }
                ),
                "r-1"
            );
            assert_eq!(r(card, FieldRef::CardCvv), "7391");
            assert_eq!(r(card, FieldRef::CardNumber), "4111");
            assert_eq!(r(note, FieldRef::NotesBody), "secret body");
            let secs = 1_700_000_000;
            let code = reveal_at(d, id, &FieldRef::Totp, secs).unwrap();
            assert_eq!(code.len(), 6);
            assert_eq!(code, totp_now(d, id, secs).unwrap().code);
            // The clock-reading variant also yields a code.
            assert_eq!(reveal(d, id, &FieldRef::Totp).unwrap().len(), 6);
            assert_eq!(
                reveal(d, id, &FieldRef::Custom { key: "nope".into() }),
                Err(SessionError::NotFound)
            );
            assert!(matches!(
                reveal(
                    d,
                    id,
                    &FieldRef::Custom {
                        key: "Password".into()
                    }
                ),
                Err(SessionError::InvalidInput(_))
            ));
            assert_eq!(
                reveal(d, card, &FieldRef::Totp),
                Err(SessionError::NotFound)
            );
        });
    }

    #[test]
    fn reveal_missing_entry_not_found() {
        let f = fx();
        f.db(|d| {
            assert_eq!(
                reveal(d, Uuid::new_v4(), &FieldRef::Password),
                Err(SessionError::NotFound)
            );
        });
    }

    #[test]
    fn history_lists_newest_first_with_changed_field_names() {
        let f = fx();
        let id = f.create(|m| {
            m.title = "A".into();
            m.password = "p1".into();
        });
        f.edit(id, |m| m.password = "p2".into());
        let items = f.db(|d| history(d, id)).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].index, 0);
        assert_eq!(items[0].title, "A");
        assert_eq!(items[0].changed_fields, ["Password"]);
        assert_eq!(items[0].modified, NOW.0 as f64 * 1000.0);

        f.edit(id, |m| m.title = "B".into());
        f.edit(id, |m| m.title = "C".into());
        let items = f.db(|d| history(d, id)).unwrap();
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["B", "A", "A"]);
        assert_eq!(items[1].changed_fields, ["Title"]);
        assert_eq!(items[2].changed_fields, ["Password"]);
        assert_eq!(items[0].changed_fields, ["Title"]);
        assert_eq!(items.iter().map(|i| i.index).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn history_of_missing_entry_is_not_found() {
        let f = fx();
        assert_eq!(
            f.db(|d| history(d, Uuid::new_v4())),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn restore_history_is_itself_snapshotted() {
        let f = fx();
        let id = f.create(|m| {
            m.title = "A".into();
            m.password = "old".into();
        });
        f.edit(id, |m| m.password = "new".into());
        assert_eq!(f.entry(id).history.len(), 1);
        restore_history(&f.session, id, 0).unwrap();
        let e = f.entry(id);
        assert_eq!(e.field("Password"), Some("old"));
        assert_eq!(e.history.len(), 2);
        // Undoable: the snapshot taken first holds the state that was replaced.
        assert_eq!(
            e.history.last().and_then(|h| h.field("Password")),
            Some("new")
        );
        assert_eq!(
            restore_history(&f.session, id, 9),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn restore_history_restores_attachments_too() {
        let f = fx();
        let path = f.write_file("a.txt", b"1");
        let id = f.create(|m| {
            m.title = "A".into();
            m.attachment_ops = vec![AttachmentOp::Add {
                path: path.to_string_lossy().into_owned(),
            }];
        });
        f.edit(id, |m| {
            m.attachment_ops = vec![AttachmentOp::Remove {
                name: "a.txt".into(),
            }];
        });
        assert!(f.entry(id).attachments.is_empty());
        restore_history(&f.session, id, 0).unwrap();
        assert_eq!(f.entry(id).attachments.len(), 1);
    }

    #[test]
    fn group_emoji_round_trip_and_clear() {
        let f = fx();
        let g = create_group(&f.session, f.root, " Work ", Some("\u{1F525}")).unwrap();
        f.db(|d| {
            let group = d.find_group(g).unwrap();
            assert_eq!(group.name, "Work");
            assert_eq!(group.custom(GROUP_EMOJI), Some("\u{1F525}"));
        });
        update_group(&f.session, g, "Play", Some("\u{1F3AE}")).unwrap();
        f.db(|d| {
            let group = d.find_group(g).unwrap();
            assert_eq!(group.name, "Play");
            assert_eq!(group.custom(GROUP_EMOJI), Some("\u{1F3AE}"));
        });
        update_group(&f.session, g, "Play", None).unwrap();
        f.db(|d| assert!(d.find_group(g).unwrap().custom(GROUP_EMOJI).is_none()));
        assert!(matches!(
            create_group(&f.session, f.root, "  ", None),
            Err(SessionError::InvalidInput(_))
        ));
        assert_eq!(
            update_group(&f.session, Uuid::new_v4(), "x", None),
            Err(SessionError::NotFound)
        );
        assert_eq!(
            create_group(&f.session, Uuid::new_v4(), "x", None),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn group_delete_goes_to_bin_with_entries() {
        let f = fx();
        let g = create_group(&f.session, f.root, "G", None).unwrap();
        let id = f.create(|m| {
            m.title = "x".into();
            m.group_id = g.simple().to_string();
        });
        delete_group(&f.session, g).unwrap();
        assert!(f.db(|d| d.is_in_recycle_bin(id)));
        assert_eq!(f.db(|d| d.parent_of_entry(id)), Some(g));
        assert_eq!(
            delete_group(&f.session, Uuid::new_v4()),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn cannot_delete_root_or_bin() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        delete_entry(&f.session, id).unwrap();
        let bin = f.db(|d| d.meta.recycle_bin_uuid).unwrap();
        assert!(matches!(
            delete_group(&f.session, f.root),
            Err(SessionError::InvalidInput(_))
        ));
        assert!(matches!(
            delete_group(&f.session, bin),
            Err(SessionError::InvalidInput(_))
        ));
        assert!(f.db(|d| d.find_group(bin).is_some()));
    }

    #[test]
    fn group_move_into_descendant_invalid() {
        let f = fx();
        let a = create_group(&f.session, f.root, "A", None).unwrap();
        let b = create_group(&f.session, a, "B", None).unwrap();
        assert!(matches!(
            move_group(&f.session, a, b),
            Err(SessionError::InvalidInput(_))
        ));
        assert!(matches!(
            move_group(&f.session, a, a),
            Err(SessionError::InvalidInput(_))
        ));
        move_group(&f.session, b, f.root).unwrap();
        f.db(|d| assert!(d.root.groups.iter().any(|g| g.uuid == b)));
    }

    #[test]
    fn edit_model_round_trips_to_identical_entry() {
        let f = fx();
        let login = f.create(|m| {
            m.title = "Bank".into();
            m.favorite = true;
            m.username = "bob".into();
            m.password = "pw".into();
            m.urls = vec!["https://a.test".into(), "https://b.test".into()];
            m.uri_match_mode = "host".into();
            m.totp = TOTP_SECRET.into();
            m.notes = "notes".into();
            m.custom_fields = vec![CustomFieldEdit {
                key: "k".into(),
                value: "v".into(),
                protected: true,
            }];
        });
        let card = f.create_as(EntryTypeDto::Card, |m| {
            m.card = Some(CardEdit {
                holder: "H".into(),
                number: "4111111111111111".into(),
                expiry: "09/28".into(),
                cvv: "1".into(),
            });
        });
        let address = f.create_as(EntryTypeDto::Address, |m| {
            if let Some(a) = m.address.as_mut() {
                a.city = "Kyiv".into();
            }
        });
        let notes = f.create_as(EntryTypeDto::Notes, |m| m.notes = "body".into());
        let files = f.create_as(EntryTypeDto::Files, |m| m.title = "F".into());
        f.session.flush().unwrap();
        for id in [login, card, address, notes, files] {
            let before = f.entry(id);
            let m = f.model(id);
            assert_eq!(m.id.as_deref(), Some(id.simple().to_string().as_str()));
            assert_eq!(save_entry(&f.session, m), Ok(id));
            assert_eq!(f.entry(id), before);
            assert!(before.history.is_empty());
            assert_eq!(f.session.status().save.state, SaveState::Idle);
        }
        assert_eq!(f.model(login).uri_match_mode, "host");
    }

    #[test]
    fn round_trip_of_foreign_entry_keeps_protection_flags() {
        let mut e = titled("Foreign");
        e.set_field("UserName", "u", true);
        e.set_field("Password", "p", false);
        e.set_field("URL", "https://x.test", true);
        e.set_field("Notes", "n", true);
        e.set_field("Odd", "1", false);
        e.set_field("MyPass.Address.stray", "s", false);
        let (db, id) = db_with(e);
        let f = fx_with(db);
        let before = f.entry(id);
        f.edit(id, |_| {});
        assert_eq!(f.entry(id), before);
        f.edit(id, |m| {
            m.username = "u2".into();
            m.password = "p2".into();
            m.urls = vec!["https://y.test".into()];
        });
        let e = f.entry(id);
        assert_eq!(protected(&e, "UserName"), Some(true));
        assert_eq!(protected(&e, "Password"), Some(false));
        assert_eq!(protected(&e, "URL"), Some(true));
        assert_eq!(e.field("Odd"), Some("1"));
        assert_eq!(e.field("MyPass.Address.stray"), Some("s"));
    }

    #[test]
    fn save_entry_missing_id_or_group_is_not_found() {
        let f = fx();
        let mut m = new_model(EntryTypeDto::Login, f.root);
        m.id = Some(Uuid::new_v4().simple().to_string());
        assert_eq!(save_entry(&f.session, m), Err(SessionError::NotFound));
        let m = new_model(EntryTypeDto::Login, Uuid::new_v4());
        assert_eq!(save_entry(&f.session, m), Err(SessionError::NotFound));
        let mut m = new_model(EntryTypeDto::Login, f.root);
        m.group_id = "not-a-uuid".into();
        assert!(matches!(
            save_entry(&f.session, m),
            Err(SessionError::InvalidInput(_))
        ));
        let mut m = new_model(EntryTypeDto::Login, f.root);
        m.uri_match_mode = "bogus".into();
        assert!(matches!(
            save_entry(&f.session, m),
            Err(SessionError::InvalidInput(_))
        ));
    }

    #[test]
    fn debug_of_the_edit_model_never_prints_values() {
        let mut m = new_model(EntryTypeDto::Card, Uuid::nil());
        m.password = "hunter2-secret".into();
        m.totp = "TOTPSECRET".into();
        m.notes = "private-notes".into();
        m.card = Some(CardEdit {
            holder: "H".into(),
            number: "4111-secret".into(),
            expiry: "e".into(),
            cvv: "7391".into(),
        });
        m.custom_fields = vec![CustomFieldEdit {
            key: "k".into(),
            value: "custom-secret".into(),
            protected: true,
        }];
        let text = format!("{m:?} {:?}", m.custom_fields);
        for secret in [
            "hunter2",
            "TOTPSECRET",
            "private-notes",
            "4111",
            "7391",
            "custom-secret",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
    }

    #[test]
    fn new_model_has_type_specific_defaults() {
        let g = Uuid::new_v4();
        let m = new_model(EntryTypeDto::Card, g);
        assert!(m.id.is_none() && m.card.is_some() && m.address.is_none());
        assert_eq!(m.group_id, g.simple().to_string());
        assert_eq!(m.uri_match_mode, "registrable-domain");
        let m = new_model(EntryTypeDto::Address, g);
        assert_eq!(m.address.map(|a| a.label), Some("Home".to_owned()));
        let m = new_model(EntryTypeDto::Login, g);
        assert!(m.card.is_none() && m.address.is_none());
    }

    // ---- branch review fixes ----

    fn history_group(f: &Fx, marker_only: bool) -> Uuid {
        f.session
            .mutate(|db, now| {
                let mut g = Group::new("H", now);
                if marker_only {
                    g.set_custom(crate::fields::GENERATOR_HISTORY_MARKER, "1");
                } else {
                    g.uuid = mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID;
                }
                let id = g.uuid;
                let root = db.root_id();
                db.add_group(root, g, now)?;
                Ok(id)
            })
            .unwrap()
    }

    fn invalid<T: std::fmt::Debug>(r: Result<T, SessionError>) {
        assert!(matches!(r, Err(SessionError::InvalidInput(_))), "{r:?}");
    }

    #[test]
    fn history_tree_is_refused_as_a_write_target() {
        for marker_only in [false, true] {
            let f = fx();
            let hist = history_group(&f, marker_only);
            let nested = f
                .session
                .mutate(|db, now| {
                    let g = Group::new("N", now);
                    let id = g.uuid;
                    db.add_group(hist, g, now)?;
                    Ok(id)
                })
                .unwrap();
            let id = f.create(|m| m.title = "x".into());
            let other = create_group(&f.session, f.root, "G", None).unwrap();
            for target in [hist, nested] {
                let mut m = new_model(EntryTypeDto::Login, target);
                m.title = "new".into();
                invalid(save_entry(&f.session, m));
                let mut m = f.model(id);
                m.group_id = target.simple().to_string();
                invalid(save_entry(&f.session, m));
                invalid(move_entry(&f.session, id, target));
                invalid(create_group(&f.session, target, "sub", None));
                invalid(move_group(&f.session, other, target));
                delete_entry(&f.session, id).unwrap();
                invalid(restore_entry(&f.session, id, target));
                restore_entry(&f.session, id, f.root).unwrap();
            }
            assert_eq!(f.db(|d| d.parent_of_entry(id)), Some(f.root));
        }
    }

    #[test]
    fn bin_uuid_pointing_at_the_root_is_no_bin() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        let root = f.root;
        f.session
            .mutate(|db, _| {
                db.meta.recycle_bin_uuid = Some(root);
                Ok(())
            })
            .unwrap();
        assert_eq!(f.db(bin_id), None);
        assert_eq!(empty_bin(&f.session), Ok(0));
        assert!(f.db(|d| d.find_entry(id).is_some()));
        f.edit(id, |m| m.title = "y".into());
    }

    #[test]
    fn same_value_operations_leave_the_session_idle() {
        let f = fx();
        let id = f.create(|m| m.title = "x".into());
        let g = create_group(&f.session, f.root, "G", Some("a")).unwrap();
        set_favorite(&f.session, id, true).unwrap();
        f.session.flush().unwrap();
        let idle = || assert_eq!(f.session.status().save.state, SaveState::Idle);
        let before = f.db(|d| d.clone());
        set_favorite(&f.session, id, true).unwrap();
        idle();
        update_group(&f.session, g, "G", Some("a")).unwrap();
        idle();
        move_entry(&f.session, id, f.root).unwrap();
        idle();
        assert!(f.db(|d| *d == before));
        set_favorite(&f.session, id, false).unwrap();
        assert_eq!(f.session.status().save.state, SaveState::Pending);
    }

    fn two_files(f: &Fx) -> (String, String) {
        let one = f.write_file("note.txt", b"one");
        std::fs::create_dir(f.dir.path().join("sub")).unwrap();
        let two = f.write_file("sub/note.txt", b"two");
        (
            one.to_string_lossy().into_owned(),
            two.to_string_lossy().into_owned(),
        )
    }

    #[test]
    fn attachment_add_of_an_existing_name_is_refused_unless_removed_first() {
        let f = fx();
        let (one, two) = two_files(&f);
        let id = f.create(|m| {
            m.title = "x".into();
            m.attachment_ops = vec![AttachmentOp::Add { path: one }];
        });
        let mut m = f.model(id);
        m.attachment_ops = vec![AttachmentOp::Add { path: two.clone() }];
        let r = save_entry(&f.session, m);
        assert!(
            matches!(&r, Err(SessionError::InvalidInput(msg)) if msg.contains("note.txt")),
            "{r:?}"
        );
        assert_eq!(f.entry(id).attachments[0].data, b"one");
        // Remove then add in one model is the replace, whatever the order of the two ops.
        let mut m = f.model(id);
        m.attachment_ops = vec![
            AttachmentOp::Add { path: two },
            AttachmentOp::Remove {
                name: "note.txt".into(),
            },
        ];
        save_entry(&f.session, m).unwrap();
        let e = f.entry(id);
        assert_eq!(e.attachments.len(), 1);
        assert_eq!(e.attachments[0].data, b"two");
    }

    #[test]
    fn two_adds_with_the_same_name_are_refused() {
        let f = fx();
        let (one, two) = two_files(&f);
        let mut m = new_model(EntryTypeDto::Login, f.root);
        m.attachment_ops = vec![
            AttachmentOp::Add { path: one },
            AttachmentOp::Add { path: two },
        ];
        let r = save_entry(&f.session, m);
        assert!(
            matches!(&r, Err(SessionError::InvalidInput(msg)) if msg.contains("note.txt")),
            "{r:?}"
        );
    }

    #[test]
    fn too_many_adds_are_refused_before_any_file_is_read() {
        let f = fx();
        // None of these files exists: reading one would be an `Io` error, not `LimitExceeded`.
        let mut m = new_model(EntryTypeDto::Login, f.root);
        m.attachment_ops = (0..=MAX_ATTACHMENTS_PER_ENTRY)
            .map(|i| AttachmentOp::Add {
                path: f
                    .dir
                    .path()
                    .join(format!("absent{i}"))
                    .to_string_lossy()
                    .into_owned(),
            })
            .collect();
        assert!(matches!(
            save_entry(&f.session, m),
            Err(SessionError::LimitExceeded(_))
        ));
    }

    #[test]
    fn duplicate_custom_keys_from_another_client_stay_saveable() {
        let mut e = titled("Dup");
        e.fields.push(mypass_kdbx::Field {
            key: "dup".into(),
            value: "first".into(),
            protected: false,
        });
        e.fields.push(mypass_kdbx::Field {
            key: "dup".into(),
            value: "second".into(),
            protected: false,
        });
        let (db, id) = db_with(e);
        let f = fx_with(db);
        f.session.flush().unwrap();
        let m = f.model(id);
        assert_eq!(m.custom_fields.len(), 1);
        assert_eq!(m.custom_fields[0].value, "first");
        // Unchanged: a no-op, the extra stays.
        let before = f.entry(id);
        save_entry(&f.session, m).unwrap();
        assert_eq!(f.entry(id), before);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        // A real change drops the extra and keeps the first.
        f.edit(id, |m| m.title = "Renamed".into());
        let e = f.entry(id);
        let dups: Vec<&str> = e
            .fields
            .iter()
            .filter(|x| x.key == "dup")
            .map(|x| x.value.as_str())
            .collect();
        assert_eq!(dups, ["first"]);
        // Editing the value to what the extra held is a real edit of the first one.
        f.edit(id, |m| m.custom_fields[0].value = "second".into());
        assert_eq!(f.entry(id).field("dup"), Some("second"));
    }

    #[test]
    fn type_change_never_overwrites_a_foreign_notes_value() {
        let mut e = titled("N");
        e.set_field(ENTRY_TYPE_FIELD, "Notes", false);
        e.set_field(NOTES_BODY, "body", false);
        e.set_field("Notes", "foreign", false);
        let (db, id) = db_with(e);
        let f = fx_with(db);
        // Unchanged text: the foreign value stays in `Notes`, the body stays in place.
        f.edit(id, |m| m.entry_type = EntryTypeDto::Login);
        let e = f.entry(id);
        assert_eq!(e.field("Notes"), Some("foreign"));
        assert_eq!(e.field(NOTES_BODY), Some("body"));
        // Edited text during a type change goes to the old key, never over `Notes`.
        let mut e = titled("N2");
        e.set_field(ENTRY_TYPE_FIELD, "Notes", false);
        e.set_field(NOTES_BODY, "body", false);
        e.set_field("Notes", "foreign", false);
        let (db, id) = db_with(e);
        let f = fx_with(db);
        f.edit(id, |m| {
            m.entry_type = EntryTypeDto::Login;
            m.notes = "edited".into();
        });
        let e = f.entry(id);
        assert_eq!(e.field("Notes"), Some("foreign"));
        assert_eq!(e.field(NOTES_BODY), Some("edited"));
        // Without a foreign value the text follows the type as before.
        let id = f.create(|m| m.notes = "plain".into());
        f.edit(id, |m| m.entry_type = EntryTypeDto::Notes);
        assert_eq!(f.entry(id).field(NOTES_BODY), Some("plain"));
    }

    #[test]
    fn history_max_size_trims_through_save_entry() {
        let f = fx();
        f.session
            .mutate(|db, _| {
                db.meta.history_max_size = 2_500;
                Ok(())
            })
            .unwrap();
        let big = |c: char| CustomFieldEdit {
            key: "k".into(),
            value: c.to_string().repeat(1_000),
            protected: false,
        };
        let id = f.create(|m| {
            m.title = "x".into();
            m.custom_fields = vec![big('a')];
        });
        for c in ['b', 'c', 'd', 'e'] {
            f.edit(id, |m| m.custom_fields = vec![big(c)]);
        }
        let e = f.entry(id);
        // Each snapshot is about 1 KiB: two fit in 2500 bytes, the older ones were dropped.
        assert_eq!(e.history.len(), 2);
        assert_eq!(e.field("k"), Some("e".repeat(1_000).as_str()));
        let newest = e.history.last().unwrap();
        assert_eq!(newest.field("k"), Some("d".repeat(1_000).as_str()));
    }
}
