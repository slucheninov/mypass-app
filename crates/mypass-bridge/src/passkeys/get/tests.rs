#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm
)] // test-only

//! Task 8: `passkeyGet` phase one and `passkeyGetApprove`. Ports of the Swift bridge tests
//! (1475-1600, 1770-1830), adapted to R7, R11, R12 and R13 and Task 3 M2. Every assertion that
//! comes back is verified with `p256` against the public key of the PEM stored in the vault.

use super::super::create::NO_PENDING;
use super::super::pending::PendingGet;
use super::super::{PasskeyContext, PasskeyHandler, PasskeyService, TURNED_OFF};
use super::{approve_in_vault, Refusal, ENTRY_NOT_FOUND, NO_PASSKEY, SIGN_FAILED};
use crate::protocol::{Request, Response};
use crate::testing::*;
use crate::views::wire_id;
use base64::Engine;
use mypass_core::entry_ext::{EntryExt, UriMatchMode, GENERATOR_HISTORY_GROUP_UUID};
use mypass_core::passkey::{b64url, parse_get, BackupFlags, PasskeyRecord};
use mypass_kdbx::{Database, Entry, Group, Timestamp};
use mypass_session::saving::AUTOSAVE_DEBOUNCE;
use mypass_session::session::{LockReason, SaveState};
use mypass_session::settings::{SettingsPatch, SettingsStore};
use mypass_session::testing::fast_database;
use p256::ecdsa::signature::Verifier;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use secrecy::SecretString;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

const ORIGIN: &str = "https://example.com";
const RP: &str = "example.com";
const CHALLENGE: &[u8] = b"0123456789abcdef-get-challenge";

// ---------------------------------------------------------------------------------------------
// Fixture

/// The entries of [`sample`].
struct Ids {
    /// `Example A`: `https://example.com`, passkey for `example.com` (credential `cred-a`, user
    /// handle `handle-a`, backup flags 1/1).
    a: Uuid,
    /// `Example B`: `https://accounts.example.com` in host mode, a passkey stored as
    /// `Example.COM` (credential `cred-b`, no user handle, `BE=0`).
    b: Uuid,
    /// A login with a URL for `example.com` and no passkey.
    login: Uuid,
    /// A login with the rpId and credential id of a passkey but no private key.
    no_key: Uuid,
    /// Never-mode `example.com` login with a passkey (credential `never`).
    never: Uuid,
    /// A binned `example.com` passkey (credential `bin`).
    bin: Uuid,
    /// A Files entry with passkey fields for `example.com` (credential `files`).
    files: Uuid,
    /// A generator-history entry with a passkey for `example.com` (credential `history`).
    history: Uuid,
    /// `Other`: `https://other.org` with a passkey for `other.org` (credential `other`).
    other: Uuid,
}

/// A real P-256 key, PKCS#8 PEM, from a fixed scalar.
fn pem(seed: u8) -> String {
    p256::SecretKey::from_slice(&[seed; 32])
        .unwrap()
        .to_pkcs8_pem(LineEnding::LF)
        .unwrap()
        .to_string()
}

fn entry(title: &str, url: &str) -> Entry {
    let mut e = Entry::new(Timestamp::now());
    e.set_field("Title", title, false);
    e.set_field("UserName", "someone", false);
    e.set_field("URL", url, false);
    e
}

fn record(rp: &str, credential: &[u8], seed: u8) -> PasskeyRecord {
    PasskeyRecord {
        rp_id: rp.to_owned(),
        credential_id: credential.to_vec(),
        private_key_pem: pem(seed).into(),
        user_handle: Some(format!("handle-{}", seed).into_bytes()),
        username: Some("alice".into()),
        created: None,
        flags: BackupFlags::default(),
    }
}

fn with_passkey(mut e: Entry, rp: &str, credential: &[u8], seed: u8) -> Entry {
    record(rp, credential, seed).write(&mut e);
    e
}

