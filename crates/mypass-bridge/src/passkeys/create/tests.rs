#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm
)] // test-only

//! Tasks 6 and 7: `passkeyCreate` phase one, candidate selection and `passkeyCreateConfirm`.
//! Ports of the Swift bridge tests (1308-1440, 1715-1770), adapted to R7, R8, R9 and R31.

use super::super::candidates::{create_candidates, excluded_credential_exists, passkey_entries};
use super::super::pending::PENDING_TTL;
use super::super::{PasskeyContext, PasskeyHandler, PasskeyService, TURNED_OFF};
use super::{COULD_NOT_SAVE, NO_PENDING};
use crate::protocol::{Request, Response};
use crate::testing::*;
use crate::views::wire_id;
use base64::Engine;
use mypass_core::entry_ext::{EntryExt, UriMatchMode, GENERATOR_HISTORY_GROUP_UUID};
use mypass_core::passkey::{b64url, PasskeyRecord, PasskeySigningKey};
use mypass_kdbx::test_util::FailingFileIo;
use mypass_kdbx::{Database, Entry, FileIo, FileMeta, Group, PlainFileIo, Timestamp};
use mypass_session::saving::AUTOSAVE_DEBOUNCE;
use mypass_session::session::{LockReason, SaveState};
use mypass_session::settings::{IdleTimeout, SettingsPatch, SettingsStore};
use mypass_session::testing::fast_database;
use p256::ecdsa::signature::Verifier;
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey};
use secrecy::SecretString;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::Path;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;
use uuid::Uuid;

const ORIGIN: &str = "https://example.com";
const RP: &str = "example.com";
const OLD_PEM: &str = "-----BEGIN PRIVATE KEY-----\nT0xELUtFWQ==\n-----END PRIVATE KEY-----\n";

// ---------------------------------------------------------------------------------------------
// Fixture

/// The entries of [`sample`].
struct Ids {
    /// Login `Example`, `https://example.com`, no passkey.
    login: Uuid,
    /// Login `Example Both`, `https://example.com`, a passkey for `example.com` (credential `both`).
    both: Uuid,
    /// `Sub`: `https://accounts.example.com` in host mode (no domain match for `example.com`),
    /// a passkey stored with the rpId `Example.COM` (credential `sub`).
    sub: Uuid,
    /// Never-mode `example.com` login with a passkey (credential `never`).
    never: Uuid,
    /// A binned `example.com` login with a passkey (credential `bin`).
    bin: Uuid,
    /// A Files entry with an `example.com` URL and passkey fields (credential `files`).
    files: Uuid,
    /// An Address entry with an `example.com` URL.
    address: Uuid,
    /// A generator-history entry with an `example.com` URL and a passkey (credential `history`).
    history: Uuid,
    /// `Other`: `https://other.org` with a passkey for `other.org` (credential `other`).
    other: Uuid,
    /// `Work Example`: an `example.com` login inside the subgroup `Work`.
    nested: Uuid,
}

fn entry(title: &str, url: &str) -> Entry {
    let mut e = Entry::new(Timestamp::now());
    e.set_field("Title", title, false);
    e.set_field("UserName", "someone", false);
    if !url.is_empty() {
        e.set_field("URL", url, false);
    }
    e
}

/// Stored passkey fields, the way the record codec writes them, with a placeholder PEM.
fn with_passkey(mut e: Entry, rp: &str, credential: &[u8]) -> Entry {
    PasskeyRecord {
        rp_id: rp.to_owned(),
        credential_id: credential.to_vec(),
        private_key_pem: OLD_PEM.to_owned().into(),
        user_handle: Some(b"old-handle".to_vec()),
        username: Some("old-user".into()),
        created: None,
        flags: mypass_core::passkey::BackupFlags::default(),
    }
    .write(&mut e);
    e
}

fn sample() -> (Database, Ids) {
    let now = Timestamp::now();
    let mut db = fast_database("Passkey Create");
    let root = db.root_id();
    let mut history = Group::new("Password Generator History", now);
    history.uuid = GENERATOR_HISTORY_GROUP_UUID;
    db.add_group(root, history, now).unwrap();
    let work = Group::new("Work", now);
    let work_id = work.uuid;
    db.add_group(root, work, now).unwrap();

    let login = entry("Example", "https://example.com");
    let both = with_passkey(entry("Example Both", "https://example.com"), RP, b"both");
    let mut sub = with_passkey(
        entry("Sub", "https://accounts.example.com"),
        "Example.COM",
        b"sub",
    );
    sub.set_uri_match_mode(UriMatchMode::Host);
    let mut never = with_passkey(entry("Never", "https://example.com"), RP, b"never");
    never.set_uri_match_mode(UriMatchMode::Never);
    let bin = with_passkey(entry("Binned", "https://example.com"), RP, b"bin");
    let mut files = with_passkey(entry("Docs", "https://example.com"), RP, b"files");
    files.set_field("MyPass.EntryType", "Files", false);
    let mut address = entry("Home", "https://example.com");
    address.set_field("MyPass.EntryType", "Address", false);
    let history = with_passkey(entry("Generated", "https://example.com"), RP, b"history");
    let other = with_passkey(entry("Other", "https://other.org"), "other.org", b"other");
    let nested = entry("Work Example", "https://example.com");

    let ids = Ids {
        login: login.uuid,
        both: both.uuid,
        sub: sub.uuid,
        never: never.uuid,
        bin: bin.uuid,
        files: files.uuid,
        address: address.uuid,
        history: history.uuid,
        other: other.uuid,
        nested: nested.uuid,
    };
    for e in [login, both, sub, never, bin, files, address, other] {
        db.add_entry(root, e, now).unwrap();
    }
    db.add_entry(GENERATOR_HISTORY_GROUP_UUID, history, now)
        .unwrap();
    db.add_entry(work_id, nested, now).unwrap();
    db.delete_entry(ids.bin, now).unwrap();
    (db, ids)
}

fn sample_fx() -> (Fx, Arc<PasskeyService>, Ids) {
    let (db, ids) = sample();
    let (f, svc) = passkey_fx_with_db(db);
    (f, svc, ids)
}