fn sample() -> (Database, Ids) {
    let now = Timestamp::now();
    let mut db = fast_database("Passkey Get");
    let root = db.root_id();
    let mut history = Group::new("Password Generator History", now);
    history.uuid = GENERATOR_HISTORY_GROUP_UUID;
    db.add_group(root, history, now).unwrap();

    let mut a = entry("Example A", ORIGIN);
    let mut ra = record(RP, b"cred-a", 1);
    ra.user_handle = Some(b"handle-a".to_vec());
    ra.write(&mut a);
    let mut b = entry("Example B", "https://accounts.example.com");
    let mut rb = record("Example.COM", b"cred-b", 2);
    rb.user_handle = None;
    rb.flags = BackupFlags {
        eligible: false,
        state: false,
    };
    rb.write(&mut b);
    b.set_uri_match_mode(UriMatchMode::Host);
    let login = entry("Plain", ORIGIN);
    let mut no_key = entry("No Key", ORIGIN);
    no_key.set_field("KPEX_PASSKEY_RELYING_PARTY", RP, false);
    no_key.set_field(
        "KPEX_PASSKEY_CREDENTIAL_ID",
        &b64url::encode(b"nokey"),
        true,
    );
    let mut never = with_passkey(entry("Never", ORIGIN), RP, b"never", 3);
    never.set_uri_match_mode(UriMatchMode::Never);
    let bin = with_passkey(entry("Binned", ORIGIN), RP, b"bin", 4);
    let mut files = with_passkey(entry("Docs", ORIGIN), RP, b"files", 5);
    files.set_field("MyPass.EntryType", "Files", false);
    let history = with_passkey(entry("Generated", ORIGIN), RP, b"history", 6);
    let other = with_passkey(
        entry("Other", "https://other.org"),
        "other.org",
        b"other",
        7,
    );

    let ids = Ids {
        a: a.uuid,
        b: b.uuid,
        login: login.uuid,
        no_key: no_key.uuid,
        never: never.uuid,
        bin: bin.uuid,
        files: files.uuid,
        history: history.uuid,
        other: other.uuid,
    };
    for e in [a, b, login, no_key, never, bin, files, other] {
        db.add_entry(root, e, now).unwrap();
    }
    db.add_entry(GENERATOR_HISTORY_GROUP_UUID, history, now)
        .unwrap();
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

/// A get request as Chrome hands it over: `origin` in the proxy override.
fn request_json(origin: &str, rp_id: Option<&str>, allow: &[&[u8]]) -> Value {
    let mut v = json!({
        "challenge": b64url::encode(CHALLENGE),
        "allowCredentials": allow.iter()
            .map(|id| json!({ "type": "public-key", "id": b64url::encode(id) }))
            .collect::<Vec<_>>(),
        "userVerification": "preferred",
        "extensions": { "remoteDesktopClientOverride":
            { "origin": origin, "sameOriginWithAncestors": true } },
    });
    if let Some(rp) = rp_id {
        v["rpId"] = json!(rp);
    }
    v
}

fn get_with(f: &Fx, id: i64, json: &str, origin: &str) -> Response {
    send(f, |token, pairing| Request::PasskeyGet {
        token,
        request_id: id,
        request_details_json: json.to_owned(),
        origin_url: origin.to_owned(),
        pairing,
    })
}

/// A valid `example.com` get from `https://example.com`, allowing `allow`.
fn get(f: &Fx, id: i64, allow: &[&[u8]]) -> Response {
    let json = request_json(ORIGIN, Some(RP), allow).to_string();
    get_with(f, id, &json, ORIGIN)
}

fn approve_raw(f: &Fx, id: i64, entry: &str) -> Response {
    send(f, |token, pairing| Request::PasskeyGetApprove {
        token,
        request_id: id,
        entry_id: entry.to_owned(),
        pairing,
    })
}

fn approve(f: &Fx, id: i64, entry: Uuid) -> Response {
    approve_raw(f, id, &wire_id(entry))
}

fn cancel(f: &Fx, id: i64) -> Response {
    send(f, |token, pairing| Request::PasskeyCancel {
        token,
        request_id: id,
        pairing,
    })
}

fn ctx<'a>(f: &'a Fx, settings: &'a SettingsStore, unlocked: bool) -> PasskeyContext<'a> {
    PasskeyContext {
        session: &f.session,
        settings,
        unlocked,
    }
}

fn set_toggle(f: &Fx, on: bool) {
    f.settings
        .update(SettingsPatch {
            passkey_proxy_enabled: Some(on),
            ..SettingsPatch::default()
        })
        .unwrap();
}

// ---------------------------------------------------------------------------------------------
// Response helpers

/// The ids offered by a `PasskeyGetPending` (and its rpId).
fn offered(r: &Response) -> Vec<Uuid> {
    let Response::PasskeyGetPending { matches, .. } = r else {
        panic!("expected PasskeyGetPending, got {r:?}");
    };
    matches
        .iter()
        .map(|m| Uuid::parse_str(&m.id).unwrap())
        .collect()
}

fn failure(r: &Response) -> (String, String) {
    match r {
        Response::PasskeyGetComplete {
            response_json: None,
            error: Some(e),
            ..
        } => (e.name.clone(), e.message.clone()),
        other => panic!("expected a failed PasskeyGetComplete, got {other:?}"),
    }
}

fn assert_failure(r: &Response, name: &str, message: &str) {
    assert_eq!(failure(r), (name.to_owned(), message.to_owned()), "{r:?}");
}

fn success(r: &Response) -> Value {
    match r {
        Response::PasskeyGetComplete {
            response_json: Some(json),
            error: None,
            ..
        } => serde_json::from_str(json).unwrap(),
        other => panic!("expected a successful PasskeyGetComplete, got {other:?}"),
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

/// Verifies `resp` against the public key of the PEM stored on `id` NOW, for `RP`, and returns
/// the authenticator data.
fn verify(f: &Fx, id: Uuid, resp: &Value) -> Vec<u8> {
    verify_with(&entry_of(f, id), resp, RP)
}

fn verify_with(e: &Entry, resp: &Value, rp: &str) -> Vec<u8> {
    let stored = e.field("KPEX_PASSKEY_PRIVATE_KEY_PEM").unwrap();
    let public = p256::SecretKey::from_pkcs8_pem(stored)
        .unwrap()
        .public_key();
    let verifying = p256::ecdsa::VerifyingKey::from(&public);
    let auth = b64_field(resp, &["response", "authenticatorData"]);
    let client_data = b64_field(resp, &["response", "clientDataJSON"]);
    let mut signed = auth.clone();
    signed.extend_from_slice(&Sha256::digest(&client_data));
    let sig = b64_field(resp, &["response", "signature"]);
    let sig = p256::ecdsa::DerSignature::try_from(sig.as_slice()).unwrap();
    verifying.verify(&signed, &sig).expect("signature verifies");
    assert_eq!(&auth[..32], Sha256::digest(rp.as_bytes()).as_slice());
    assert_eq!(
        auth.len(),
        37,
        "no attested credential data on an assertion"
    );
    // The credential is the one stored on the entry.
    assert_eq!(
        b64_field(resp, &["rawId"]),
        PasskeyRecord::credential_id(e).unwrap()
    );
    auth
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

// =============================================================================================
// Phase one

#[test]
fn get_returns_pending_with_only_rp_matches() {
    let (f, svc, ids) = sample_fx();
    let before = snapshot(&f);
    let r = get(&f, 1, &[]);
    let Response::PasskeyGetPending {
        request_id, rp_id, ..
    } = &r
    else {
        panic!("{r:?}")
    };
    assert_eq!((*request_id, rp_id.as_str()), (1, RP));
    assert_eq!(offered(&r), vec![ids.a, ids.b]);
    assert!(!offered(&r).contains(&ids.other));
    assert_eq!(svc.pending_len(), (0, 1));
    // Phase one reads no key and writes nothing.
    assert_eq!(snapshot(&f), before);
    let pending = svc.take_get(1).unwrap();
    assert_eq!(pending.candidates, vec![ids.a, ids.b]);
}

#[test]
fn get_filters_by_allow_credentials() {
    let (f, _svc, ids) = sample_fx();
    assert_eq!(offered(&get(&f, 2, &[b"cred-b"])), vec![ids.b]);
    // An id that matches nothing does not widen the list; the matching one is still offered.
    assert_eq!(offered(&get(&f, 3, &[b"unknown", b"cred-a"])), vec![ids.a]);
    // Another site's credential id is never offered under this rpId.
    assert_failure(&get(&f, 4, &[b"other"]), "NotAllowedError", NO_PASSKEY);
}

#[test]
fn get_with_empty_allow_list_offers_all_rp_passkeys() {
    let (f, _svc, ids) = sample_fx();
    // Discoverable sign-in (create answers credProps rk:true): looked up by the rpId, compared
    // canonically, so `Example.COM` is found too.
    assert_eq!(offered(&get(&f, 5, &[])), vec![ids.a, ids.b]);
    // `allowCredentials` absent altogether is the same request.
    let mut json = request_json(ORIGIN, Some(RP), &[]);
    json.as_object_mut().unwrap().remove("allowCredentials");
    assert_eq!(
        offered(&get_with(&f, 6, &json.to_string(), ORIGIN)),
        vec![ids.a, ids.b]
    );
    // `rpId` absent defaults to the origin host.
    let json = request_json(ORIGIN, None, &[]);
    assert_eq!(
        offered(&get_with(&f, 7, &json.to_string(), ORIGIN)),
        vec![ids.a, ids.b]
    );
    // An upper-case `rpId` is canonicalised (R13) and finds the same passkeys, and the
    // assertion hashes the canonical form.
    let json = request_json(ORIGIN, Some("EXAMPLE.Com"), &[]);
    let r = get_with(&f, 8, &json.to_string(), ORIGIN);
    let Response::PasskeyGetPending { rp_id, .. } = &r else {
        panic!("{r:?}")
    };
    assert_eq!(rp_id, RP);
    assert_eq!(offered(&r), vec![ids.a, ids.b]);
    verify(&f, ids.a, &success(&approve(&f, 8, ids.a)));
}

#[test]
fn get_returns_no_passkey_for_this_site_when_none_match() {
    let (f, svc, _ids) = sample_fx();
    let json = request_json("https://unknown.net", Some("unknown.net"), &[]).to_string();
    assert_failure(
        &get_with(&f, 8, &json, "https://unknown.net"),
        "NotAllowedError",
        NO_PASSKEY,
    );
    assert_failure(&get(&f, 9, &[b"nothing"]), "NotAllowedError", NO_PASSKEY);
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn get_ignores_recycle_bin_and_never_mode_and_files_entries() {
    let (f, _svc, ids) = sample_fx();
    let got = offered(&get(&f, 10, &[]));
    for hidden in [ids.never, ids.bin, ids.files, ids.history, ids.login] {
        assert!(!got.contains(&hidden), "{hidden}");
    }
    // Even when the page names their credentials.
    for credential in [&b"never"[..], b"bin", b"files", b"history"] {
        assert_failure(&get(&f, 11, &[credential]), "NotAllowedError", NO_PASSKEY);
    }
}

#[test]
fn get_ignores_login_with_rp_but_no_private_key() {
    let (f, _svc, ids) = sample_fx();
    assert!(!offered(&get(&f, 12, &[])).contains(&ids.no_key));
    assert_failure(&get(&f, 13, &[b"nokey"]), "NotAllowedError", NO_PASSKEY);
}

#[test]
fn get_refuses_when_toggle_off() {
    let (f, svc, _ids) = sample_fx();
    set_toggle(&f, false);
    assert_failure(&get(&f, 14, &[]), "NotAllowedError", TURNED_OFF);
    // Before the lock check too (R4: the handler called directly).
    let r = svc.handle(
        &ctx(&f, &f.settings, false),
        &Request::PasskeyGet {
            token: "tok".into(),
            request_id: 15,
            request_details_json: "{}".into(),
            origin_url: ORIGIN.into(),
            pairing: None,
        },
    );
    assert_failure(&r, "NotAllowedError", TURNED_OFF);
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn get_requires_unlocked_vault() {
    let (f, svc, _ids) = sample_fx();
    let json = request_json(ORIGIN, Some(RP), &[]).to_string();
    let r = svc.handle(
        &ctx(&f, &f.settings, false),
        &Request::PasskeyGet {
            token: "tok".into(),
            request_id: 16,
            request_details_json: json,
            origin_url: ORIGIN.into(),
            pairing: None,
        },
    );
    assert_eq!(r, Response::Locked);
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn get_rejects_origin_mismatch_and_unauthorised_rp_id() {
    let (f, svc, _ids) = sample_fx();
    // The override origin is not the frame's origin.
    let json = request_json("https://evil.com", Some(RP), &[]).to_string();
    let (name, _) = failure(&get_with(&f, 17, &json, ORIGIN));
    assert_eq!(name, "SecurityError");
    // An rpId that is not a registrable suffix of the origin.
    let json = request_json("https://evil.com", Some(RP), &[]).to_string();
    let (name, _) = failure(&get_with(&f, 18, &json, "https://evil.com"));
    assert_eq!(name, "SecurityError");
    // Plain http is never served (R3).
    let json = request_json("http://example.com", Some(RP), &[]).to_string();
    let (name, _) = failure(&get_with(&f, 19, &json, "http://example.com"));
    assert_eq!(name, "SecurityError");
    // Garbage.
    assert_failure(
        &get_with(&f, 20, "not json", ORIGIN),
        "NotAllowedError",
        "Invalid passkey request",
    );
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn get_allows_rp_id_on_parent_registrable_domain() {
    let (f, _svc, ids) = sample_fx();
    let origin = "https://accounts.example.com";
    let json = request_json(origin, Some(RP), &[]).to_string();
    let r = get_with(&f, 21, &json, origin);
    assert_eq!(offered(&r), vec![ids.a, ids.b]);
    let resp = success(&approve(&f, 21, ids.a));
    verify(&f, ids.a, &resp);
    let cd: Value =
        serde_json::from_slice(&b64_field(&resp, &["response", "clientDataJSON"])).unwrap();
    assert_eq!(cd["origin"], origin);
}

#[test]
fn get_does_not_reset_idle_timer() {
    let (f, _svc, _ids) = sample_fx();
    assert!(!resets_timer(&f, || {
        assert!(matches!(
            get(&f, 22, &[]),
            Response::PasskeyGetPending { .. }
        ));
    }));
}

// =============================================================================================
// Approve

#[test]
fn approve_signs_assertion_that_verifies() {
    let (f, svc, ids) = sample_fx();
    get(&f, 30, &[]);
    let r = approve(&f, 30, ids.a);
    let resp = success(&r);
    assert_eq!(svc.pending_len(), (0, 0));

    // The shape Chrome's completeGetRequest takes (and the Swift app returned).
    assert_eq!(
        keys(&resp),
        ["clientExtensionResults", "id", "rawId", "response", "type"]
    );
    assert_eq!(
        keys(&resp["response"]),
        [
            "authenticatorData",
            "clientDataJSON",
            "signature",
            "userHandle"
        ]
    );
    assert_eq!(resp["type"], "public-key");
    assert_eq!(resp["id"], resp["rawId"]);
    assert_eq!(resp["clientExtensionResults"], json!({}));
    assert_eq!(b64_field(&resp, &["rawId"]), b"cred-a");

    let auth = verify(&f, ids.a, &resp);
    assert_eq!(auth[32], 0x1D, "UP|UV|BE|BS");

    let cd: Value =
        serde_json::from_slice(&b64_field(&resp, &["response", "clientDataJSON"])).unwrap();
    assert_eq!(cd["type"], "webauthn.get");
    assert_eq!(cd["origin"], ORIGIN);
    assert_eq!(cd["challenge"], b64url::encode(CHALLENGE));
    assert_eq!(b64_field(&resp, &["response", "userHandle"]), b"handle-a");
    let stored = PasskeyRecord::read(&entry_of(&f, ids.a)).unwrap();
    assert_eq!(stored.user_handle.as_deref(), Some(&b"handle-a"[..]));
}

#[test]
fn approve_uses_stored_backup_flags() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 31, &[]);
    let resp = success(&approve(&f, 31, ids.b));
    // `Example.COM` was stored; the assertion hashes the canonical rpId (R13).
    let auth = verify(&f, ids.b, &resp);
    assert_eq!(auth[32], 0x05, "UP|UV, BE=0 so no BS");
    // No stored user handle: `userHandle` is omitted.
    assert!(resp["response"].get("userHandle").is_none());
}

#[test]
fn approve_without_pending_fails() {
    let (f, _svc, ids) = sample_fx();
    let before = snapshot(&f);
    assert_failure(&approve(&f, 32, ids.a), "NotAllowedError", NO_PENDING);
    assert_eq!(snapshot(&f), before);
}

#[test]
fn approve_twice_second_is_no_pending_request() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 33, &[]);
    verify(&f, ids.a, &success(&approve(&f, 33, ids.a)));
    assert_failure(&approve(&f, 33, ids.a), "NotAllowedError", NO_PENDING);
}

#[test]
fn approve_entry_for_other_rp_is_rejected() {
    let (f, svc, ids) = sample_fx();
    get(&f, 34, &[]);
    assert_failure(
        &approve(&f, 34, ids.other),
        "NotAllowedError",
        ENTRY_NOT_FOUND,
    );
    // Consumed.
    assert_eq!(svc.pending_len(), (0, 0));
    // Every hidden entry is refused the same way, as is an id that is no id.
    for hidden in [
        ids.never,
        ids.bin,
        ids.files,
        ids.history,
        ids.no_key,
        ids.login,
    ] {
        get(&f, 35, &[]);
        assert_failure(&approve(&f, 35, hidden), "NotAllowedError", ENTRY_NOT_FOUND);
    }
    get(&f, 36, &[]);
    assert_failure(
        &approve_raw(&f, 36, "not-a-uuid"),
        "NotAllowedError",
        ENTRY_NOT_FOUND,
    );
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn approve_entry_not_offered_by_phase_one_is_rejected() {
    let (f, _svc, ids) = sample_fx();
    // The page asked for `cred-a` only: `b` is a passkey for the rpId but was never offered.
    get(&f, 37, &[b"cred-a"]);
    assert_failure(&approve(&f, 37, ids.b), "NotAllowedError", ENTRY_NOT_FOUND);
    // `other` becomes an example.com passkey after phase one: a current match the popup never saw.
    get(&f, 38, &[]);
    f.session
        .mutate(|db, now| {
            db.update_entry(ids.other, now, |e| {
                e.set_field("KPEX_PASSKEY_RELYING_PARTY", RP, false);
            })
            .unwrap();
            Ok(())
        })
        .unwrap();
    assert_failure(
        &approve(&f, 38, ids.other),
        "NotAllowedError",
        ENTRY_NOT_FOUND,
    );
}

#[test]
fn approve_entry_moved_to_other_rp_or_binned_after_get_is_rejected() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 39, &[]);
    f.session
        .mutate(|db, now| {
            db.update_entry(ids.a, now, |e| {
                e.set_field("KPEX_PASSKEY_RELYING_PARTY", "other.org", false);
            })
            .unwrap();
            Ok(())
        })
        .unwrap();
    assert_failure(&approve(&f, 39, ids.a), "NotAllowedError", ENTRY_NOT_FOUND);

    get(&f, 40, &[]);
    f.session
        .mutate(|db, now| {
            db.delete_entry(ids.b, now).unwrap();
            Ok(())
        })
        .unwrap();
    assert_failure(&approve(&f, 40, ids.b), "NotAllowedError", ENTRY_NOT_FOUND);
}

/// R11: a replace between get and approve changes the credential id; the page asked for the old
/// one, so nothing is signed.
#[test]
fn approve_rechecks_allow_credentials_after_credential_changed() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 41, &[b"cred-a"]);
    f.session
        .mutate(|db, now| {
            db.update_entry(ids.a, now, |e| record(RP, b"cred-new", 9).write(e))
                .unwrap();
            Ok(())
        })
        .unwrap();
    let before_accessed = entry_of(&f, ids.a).accessed;
    assert_failure(
        &approve(&f, 41, ids.a),
        "NotAllowedError",
        "The passkey is not one the site asked for",
    );
    assert_eq!(entry_of(&f, ids.a).accessed, before_accessed);

    // With an empty allow list the new credential is the entry's passkey for the rpId: it signs,
    // with the new key and id.
    get(&f, 42, &[]);
    let resp = success(&approve(&f, 42, ids.a));
    verify(&f, ids.a, &resp);
    assert_eq!(b64_field(&resp, &["rawId"]), b"cred-new");
}

/// Task 3 M2 at the function level: an entry whose stored rp does not canonicalise to the pending
/// rpId is refused, whatever the candidate list says.
#[test]
fn approve_in_vault_binds_the_key_to_the_pending_rp() {
    let (db, ids) = sample();
    let json = request_json("https://other.org", Some("other.org"), &[]).to_string();
    let request = parse_get("https://other.org", &json).unwrap();
    // A forged candidate list naming an example.com passkey under an other.org ceremony.
    let pending = PendingGet::new(&db, request.clone(), vec![ids.a, ids.other]);
    assert_eq!(
        approve_in_vault(&db, &pending, ids.a),
        Err(Refusal::NotFound)
    );
    let resp: Value =
        serde_json::from_str(&approve_in_vault(&db, &pending, ids.other).unwrap()).unwrap();
    verify_with(db.find_entry(ids.other).unwrap(), &resp, "other.org");
    // Another vault: refused before anything is read.
    let other_vault = fast_database("Elsewhere");
    let pending = PendingGet::new(&other_vault, request, vec![ids.other]);
    assert_eq!(
        approve_in_vault(&db, &pending, ids.other),
        Err(Refusal::OtherVault)
    );
}

#[test]
fn approve_with_unusable_key_fails_to_sign_and_consumes_pending() {
    let (f, svc, ids) = sample_fx();
    get(&f, 43, &[]);
    f.session
        .mutate(|db, now| {
            db.update_entry(ids.a, now, |e| {
                e.set_field(
                    "KPEX_PASSKEY_PRIVATE_KEY_PEM",
                    "-----BEGIN PRIVATE KEY-----\nT0xELUtFWQ==\n-----END PRIVATE KEY-----\n",
                    true,
                );
            })
            .unwrap();
            Ok(())
        })
        .unwrap();
    assert_failure(&approve(&f, 43, ids.a), "NotAllowedError", SIGN_FAILED);
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn approve_after_lock_writes_and_signs_nothing() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 44, &[]);
    let before = snapshot(&f);
    f.session.lock(LockReason::Manual);
    assert_eq!(approve(&f, 44, ids.a), Response::Locked);
    f.session
        .unlock(SecretString::from("pw".to_owned()))
        .unwrap();
    // The lock dropped the ceremony: nothing to approve, nothing signed, nothing touched.
    assert_failure(&approve(&f, 44, ids.a), "NotAllowedError", NO_PENDING);
    assert_eq!(snapshot(&f), before);
    assert!(f.ui.calls().iter().all(|c| c != "credential_filled"));
}

#[test]
fn approve_after_direct_vault_switch_signs_nothing_in_the_new_vault() {
    let (f, _observed, ids) = sample_fx();
    // Not registered with the session: stands for the window before the observer's clear runs.
    let svc = PasskeyService::new(f.sched.clone(), f.ui.clone());
    let json = request_json(ORIGIN, Some(RP), &[]).to_string();
    let r = svc.handle(
        &ctx(&f, &f.settings, true),
        &Request::PasskeyGet {
            token: "tok".into(),
            request_id: 45,
            request_details_json: json,
            origin_url: ORIGIN.into(),
            pairing: None,
        },
    );
    assert!(matches!(r, Response::PasskeyGetPending { .. }));
    // Vault B holds an entry with the very same UUID and a passkey for the same rpId.
    let (mut b, _) = sample();
    let now = Timestamp::now();
    let root = b.root_id();
    let mut twin = with_passkey(entry("Twin", ORIGIN), RP, b"twin", 8);
    twin.uuid = ids.a;
    b.add_entry(root, twin, now).unwrap();
    let dir = tempfile::tempdir().unwrap();
    f.session
        .create_vault_with(
            &dir.path().join("b.kdbx"),
            SecretString::from("pw".to_owned()),
            b,
        )
        .unwrap();
    assert!(f
        .session
        .with_db(|db| db.find_entry(ids.a).is_some())
        .unwrap());
    let b_before = snapshot(&f);
    let r = svc.handle(
        &ctx(&f, &f.settings, true),
        &Request::PasskeyGetApprove {
            token: "tok".into(),
            request_id: 45,
            entry_id: wire_id(ids.a),
            pairing: None,
        },
    );
    assert_failure(&r, "NotAllowedError", NO_PENDING);
    assert_eq!(snapshot(&f), b_before);
}