// ---------------------------------------------------------------------------------------------
// Requests

/// A create request as Chrome hands it over: `origin` in the proxy override, ES256 offered.
fn request_json(origin: &str, rp_id: Option<&str>, exclude: &[&[u8]]) -> Value {
    let mut v = json!({
        "challenge": b64url::encode(b"0123456789abcdef-challenge"),
        "rp": { "name": "Example" },
        "user": { "id": b64url::encode(b"user-handle-1"), "name": "alice", "displayName": "Alice" },
        "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }],
        "excludeCredentials": exclude.iter()
            .map(|id| json!({ "type": "public-key", "id": b64url::encode(id) }))
            .collect::<Vec<_>>(),
        "extensions": { "remoteDesktopClientOverride":
            { "origin": origin, "sameOriginWithAncestors": true } },
    });
    if let Some(rp) = rp_id {
        v["rp"]["id"] = json!(rp);
    }
    v
}

fn create_with(f: &Fx, id: i64, json: &str, origin: &str) -> Response {
    send(f, |token, pairing| Request::PasskeyCreate {
        token,
        request_id: id,
        request_details_json: json.to_owned(),
        origin_url: origin.to_owned(),
        pairing,
    })
}

/// A valid `example.com` create from `https://example.com`, excluding `exclude`.
fn create(f: &Fx, id: i64, exclude: &[&[u8]]) -> Response {
    let json = request_json(ORIGIN, Some(RP), exclude).to_string();
    create_with(f, id, &json, ORIGIN)
}

fn confirm(f: &Fx, id: i64, entry: Option<&str>, replace: bool) -> Response {
    send(f, |token, pairing| Request::PasskeyCreateConfirm {
        token,
        request_id: id,
        entry_id: entry.map(str::to_owned),
        replace,
        pairing,
    })
}

fn confirm_into(f: &Fx, id: i64, entry: Uuid, replace: bool) -> Response {
    confirm(f, id, Some(&wire_id(entry)), replace)
}

fn ctx<'a>(f: &'a Fx, settings: &'a SettingsStore, unlocked: bool) -> PasskeyContext<'a> {
    PasskeyContext {
        session: &f.session,
        settings,
        unlocked,
    }
}

// ---------------------------------------------------------------------------------------------
// Response helpers

/// The ids offered by a `PasskeyCreatePending`.
fn offered(r: &Response) -> Vec<Uuid> {
    let Response::PasskeyCreatePending { matches, .. } = r else {
        panic!("expected PasskeyCreatePending, got {r:?}");
    };
    matches
        .iter()
        .map(|m| Uuid::parse_str(&m.id).unwrap())
        .collect()
}

/// `(name, message)` of a failed create completion.
fn failure(r: &Response) -> (String, String) {
    match r {
        Response::PasskeyCreateComplete {
            response_json: None,
            error: Some(e),
            ..
        } => (e.name.clone(), e.message.clone()),
        other => panic!("expected a failed PasskeyCreateComplete, got {other:?}"),
    }
}

fn assert_failure(r: &Response, name: &str, message: &str) {
    assert_eq!(failure(r), (name.to_owned(), message.to_owned()), "{r:?}");
}

/// The response JSON of a successful create completion.
fn success(r: &Response) -> Value {
    match r {
        Response::PasskeyCreateComplete {
            response_json: Some(json),
            error: None,
            ..
        } => serde_json::from_str(json).unwrap(),
        other => panic!("expected a successful PasskeyCreateComplete, got {other:?}"),
    }
}

fn b64_field(v: &Value, path: &[&str]) -> Vec<u8> {
    let mut cur = v;
    for p in path {
        cur = &cur[*p];
    }
    b64url::decode(cur.as_str().unwrap()).unwrap()
}

fn snapshot(f: &Fx) -> Database {
    f.session.with_db(Clone::clone).unwrap()
}

fn entry_of(f: &Fx, id: Uuid) -> Entry {
    f.session
        .with_db(|db| db.find_entry(id).cloned())
        .unwrap()
        .unwrap()
}

/// The live entry holding `credential` (the new one a confirm wrote).
fn entry_with_credential(db: &Database, credential: &[u8]) -> Option<Entry> {
    db.entries_excluding(&[])
        .into_iter()
        .find(|e| PasskeyRecord::credential_id(e).as_deref() == Some(credential))
        .cloned()
}

fn no_kpex(db: &Database) -> bool {
    db.entries_excluding(&[])
        .iter()
        .all(|e| !e.fields.iter().any(|fl| fl.key.starts_with("KPEX_")))
}

fn keep_unlocked(f: &Fx) {
    f.settings
        .update(SettingsPatch {
            auto_lock_idle_timeout: Some(IdleTimeout::Never),
            ..SettingsPatch::default()
        })
        .unwrap();
}

// =============================================================================================
// Task 6: phase one and candidates

#[test]
fn create_phase_one_parses_routes_and_writes_nothing() {
    let (f, svc) = passkey_fx();
    let before = snapshot(&f);
    assert!(no_kpex(&before));
    let r = create(&f, 1, &[]);
    let Response::PasskeyCreatePending {
        request_id,
        rp_id,
        rp_name,
        matches,
    } = &r
    else {
        panic!("{r:?}");
    };
    assert_eq!(
        (*request_id, rp_id.as_str(), rp_name.as_str()),
        (1, RP, "Example")
    );
    assert!(matches.is_empty());
    assert_eq!(svc.pending_len(), (1, 0));
    let after = snapshot(&f);
    assert_eq!(after, before);
    assert!(no_kpex(&after));
    assert_eq!(f.session.status().save.state, SaveState::Idle);
}

#[test]
fn create_returns_pending_with_zero_matches_for_unknown_site() {
    let (f, _svc, _ids) = sample_fx();
    let json = request_json("https://unknown.org", None, &[]);
    let r = create_with(&f, 2, &json.to_string(), "https://unknown.org");
    assert!(offered(&r).is_empty());
    let Response::PasskeyCreatePending { rp_id, .. } = &r else {
        panic!()
    };
    assert_eq!(rp_id, "unknown.org");
}