#[test]
fn approve_touches_access_time_without_marking_dirty() {
    let (f, _svc, ids) = sample_fx();
    let on_disk_before = on_disk(&f);
    let before = entry_of(&f, ids.a);
    get(&f, 46, &[]);
    verify(&f, ids.a, &success(&approve(&f, 46, ids.a)));
    let after = entry_of(&f, ids.a);
    assert_ne!(after.accessed, before.accessed);
    assert_eq!(after.usage_count, before.usage_count + 1);
    // Nothing else changed on the entry: no history item, same modification time and fields.
    assert_eq!(after.history, before.history);
    assert_eq!(after.modified, before.modified);
    assert_eq!(after.fields, before.fields);
    // Not dirty: no save is pending and none happens after the debounce.
    assert_eq!(f.session.status().save.state, SaveState::Idle);
    f.sched.advance(AUTOSAVE_DEBOUNCE + Duration::from_secs(1));
    assert_eq!(f.session.status().save.state, SaveState::Idle);
    assert_eq!(on_disk(&f), on_disk_before);
    // The UI hears about the use once.
    let filled =
        f.ui.calls()
            .into_iter()
            .filter(|c| c == "credential_filled")
            .count();
    assert_eq!(filled, 1);
}

#[test]
fn approve_resets_idle_timer_only_on_success() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 47, &[]);
    assert!(resets_timer(&f, || {
        verify(&f, ids.a, &success(&approve(&f, 47, ids.a)));
    }));
    let (f, _svc, ids) = sample_fx();
    get(&f, 48, &[]);
    assert!(!resets_timer(&f, || {
        failure(&approve(&f, 48, ids.other));
    }));
    let (f, _svc, ids) = sample_fx();
    assert!(!resets_timer(&f, || {
        failure(&approve(&f, 49, ids.a));
    }));
}