#[test]
fn rp_name_falls_back_to_rp_id() {
    let (f, _svc) = passkey_fx();
    let mut json = request_json(ORIGIN, Some(RP), &[]);
    json["rp"] = json!({ "id": RP });
    let r = create_with(&f, 1, &json.to_string(), ORIGIN);
    let Response::PasskeyCreatePending { rp_name, .. } = &r else {
        panic!("{r:?}")
    };
    assert_eq!(rp_name, RP);
}

#[test]
fn create_offers_domain_matched_logins_without_passkeys() {
    let (f, _svc, ids) = sample_fx();
    let r = create(&f, 1, &[]);
    let Response::PasskeyCreatePending { matches, .. } = &r else {
        panic!("{r:?}")
    };
    let login = matches
        .iter()
        .find(|m| m.id == wire_id(ids.login))
        .expect("the plain login is offered");
    assert_eq!(login.title, "Example");
    assert!(!login.has_passkey);
    let both = matches.iter().find(|m| m.id == wire_id(ids.both)).unwrap();
    assert!(both.has_passkey);
    // Summaries never carry a URL or a secret.
    assert!(matches.iter().all(|m| m.url.is_none()));
}

#[test]
fn create_unions_rp_id_passkey_entries_with_domain_matches_without_duplicates() {
    let (f, _svc, ids) = sample_fx();
    // Domain matches first, in document order (the root's entries, then the subgroup), then the
    // rpId-only passkey entry; `both` (domain AND rpId) appears once.
    assert_eq!(
        offered(&create(&f, 1, &[])),
        vec![ids.login, ids.both, ids.nested, ids.sub]
    );
}

#[test]
fn never_bin_files_entries_are_not_candidates() {
    let (f, _svc, ids) = sample_fx();
    let got = offered(&create(&f, 1, &[]));
    for (name, id) in [
        ("never", ids.never),
        ("bin", ids.bin),
        ("files", ids.files),
        ("address", ids.address),
        ("history", ids.history),
        ("other site", ids.other),
    ] {
        assert!(!got.contains(&id), "{name}");
    }
    // The rpId lookup on its own applies the same exclusions.
    let by_rp: Vec<Uuid> = f
        .session
        .with_db(|db| passkey_entries(db, RP).iter().map(|e| e.uuid).collect())
        .unwrap();
    assert_eq!(by_rp, vec![ids.both, ids.sub]);
}

#[test]
fn rp_id_is_compared_canonically_with_stored_rp() {
    let (f, _svc, ids) = sample_fx();
    f.session
        .with_db(|db| {
            // `sub` stores `Example.COM`.
            for asked in ["example.com", "EXAMPLE.com"] {
                let found: Vec<Uuid> = passkey_entries(db, asked).iter().map(|e| e.uuid).collect();
                assert!(found.contains(&ids.sub), "{asked}");
            }
            assert!(excluded_credential_exists(
                db,
                "example.com",
                &[b"sub".to_vec()]
            ));
            // A different rpId never matches, whatever the case.
            assert!(passkey_entries(db, "other.example.com").is_empty());
            assert!(!excluded_credential_exists(
                db,
                "other.org",
                &[b"sub".to_vec()]
            ));
            // The union helper on its own: no duplicates.
            let all = create_candidates(db, ORIGIN, RP);
            let mut uniq: Vec<Uuid> = all.iter().map(|e| e.uuid).collect();
            uniq.dedup();
            assert_eq!(uniq.len(), all.len());
        })
        .unwrap();
}

#[test]
fn create_refuses_when_toggle_off_even_when_locked_check_would_apply() {
    let (f, svc) = passkey_fx();
    f.settings
        .update(SettingsPatch {
            passkey_proxy_enabled: Some(false),
            ..SettingsPatch::default()
        })
        .unwrap();
    let json = request_json(ORIGIN, Some(RP), &[]).to_string();
    let request = Request::PasskeyCreate {
        token: "tok".into(),
        request_id: 3,
        request_details_json: json,
        origin_url: ORIGIN.into(),
        pairing: Some(f.secret.clone()),
    };
    for unlocked in [false, true] {
        assert_failure(
            &svc.handle(&ctx(&f, &f.settings, unlocked), &request),
            "NotAllowedError",
            TURNED_OFF,
        );
    }
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn create_requires_unlocked_vault_and_writes_nothing() {
    let (f, svc) = passkey_fx();
    let json = request_json(ORIGIN, Some(RP), &[]).to_string();
    let request = Request::PasskeyCreate {
        token: "tok".into(),
        request_id: 3,
        request_details_json: json,
        origin_url: ORIGIN.into(),
        pairing: Some(f.secret.clone()),
    };
    // Past the gates, the handler still refuses a locked context.
    assert_eq!(
        svc.handle(&ctx(&f, &f.settings, false), &request),
        Response::Locked
    );
    f.session.lock(LockReason::Manual);
    assert_eq!(create(&f, 3, &[]), Response::Locked);
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn create_rejects_origin_mismatch_with_security_error() {
    let (f, svc) = passkey_fx();
    // The proxy override says example.com; the wire origin says otherwise.
    let json = request_json(ORIGIN, Some(RP), &[]).to_string();
    assert_failure(
        &create_with(&f, 4, &json, "https://evil.example"),
        "SecurityError",
        "Passkey request origin could not be verified",
    );
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn create_rejects_rp_id_not_authorised_before_any_state_is_created() {
    let (f, svc) = passkey_fx();
    let json = request_json(ORIGIN, Some("other.org"), &[]).to_string();
    assert_failure(
        &create_with(&f, 5, &json, ORIGIN),
        "SecurityError",
        "This site is not authorized for the requested passkey relying party",
    );
    assert_eq!(svc.pending_len(), (0, 0));
    // R3: plain http is refused the same way, localhost included.
    for origin in ["http://example.com", "http://localhost:8765"] {
        let json = request_json(origin, None, &[]).to_string();
        let (name, _) = failure(&create_with(&f, 6, &json, origin));
        assert_eq!(name, "SecurityError", "{origin}");
    }
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn create_rejects_rp_id_climbing_past_public_suffix() {
    let (f, svc) = passkey_fx();
    for (origin, rp) in [
        ("https://login.example.com", "com"),
        ("https://login.example.co.uk", "co.uk"),
    ] {
        let json = request_json(origin, Some(rp), &[]).to_string();
        let (name, _) = failure(&create_with(&f, 7, &json, origin));
        assert_eq!(name, "SecurityError", "{rp}");
    }
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn create_allows_rp_id_on_parent_registrable_domain() {
    let (f, _svc, ids) = sample_fx();
    let origin = "https://login.example.com";
    let json = request_json(origin, Some("Example.com"), &[]).to_string();
    let r = create_with(&f, 8, &json, origin);
    let Response::PasskeyCreatePending { rp_id, .. } = &r else {
        panic!("{r:?}")
    };
    // The canonical form is what is answered and stored (R13).
    assert_eq!(rp_id, RP);
    assert!(offered(&r).contains(&ids.sub));
}

#[test]
fn create_from_cross_origin_frame_is_not_allowed() {
    let (f, svc) = passkey_fx();
    let mut json = request_json(ORIGIN, Some(RP), &[]);
    json["extensions"]["remoteDesktopClientOverride"]["sameOriginWithAncestors"] = json!(false);
    let (name, _) = failure(&create_with(&f, 9, &json.to_string(), ORIGIN));
    assert_eq!(name, "NotAllowedError");
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn create_without_es256_returns_not_supported_error() {
    let (f, svc) = passkey_fx();
    let mut json = request_json(ORIGIN, Some(RP), &[]);
    json["pubKeyCredParams"] = json!([{ "type": "public-key", "alg": -257 }]);
    assert_failure(
        &create_with(&f, 10, &json.to_string(), ORIGIN),
        "NotSupportedError",
        "MyPass passkeys require ES256",
    );
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn create_garbage_json_is_invalid_passkey_request() {
    let (f, svc) = passkey_fx();
    for garbage in ["", "{", "[]", "null", r#"{"challenge":"AA"}"#] {
        assert_failure(
            &create_with(&f, 11, garbage, ORIGIN),
            "NotAllowedError",
            "Invalid passkey request",
        );
    }
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn reused_request_id_replaces_pending() {
    let (f, svc, ids) = sample_fx();
    assert!(matches!(
        create(&f, 12, &[]),
        Response::PasskeyCreatePending { .. }
    ));
    let other = request_json("https://other.org", None, &[]).to_string();
    let r = create_with(&f, 12, &other, "https://other.org");
    assert_eq!(offered(&r), vec![ids.other]);
    assert_eq!(svc.pending_len(), (1, 0));
    // The confirm runs the newer ceremony: an example.com target is no longer a candidate.
    assert_failure(
        &confirm_into(&f, 12, ids.login, false),
        "NotAllowedError",
        "Entry not found or domain mismatch",
    );
}

#[test]
fn pending_create_debug_holds_no_page_data_or_key() {
    let (f, svc) = passkey_fx();
    create(&f, 13, &[]);
    let pending = svc.take_create(13).unwrap();
    let debug = format!("{pending:?}");
    for leak in ["alice", "user-handle", "PRIVATE", "challenge\":"] {
        assert!(!debug.contains(leak), "{leak}: {debug}");
    }
}

#[test]
fn create_does_not_reset_idle_timer() {
    let (f, _svc) = passkey_fx();
    assert!(!resets_timer(&f, || {
        assert!(matches!(
            create(&f, 14, &[]),
            Response::PasskeyCreatePending { .. }
        ));
    }));
}

// =============================================================================================
// Task 7: confirm

/// Decodes the CBOR attestation object into (fmt, sig, authData).
fn attestation(object: &[u8]) -> (String, Option<Vec<u8>>, Vec<u8>) {
    use ciborium::value::Value as Cbor;
    let v: Cbor = ciborium::from_reader(object).unwrap();
    let map = v.as_map().unwrap();
    let get = |k: &str| {
        map.iter()
            .find(|(key, _)| key.as_text() == Some(k))
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    let fmt = get("fmt").as_text().unwrap().to_owned();
    let stmt = get("attStmt");
    let sig = stmt
        .as_map()
        .unwrap()
        .iter()
        .find(|(k, _)| k.as_text() == Some("sig"))
        .map(|(_, v)| v.as_bytes().unwrap().clone());
    let auth = get("authData").as_bytes().unwrap().clone();
    (fmt, sig, auth)
}

#[test]
fn confirm_writes_entry_and_returns_verifiable_response_json() {
    let (f, _svc) = passkey_fx();
    let mut json = request_json(ORIGIN, Some(RP), &[]);
    json["attestation"] = json!("direct");
    assert!(matches!(
        create_with(&f, 20, &json.to_string(), ORIGIN),
        Response::PasskeyCreatePending { .. }
    ));
    let count_before = f
        .session
        .with_db(|db| db.entries_excluding(&[]).len())
        .unwrap();
    let resp = success(&confirm(&f, 20, None, false));

    let credential = b64_field(&resp, &["rawId"]);
    assert_eq!(resp["id"], resp["rawId"]);
    let db = snapshot(&f);
    assert_eq!(db.entries_excluding(&[]).len(), count_before + 1);
    let e = entry_with_credential(&db, &credential).expect("the new entry holds the credential");
    assert_eq!(db.parent_of_entry(e.uuid), Some(db.root_id()));
    assert_eq!(e.field("Title"), Some("Example"));
    assert_eq!(e.field("UserName"), Some("alice"));
    assert_eq!(e.field("URL"), Some(ORIGIN));
    for key in mypass_core::passkey::record::KEYS {
        assert!(e.field(key).is_some(), "{key}");
    }
    for (key, protected) in [
        ("KPEX_PASSKEY_CREDENTIAL_ID", true),
        ("KPEX_PASSKEY_PRIVATE_KEY_PEM", true),
        ("KPEX_PASSKEY_USER_HANDLE", true),
        ("KPEX_PASSKEY_RELYING_PARTY", false),
        ("KPEX_PASSKEY_USERNAME", false),
    ] {
        let field = e.fields.iter().find(|fl| fl.key == key).unwrap();
        assert_eq!(field.protected, protected, "{key}");
    }
    let record = PasskeyRecord::read(&e).unwrap();
    assert_eq!(record.rp_id, RP);
    assert_eq!(record.user_handle.as_deref(), Some(&b"user-handle-1"[..]));
    assert_eq!(record.username.as_deref(), Some("alice"));
    assert!(record.created.is_some());
    assert!(e.history.is_empty());

    // The response's public key is the stored key's.
    let key = PasskeySigningKey::from_pem(&record.private_key_pem).unwrap();
    let spki = b64_field(&resp, &["response", "publicKey"]);
    assert_eq!(key.public_key_spki_der().unwrap(), spki);

    // clientDataJSON is bound to the origin and the challenge.
    let client_data = b64_field(&resp, &["response", "clientDataJSON"]);
    let cd: Value = serde_json::from_slice(&client_data).unwrap();
    assert_eq!(cd["type"], "webauthn.create");
    assert_eq!(cd["origin"], ORIGIN);
    assert_eq!(
        cd["challenge"],
        b64url::encode(b"0123456789abcdef-challenge")
    );

    // Packed self-attestation verifies with the stored key, independently of our encoder.
    let (fmt, sig, auth) = attestation(&b64_field(&resp, &["response", "attestationObject"]));
    assert_eq!(fmt, "packed");
    assert_eq!(auth, b64_field(&resp, &["response", "authenticatorData"]));
    assert_eq!(&auth[..32], Sha256::digest(RP.as_bytes()).as_slice());
    assert_eq!(auth[32], 0x5D);
    let mut signed = auth.clone();
    signed.extend_from_slice(&Sha256::digest(&client_data));
    let verifying = p256::ecdsa::VerifyingKey::from_public_key_der(&spki).unwrap();
    let signature = p256::ecdsa::DerSignature::try_from(sig.unwrap().as_slice()).unwrap();
    verifying.verify(&signed, &signature).unwrap();
    // The credential id inside authData is the stored one.
    let len = u16::from_be_bytes([auth[53], auth[54]]) as usize;
    assert_eq!(&auth[55..55 + len], credential.as_slice());

    // Durable: on disk now, with no autosave tick.
    assert_eq!(f.session.status().save.state, SaveState::Idle);
}

#[test]
fn confirm_into_chosen_entry_among_multiple_domain_matches() {
    let (f, _svc, ids) = sample_fx();
    create(&f, 21, &[]);
    let before_both = entry_of(&f, ids.both);
    let resp = success(&confirm_into(&f, 21, ids.login, false));
    let credential = b64_field(&resp, &["rawId"]);
    let login = entry_of(&f, ids.login);
    assert_eq!(PasskeyRecord::credential_id(&login), Some(credential));
    // The entry keeps its own title, user name and URL.
    assert_eq!(login.field("Title"), Some("Example"));
    assert_eq!(login.field("UserName"), Some("someone"));
    assert_eq!(entry_of(&f, ids.both), before_both);
}

#[test]
fn confirm_into_entry_in_a_subgroup_is_written() {
    let (f, _svc, ids) = sample_fx();
    create(&f, 22, &[]);
    let resp = success(&confirm_into(&f, 22, ids.nested, false));
    let nested = entry_of(&f, ids.nested);
    assert_eq!(
        PasskeyRecord::credential_id(&nested),
        Some(b64_field(&resp, &["rawId"]))
    );
}

#[test]
fn confirm_entry_outside_candidate_set_is_rejected() {
    let (f, svc, ids) = sample_fx();
    let before = snapshot(&f);
    for (name, target) in [
        ("forged", wire_id(Uuid::from_u128(0xfeed))),
        ("other site", wire_id(ids.other)),
        ("bin", wire_id(ids.bin)),
        ("files", wire_id(ids.files)),
        ("never", wire_id(ids.never)),
        ("history", wire_id(ids.history)),
        ("address", wire_id(ids.address)),
        ("not an id", "not-a-uuid".to_owned()),
        ("empty", String::new()),
    ] {
        create(&f, 23, &[]);
        assert_failure(
            &confirm(&f, 23, Some(&target), true),
            "NotAllowedError",
            "Entry not found or domain mismatch",
        );
        // The failure consumed the ceremony.
        assert_eq!(svc.pending_len(), (0, 0), "{name}");
    }
    assert_eq!(snapshot(&f), before);
    assert!(!f.ui.calls().contains(&"credential_filled".to_owned()));
}

#[test]
fn confirm_target_binned_while_dialog_open_is_rejected() {
    let (f, _svc, ids) = sample_fx();
    create(&f, 24, &[]);
    f.session
        .mutate(|db, now| {
            db.delete_entry(ids.login, now).unwrap();
            Ok(())
        })
        .unwrap();
    let before = snapshot(&f);
    assert_failure(
        &confirm_into(&f, 24, ids.login, false),
        "NotAllowedError",
        "Entry not found or domain mismatch",
    );
    assert_eq!(snapshot(&f), before);
}

#[test]
fn entry_id_in_any_case_or_simple_form_is_accepted_when_candidate() {
    let (f, _svc, ids) = sample_fx();
    let forms = [
        ids.login.hyphenated().to_string(),
        ids.login.simple().to_string().to_uppercase(),
        format!(" {} ", wire_id(ids.login)),
    ];
    for (i, form) in forms.iter().enumerate() {
        let id = 30 + i64::try_from(i).unwrap();
        create(&f, id, &[]);
        // The login holds a key after the first round: replace from then on.
        success(&confirm(&f, id, Some(form), i > 0));
    }
}

#[test]
fn replace_requires_flag_and_snapshots_history() {
    let (f, _svc, ids) = sample_fx();
    let before = snapshot(&f);
    create(&f, 40, &[]);
    assert_failure(
        &confirm_into(&f, 40, ids.both, false),
        "InvalidStateError",
        "Entry already has a passkey",
    );
    assert_eq!(snapshot(&f), before);

    let old = entry_of(&f, ids.both);
    create(&f, 41, &[]);
    let resp = success(&confirm_into(&f, 41, ids.both, true));
    let now = entry_of(&f, ids.both);
    let new_id = PasskeyRecord::credential_id(&now).unwrap();
    assert_ne!(new_id, b"both".to_vec());
    assert_eq!(new_id, b64_field(&resp, &["rawId"]));
    assert_eq!(now.history.len(), old.history.len() + 1);
    // The newest history item is the old version, old private key included.
    let newest = now.history.last().unwrap();
    assert_eq!(newest.field("KPEX_PASSKEY_PRIVATE_KEY_PEM"), Some(OLD_PEM));
    assert_eq!(PasskeyRecord::credential_id(newest), Some(b"both".to_vec()));
    assert_ne!(now.field("KPEX_PASSKEY_PRIVATE_KEY_PEM"), Some(OLD_PEM));
    // The stale user handle of the old passkey is replaced, not kept.
    assert_eq!(
        PasskeyRecord::read(&now).unwrap().user_handle.as_deref(),
        Some(&b"user-handle-1"[..])
    );
}

#[test]
fn attach_to_entry_without_passkey_adds_history_item() {
    let (f, _svc, ids) = sample_fx();
    let before = entry_of(&f, ids.login);
    create(&f, 42, &[]);
    success(&confirm_into(&f, 42, ids.login, false));
    let after = entry_of(&f, ids.login);
    assert_eq!(after.history.len(), before.history.len() + 1);
    assert!(!PasskeyRecord::has_private_key(
        after.history.last().unwrap()
    ));
}

#[test]
fn create_into_new_entry_adds_no_history() {
    let (f, _svc, _ids) = sample_fx();
    create(&f, 43, &[]);
    let resp = success(&confirm(&f, 43, None, false));
    let e = entry_with_credential(&snapshot(&f), &b64_field(&resp, &["rawId"])).unwrap();
    assert!(e.history.is_empty());
}

#[test]
fn excluded_credential_reveals_only_on_confirm() {
    let (f, svc, ids) = sample_fx();
    let before = snapshot(&f);
    // Phase one does not reveal that the excluded credential exists.
    assert!(matches!(
        create(&f, 44, &[b"both"]),
        Response::PasskeyCreatePending { .. }
    ));
    assert_failure(
        &confirm(&f, 44, None, false),
        "InvalidStateError",
        "A passkey for this account already exists",
    );
    // Never replaces either, even with consent.
    create(&f, 45, &[b"both"]);
    assert_failure(
        &confirm_into(&f, 45, ids.both, true),
        "InvalidStateError",
        "A passkey for this account already exists",
    );
    assert_eq!(snapshot(&f), before);
    assert_eq!(svc.pending_len(), (0, 0));
    // A standard-alphabet, padded id of the same bytes is the same credential.
    let mut json = request_json(ORIGIN, Some(RP), &[]);
    json["excludeCredentials"] = json!([{ "type": "public-key",
        "id": base64::engine::general_purpose::STANDARD.encode(b"both") }]);
    create_with(&f, 46, &json.to_string(), ORIGIN);
    let (name, _) = failure(&confirm(&f, 46, None, false));
    assert_eq!(name, "InvalidStateError");
}

#[test]
fn excluded_passkey_added_while_dialog_is_open_prevents_new_credential() {
    let (f, _svc, ids) = sample_fx();
    assert!(matches!(
        create(&f, 47, &[b"fresh"]),
        Response::PasskeyCreatePending { .. }
    ));
    // Another registration (or a sync) adds the excluded credential while the popup is open.
    f.session
        .mutate(|db, now| {
            db.update_entry(ids.login, now, |e| {
                *e = with_passkey(e.clone(), RP, b"fresh");
            })
            .unwrap();
            Ok(())
        })
        .unwrap();
    let before = snapshot(&f);
    assert_failure(
        &confirm(&f, 47, None, false),
        "InvalidStateError",
        "A passkey for this account already exists",
    );
    assert_eq!(snapshot(&f), before);
}

#[test]
fn excluded_check_ignores_never_mode_but_not_bin() {
    let (f, _svc, _ids) = sample_fx();
    // A never-mode entry still owns its credential.
    create(&f, 48, &[b"never"]);
    let (name, _) = failure(&confirm(&f, 48, None, false));
    assert_eq!(name, "InvalidStateError");
    // A binned passkey is not a live credential.
    create(&f, 49, &[b"bin"]);
    success(&confirm(&f, 49, None, false));
    // Another rpId's credential with the same bytes does not count.
    create(&f, 50, &[b"other"]);
    success(&confirm(&f, 50, None, false));
}

#[test]
fn confirm_twice_second_is_no_pending_request() {
    let (f, _svc, _ids) = sample_fx();
    create(&f, 51, &[]);
    success(&confirm(&f, 51, None, false));
    assert_failure(&confirm(&f, 51, None, false), "NotAllowedError", NO_PENDING);
}

#[test]
fn confirm_after_ttl_is_no_pending_request() {
    let (f, _svc, _ids) = sample_fx();
    keep_unlocked(&f);
    create(&f, 52, &[]);
    f.sched.advance(PENDING_TTL);
    assert!(f.session.is_unlocked());
    let before = snapshot(&f);
    assert_failure(&confirm(&f, 52, None, false), "NotAllowedError", NO_PENDING);
    assert_eq!(snapshot(&f), before);
}

#[test]
fn confirm_without_pending_for_unknown_id() {
    let (f, _svc, _ids) = sample_fx();
    create(&f, 53, &[]);
    assert_failure(&confirm(&f, 54, None, false), "NotAllowedError", NO_PENDING);
    // The other ceremony is untouched.
    success(&confirm(&f, 53, None, false));
}

#[test]
fn create_pending_cannot_be_approved_as_get() {
    let (f, svc, ids) = sample_fx();
    create(&f, 55, &[]);
    let r = send(&f, |token, pairing| Request::PasskeyGetApprove {
        token,
        request_id: 55,
        entry_id: wire_id(ids.both),
        pairing,
    });
    let Response::PasskeyGetComplete {
        response_json: None,
        error: Some(e),
        ..
    } = &r
    else {
        panic!("{r:?}")
    };
    assert_eq!(e.name, "NotAllowedError");
    assert!(svc.take_get(55).is_none());
    // The create ceremony is still there for its own confirm.
    assert_eq!(svc.pending_len(), (1, 0));
}

#[test]
fn confirm_after_relock_is_locked_and_writes_nothing() {
    let (f, _svc, _ids) = sample_fx();
    create(&f, 56, &[]);
    f.session.lock(LockReason::Manual);
    assert_eq!(confirm(&f, 56, None, false), Response::Locked);
    f.session
        .unlock(SecretString::from("pw".to_owned()))
        .unwrap();
    let before = snapshot(&f);
    assert_failure(&confirm(&f, 56, None, false), "NotAllowedError", NO_PENDING);
    assert_eq!(snapshot(&f), before);
}

#[test]
fn quitting_session_refuses_confirm_without_response_json() {
    let (f, svc, _ids) = sample_fx();
    create(&f, 57, &[]);
    let before = snapshot(&f);
    f.session.begin_quit();
    // The bridge gate answers first.
    assert_eq!(
        error_message(&confirm(&f, 57, None, false)),
        "MyPass is quitting"
    );
    // Past the gate, `mutate` refuses with `Quitting`.
    let request = Request::PasskeyCreateConfirm {
        token: "tok".into(),
        request_id: 57,
        entry_id: None,
        replace: false,
        pairing: Some(f.secret.clone()),
    };
    assert_failure(
        &svc.handle(&ctx(&f, &f.settings, true), &request),
        "NotAllowedError",
        COULD_NOT_SAVE,
    );
    assert_eq!(snapshot(&f), before);
    f.session.cancel_quit();
}

/// The vault file as it is on disk now.
fn on_disk(f: &Fx) -> Database {
    let path = f.session.vault_path().unwrap();
    Database::open(
        &std::fs::read(path).unwrap(),
        &SecretString::from("pw".to_owned()),
    )
    .unwrap()
}

#[test]
fn confirm_flushes_before_returning_response_json() {
    let (f, _svc, _ids) = sample_fx();
    create(&f, 58, &[]);
    // The manual scheduler never fires the 1 s autosave debounce.
    let resp = success(&confirm(&f, 58, None, false));
    let credential = b64_field(&resp, &["rawId"]);
    assert!(entry_with_credential(&on_disk(&f), &credential).is_some());
    assert_eq!(f.session.status().save.state, SaveState::Idle);
}

#[test]
fn flush_failure_returns_could_not_save() {
    let (f, _svc, _ids) = sample_fx();
    let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
    f.session.set_file_io(io.clone());
    io.fail_write_atomic(ErrorKind::PermissionDenied);
    create(&f, 59, &[]);
    let r = confirm(&f, 59, None, false);
    assert_failure(&r, "NotAllowedError", COULD_NOT_SAVE);
    assert!(!r.was_served());
    assert!(!f.ui.calls().contains(&"credential_filled".to_owned()));
    // The edit stays in memory for the autosave retry; nothing reached the disk.
    let in_memory = snapshot(&f);
    let written: Vec<Entry> = in_memory
        .entries_excluding(&[])
        .into_iter()
        .filter(|e| PasskeyRecord::read(e).is_some_and(|r| r.username.as_deref() == Some("alice")))
        .cloned()
        .collect();
    assert_eq!(written.len(), 1);
    let credential = PasskeyRecord::credential_id(&written[0]).unwrap();
    assert!(entry_with_credential(&on_disk(&f), &credential).is_none());
    assert_eq!(f.session.status().save.state, SaveState::Failed);
    // Healed, the next flush writes it.
    io.heal();
    f.session.flush().unwrap();
    assert!(entry_with_credential(&on_disk(&f), &credential).is_some());
}

/// Test-only IO whose next `write_atomic` after `arm` blocks until `release`.
#[derive(Default)]
struct GateIo {
    state: Mutex<(bool, bool, bool)>, // (armed, entered, open)
    cv: Condvar,
}

const GATE_TIMEOUT: Duration = Duration::from_secs(5);

impl GateIo {
    fn arm(&self) {
        *self.state.lock().unwrap() = (true, false, false);
    }
    fn wait_entered(&self) {
        let g = self.state.lock().unwrap();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, GATE_TIMEOUT, |s| !s.1)
            .unwrap();
        assert!(g.1, "the gated write never started");
    }
    fn release(&self) {
        self.state.lock().unwrap().2 = true;
        self.cv.notify_all();
    }
}

impl FileIo for GateIo {
    fn read(&self, path: &Path) -> mypass_kdbx::Result<Vec<u8>> {
        PlainFileIo.read(path)
    }
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> mypass_kdbx::Result<()> {
        let mut g = self.state.lock().unwrap();
        if g.0 {
            g.0 = false;
            g.1 = true;
            self.cv.notify_all();
            let (g2, _) = self
                .cv
                .wait_timeout_while(g, GATE_TIMEOUT, |s| !s.2)
                .unwrap();
            assert!(g2.2, "the gate was not released");
        } else {
            drop(g);
        }
        PlainFileIo.write_atomic(path, bytes)
    }
    fn metadata(&self, path: &Path) -> mypass_kdbx::Result<FileMeta> {
        PlainFileIo.metadata(path)
    }
}

#[test]
fn flush_waits_for_in_flight_background_save_before_responding() {
    let (f, _svc, ids) = sample_fx();
    keep_unlocked(&f);
    let gate = Arc::new(GateIo::default());
    f.session.set_file_io(gate.clone());
    // An unrelated edit, its autosave stuck mid-write on a helper thread.
    f.session
        .mutate(|db, now| {
            db.update_entry(ids.login, now, |e| e.set_field("Notes", "edited", false))
                .unwrap();
            Ok(())
        })
        .unwrap();
    gate.arm();
    let sched = f.sched.clone();
    let autosave = std::thread::spawn(move || sched.advance(AUTOSAVE_DEBOUNCE));
    gate.wait_entered();

    create(&f, 60, &[]);
    let (tx, rx) = mpsc::channel();
    let svc = f.svc.clone();
    let secret = f.secret.clone();
    let confirmer = std::thread::spawn(move || {
        let r = svc.handle(Request::PasskeyCreateConfirm {
            token: "tok".into(),
            request_id: 60,
            entry_id: None,
            replace: false,
            pairing: Some(secret),
        });
        let _ = tx.send(r);
    });
    // The confirm wrote its edit in memory but must not answer while the earlier write runs.
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    gate.release();
    let r = rx
        .recv_timeout(GATE_TIMEOUT)
        .expect("confirm answers after the write");
    confirmer.join().unwrap();
    autosave.join().unwrap();
    let credential = b64_field(&success(&r), &["rawId"]);
    let disk = on_disk(&f);
    assert!(entry_with_credential(&disk, &credential).is_some());
    assert_eq!(
        disk.find_entry(ids.login).unwrap().field("Notes"),
        Some("edited")
    );
}

#[test]
fn confirm_marks_activity_only_on_success() {
    // Success: the gesture-gated confirm resets the idle timer.
    let (f, _svc, _ids) = sample_fx();
    create(&f, 61, &[]);
    assert!(resets_timer(&f, || {
        success(&confirm(&f, 61, None, false));
    }));
    // A refused confirm does not.
    let (f, _svc, _ids) = sample_fx();
    assert!(!resets_timer(&f, || {
        failure(&confirm(&f, 62, None, false));
    }));
}

#[test]
fn confirm_calls_credential_filled_once() {
    let (f, _svc, ids) = sample_fx();
    create(&f, 63, &[]);
    success(&confirm_into(&f, 63, ids.login, false));
    // Errors after it add none.
    failure(&confirm(&f, 63, None, false));
    create(&f, 64, &[]);
    failure(&confirm_into(&f, 64, ids.other, false));
    let filled: Vec<String> =
        f.ui.calls()
            .into_iter()
            .filter(|c| c == "credential_filled")
            .collect();
    assert_eq!(filled.len(), 1);
}

#[test]
fn private_key_not_in_any_response_or_ui_call() {
    let (f, _svc, ids) = sample_fx();
    let pending = create(&f, 65, &[]);
    let done = confirm_into(&f, 65, ids.login, false);
    let pem = entry_of(&f, ids.login)
        .field("KPEX_PASSKEY_PRIVATE_KEY_PEM")
        .unwrap()
        .to_owned();
    let secret = p256::SecretKey::from_pkcs8_pem(&pem).unwrap();
    let scalar = secret.to_bytes();
    let std = base64::engine::general_purpose::STANDARD;
    let mut needles = vec![
        "PRIVATE KEY".to_owned(),
        b64url::encode(&scalar),
        std.encode(scalar),
        scalar
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    ];
    // Every base64 line of the PEM body.
    needles.extend(
        pem.lines()
            .filter(|l| !l.starts_with("-----") && l.len() > 16)
            .map(str::to_owned),
    );
    let mut haystacks = vec![
        serde_json::to_string(&pending).unwrap(),
        serde_json::to_string(&done).unwrap(),
        format!("{pending:?}"),
        format!("{done:?}"),
    ];
    haystacks.extend(f.ui.details());
    for hay in &haystacks {
        for needle in &needles {
            assert!(!hay.contains(needle.as_str()), "leak of {needle:?}");
        }
    }
    // Sanity: the response is a real success.
    success(&done);
}

#[test]
fn confirm_target_not_offered_by_phase_one_is_rejected() {
    let (f, _svc, ids) = sample_fx();
    create(&f, 66, &[]);
    // After phase one `other` becomes an example.com login: a current candidate the popup was
    // never shown, so a confirm naming it is forged or stale.
    f.session
        .mutate(|db, now| {
            db.update_entry(ids.other, now, |e| e.set_field("URL", ORIGIN, false))
                .unwrap();
            Ok(())
        })
        .unwrap();
    assert!(f
        .session
        .with_db(|db| create_candidates(db, ORIGIN, RP)
            .iter()
            .any(|e| e.uuid == ids.other))
        .unwrap());
    let before = snapshot(&f);
    assert_failure(
        &confirm_into(&f, 66, ids.other, true),
        "NotAllowedError",
        "Entry not found or domain mismatch",
    );
    assert_eq!(snapshot(&f), before);
}

/// R7 / Task 5 I1 through the real handler: a ceremony taken after a direct switch A -> B (before
/// the observer clear) refuses inside the `mutate` and leaves B untouched.
#[test]
fn confirm_after_direct_vault_switch_writes_nothing_into_the_new_vault() {
    let (f, _observed, _ids) = sample_fx();
    // Not registered with the session: stands for the window before the observer's clear runs.
    let svc = PasskeyService::new(f.sched.clone(), f.ui.clone());
    let json = request_json(ORIGIN, Some(RP), &[]).to_string();
    let create_request = Request::PasskeyCreate {
        token: "tok".into(),
        request_id: 67,
        request_details_json: json,
        origin_url: ORIGIN.into(),
        pairing: Some(f.secret.clone()),
    };
    assert!(matches!(
        svc.handle(&ctx(&f, &f.settings, true), &create_request),
        Response::PasskeyCreatePending { .. }
    ));
    let dir = tempfile::tempdir().unwrap();
    f.session
        .create_vault_with(
            &dir.path().join("b.kdbx"),
            SecretString::from("pw".to_owned()),
            fast_database("Vault B"),
        )
        .unwrap();
    let b_before = snapshot(&f);
    let confirm_request = Request::PasskeyCreateConfirm {
        token: "tok".into(),
        request_id: 67,
        entry_id: None,
        replace: false,
        pairing: Some(f.secret.clone()),
    };
    assert_failure(
        &svc.handle(&ctx(&f, &f.settings, true), &confirm_request),
        "NotAllowedError",
        NO_PENDING,
    );
    assert_eq!(snapshot(&f), b_before);
    assert!(no_kpex(&b_before));
}