#[test]
fn approve_refuses_when_toggle_turned_off_after_get() {
    let (f, svc, ids) = sample_fx();
    get(&f, 50, &[]);
    // Off set in the app's Settings window: the table is not cleared, approve re-checks.
    set_toggle(&f, false);
    assert_eq!(svc.pending_len(), (0, 1));
    assert_failure(&approve(&f, 50, ids.a), "NotAllowedError", TURNED_OFF);
    set_toggle(&f, true);
    assert_failure(&approve(&f, 50, ids.a), "NotAllowedError", NO_PENDING);
}

#[test]
fn get_then_cancel_clears_pending() {
    let (f, svc, _ids) = sample_fx();
    get(&f, 51, &[]);
    assert_eq!(svc.pending_len(), (0, 1));
    assert_eq!(
        cancel(&f, 51),
        Response::PasskeyCancelled { request_id: 51 }
    );
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn cancel_with_get_id_cannot_approve() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 52, &[]);
    cancel(&f, 52);
    let before = snapshot(&f);
    assert_failure(&approve(&f, 52, ids.a), "NotAllowedError", NO_PENDING);
    assert_eq!(snapshot(&f), before);
}

#[test]
fn reused_request_id_replaces_pending_get() {
    let (f, _svc, ids) = sample_fx();
    get(&f, 53, &[b"cred-b"]);
    get(&f, 53, &[b"cred-a"]);
    assert_failure(&approve(&f, 53, ids.b), "NotAllowedError", ENTRY_NOT_FOUND);
    get(&f, 53, &[b"cred-a"]);
    verify(&f, ids.a, &success(&approve(&f, 53, ids.a)));
}

#[test]
fn signature_counter_stays_zero_across_two_assertions() {
    let (f, _svc, ids) = sample_fx();
    let mut signatures = Vec::new();
    for id in [54, 55] {
        get(&f, id, &[]);
        let resp = success(&approve(&f, id, ids.a));
        let auth = verify(&f, ids.a, &resp);
        assert_eq!(&auth[33..37], &[0, 0, 0, 0]);
        signatures.push(b64_field(&resp, &["response", "signature"]));
    }
    // Same key, same challenge, same authData: RFC 6979 gives the same signature, which shows
    // nothing (no counter) moved between the two.
    assert_eq!(signatures[0], signatures[1]);
    // No counter field was written to the entry.
    assert!(entry_of(&f, ids.a)
        .fields
        .iter()
        .all(|fl| !fl.key.to_ascii_lowercase().contains("counter")));
}

#[test]
fn no_passkey_response_contains_pem_or_scalar() {
    let (f, _svc, ids) = sample_fx();
    // A full create + get run, covering all seven passkey responses.
    let create_json = json!({
        "challenge": b64url::encode(b"0123456789abcdef-challenge"),
        "rp": { "id": RP, "name": "Example" },
        "user": { "id": b64url::encode(b"user-handle-1"), "name": "alice", "displayName": "Alice" },
        "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }],
        "extensions": { "remoteDesktopClientOverride":
            { "origin": ORIGIN, "sameOriginWithAncestors": true } },
    })
    .to_string();
    let mut responses = vec![
        send(&f, |token, pairing| Request::GetPasskeyProxyStatus {
            token,
            pairing,
        }),
        send(&f, |token, pairing| Request::SetPasskeyProxyEnabled {
            token,
            enabled: true,
            pairing,
        }),
        send(&f, |token, pairing| Request::PasskeyCreate {
            token,
            request_id: 60,
            request_details_json: create_json.clone(),
            origin_url: ORIGIN.to_owned(),
            pairing,
        }),
        send(&f, |token, pairing| Request::PasskeyCreateConfirm {
            token,
            request_id: 60,
            entry_id: Some(wire_id(ids.login)),
            replace: false,
            pairing,
        }),
    ];
    let created = entry_of(&f, ids.login);
    let get_pending = get(&f, 61, &[]);
    assert!(offered(&get_pending).contains(&ids.login));
    let approved = approve(&f, 61, ids.login);
    verify(&f, ids.login, &success(&approved));
    get(&f, 62, &[]);
    responses.extend([get_pending, approved, cancel(&f, 62)]);
    let names: Vec<String> = responses
        .iter()
        .map(|r| {
            serde_json::to_value(r).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        names,
        [
            "passkeyProxyStatus",
            "passkeyProxyEnabled",
            "passkeyCreatePending",
            "passkeyCreateComplete",
            "passkeyGetPending",
            "passkeyGetComplete",
            "passkeyCancelled"
        ]
    );

    let std = base64::engine::general_purpose::STANDARD;
    let mut needles = vec!["PRIVATE KEY".to_owned()];
    for e in [created, entry_of(&f, ids.a), entry_of(&f, ids.b)] {
        let pem = e.field("KPEX_PASSKEY_PRIVATE_KEY_PEM").unwrap().to_owned();
        let scalar = p256::SecretKey::from_pkcs8_pem(&pem).unwrap().to_bytes();
        needles.push(b64url::encode(&scalar));
        needles.push(std.encode(scalar));
        needles.push(scalar.iter().map(|b| format!("{b:02x}")).collect());
        needles.extend(
            pem.lines()
                .filter(|l| !l.starts_with("-----") && l.len() > 16)
                .map(str::to_owned),
        );
    }
    let mut haystacks: Vec<String> = responses
        .iter()
        .flat_map(|r| [serde_json::to_string(r).unwrap(), format!("{r:?}")])
        .collect();
    haystacks.extend(f.ui.details());
    for hay in &haystacks {
        for needle in &needles {
            assert!(!hay.contains(needle.as_str()), "leak of {needle:?}");
        }
    }
}

fn on_disk(f: &Fx) -> Database {
    let path = f.session.vault_path().unwrap();
    Database::open(
        &std::fs::read(path).unwrap(),
        &SecretString::from("pw".to_owned()),
    )
    .unwrap()
}

#[test]
fn get_rows_fall_back_to_the_passkey_user_name_when_the_entry_has_none() {
    // Task 8 M3: a KeePassXC import often has an empty UserName; the chooser still shows who.
    let (mut db, ids) = sample();
    let now = Timestamp::now();
    db.update_entry(ids.a, now, |a| a.set_field("UserName", "", false))
        .unwrap();
    let (f, _svc) = passkey_fx_with_db(db);
    let r = get(&f, 90, &[]);
    let Response::PasskeyGetPending { matches, .. } = &r else {
        panic!("{r:?}");
    };
    let by_id = |id: Uuid| {
        matches
            .iter()
            .find(|m| m.id == crate::views::wire_id(id))
            .unwrap()
            .username
            .clone()
    };
    // `a` has no UserName: the passkey's own name; `b` keeps its entry UserName.
    assert_eq!(by_id(ids.a), "alice");
    assert_eq!(by_id(ids.b), "someone");
}
