//! Passkeys end to end over TCP: a real `LoopbackServer`, a real `BridgeService` with the real
//! `PasskeyService`, a real `VaultSession` over a temp vault file, and a raw TCP client that does
//! what `mypass-host` does (hello, verify the proof, one token-bearing frame per connection),
//! carrying the exact JSON `extension/background.js` and `extension/popup.js` send.
//!
//! Every `responseJson` is verified here independently of the encoder: CBOR through `ciborium`,
//! signatures through `p256`, hashes through `sha2`. The vault file is reopened with
//! `Database::open` to find the passkey. Nothing touches the real Keychain (in-memory secret
//! stores), the real home or any address but 127.0.0.1; no test sleeps for a fixed time.
#![cfg(feature = "server")]
// test-only support code
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use mypass_bridge::codec::{encode_frame, read_frame, MAX_FRAME_BYTES};
use mypass_bridge::diagnostics::DiagnosticsStore;
use mypass_bridge::endpoint::{generate_token, new_nonce, verify_proof, EndpointStore, Handshake};
use mypass_bridge::pairing::{PairedBrowserStore, PairingChallenge, PairingManager, StartOutcome};
use mypass_bridge::passkeys::{PasskeyContext, PasskeyHandler, PasskeyService};
use mypass_bridge::protocol::{Request, Response};
use mypass_bridge::server::{Handler, LoopbackServer, ServerConfig};
use mypass_bridge::service::{BridgeDeps, BridgeService};
use mypass_bridge::ui::{BridgeUi, ProposalStore};
use mypass_core::passkey::PasskeyRecord;
use mypass_kdbx::{Database, Entry, PlainFileIo, Timestamp};
use mypass_platform::testing::InMemorySecretStore;
use mypass_session::autolock::AutoLock;
use mypass_session::scheduler::ManualScheduler;
use mypass_session::session::{LockReason, SessionDeps, VaultSession};
use mypass_session::settings::SettingsStore;
use mypass_session::testing::fast_database;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{DerSignature, VerifyingKey};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey};
use secrecy::SecretString;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const GUARD: Duration = Duration::from_secs(10);
const ORIGIN: &str = "https://example.com";
const RP: &str = "example.com";
const CREATE_CHALLENGE: &[u8] = b"0123456789abcdef-create-challenge";
const GET_CHALLENGE: &[u8] = b"0123456789abcdef-get-challenge";
const USER_HANDLE: &[u8] = b"user-handle-1";
/// The MyPass AAGUID (R12).
const AAGUID: [u8; 16] = [
    0x94, 0x80, 0x34, 0x71, 0xf8, 0xb4, 0x48, 0xf9, 0x98, 0xff, 0xfc, 0x4a, 0xc1, 0xa9, 0x2a, 0xf8,
];

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(v: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(v.as_str().unwrap_or_else(|| panic!("not a string: {v}")))
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// The app side

/// Every UI call is ignored: a passkey ceremony makes none.
struct QuietUi;

impl BridgeUi for QuietUi {
    fn pairing_code(&self, _c: &PairingChallenge) {}
    fn pairing_finished(&self) {}
    fn save_login_proposal(&self, _id: &str, _url: &str, _username: &str) {}
    fn save_totp_proposal(&self, _id: &str, _url: &str, _entry_id: Option<Uuid>) {}
    fn unlock_requested(&self) {}
    fn open_app_requested(&self) {}
    fn open_browser_settings_requested(&self) {}
    fn credential_filled(&self, _id: Uuid) {}
}

/// The real service, counting what reaches it.
struct Counting {
    inner: Arc<PasskeyService>,
    handled: AtomicUsize,
}

impl PasskeyHandler for Counting {
    fn handle(&self, ctx: &PasskeyContext<'_>, request: &Request) -> Response {
        self.handled.fetch_add(1, Ordering::SeqCst);
        self.inner.handle(ctx, request)
    }
    fn on_session_locked(&self) {
        self.inner.on_session_locked();
    }
}

struct App {
    _dir: tempfile::TempDir,
    vault: PathBuf,
    session: VaultSession,
    passkeys: Arc<Counting>,
    server: LoopbackServer,
    /// The pairing secret of the one paired browser.
    secret: String,
}

impl App {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("v.kdbx");
        let settings = SettingsStore::in_memory();
        let sched = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let session = VaultSession::new(SessionDeps {
            settings: settings.clone(),
            scheduler: sched.clone(),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        let autolock = AutoLock::new(session.clone(), settings.clone(), sched.clone());
        let pairing = Arc::new(
            PairingManager::new(
                PairedBrowserStore::new(InMemorySecretStore::new()),
                sched.clone(),
            )
            .unwrap(),
        );
        let secret = match pairing.start(Some("Chrome")) {
            StartOutcome::Started(c) => pairing.complete(&c.code).unwrap().unwrap(),
            StartOutcome::Throttled => panic!("throttled"),
        };
        let ui: Arc<dyn BridgeUi> = Arc::new(QuietUi);
        let passkeys = Arc::new(Counting {
            inner: PasskeyService::new(sched.clone(), Arc::clone(&ui)),
            handled: AtomicUsize::new(0),
        });
        let svc = BridgeService::new(BridgeDeps {
            session: session.clone(),
            autolock,
            settings,
            scheduler: sched.clone(),
            pairing,
            diagnostics: Arc::new(DiagnosticsStore::new()),
            proposals: Arc::new(ProposalStore::new(sched)),
            ui,
            passkeys: passkeys.clone(),
        });

        let mut db = fast_database("Passkey E2E");
        let now = Timestamp::now();
        let mut e = Entry::new(now);
        e.set_field("Title", "Unrelated", false);
        e.set_field("URL", "https://other.org", false);
        let root = db.root_id();
        db.add_entry(root, e, now).unwrap();
        session
            .create_vault_with(&vault, SecretString::from("pw".to_owned()), db)
            .unwrap();

        let handler: Handler = Arc::new(move |r| svc.handle(r));
        let server = LoopbackServer::start(
            ServerConfig::default(),
            Arc::new(EndpointStore::new(InMemorySecretStore::new())),
            handler,
        )
        .unwrap();
        Self {
            _dir: dir,
            vault,
            session,
            passkeys,
            server,
            secret,
        }
    }

    fn handled(&self) -> usize {
        self.passkeys.handled.load(Ordering::SeqCst)
    }

    /// A connected, hello-verified stream: what the host holds before it sends the token.
    fn connect(&self) -> TcpStream {
        let mut c = TcpStream::connect(self.server.local_addr()).unwrap();
        c.set_read_timeout(Some(GUARD)).unwrap();
        let nonce = new_nonce();
        send(&mut c, &json!({"type": "hello", "nonce": nonce}));
        let ack = recv(&mut c).expect("helloAck");
        let Handshake::HelloAck { proof } = serde_json::from_value(ack).unwrap() else {
            panic!("not an ack");
        };
        assert!(verify_proof(&self.server.endpoint().token, &nonce, &proof));
        c
    }

    /// One extension message, as the host relays it: a fresh connection, the endpoint token
    /// added, one reply.
    fn ask_with_token(&self, message: &Value, token: &str) -> Value {
        let mut c = self.connect();
        let mut framed = message.clone();
        framed["token"] = json!(token);
        send(&mut c, &framed);
        recv(&mut c).expect("a reply")
    }

    fn ask(&self, message: &Value) -> Value {
        self.ask_with_token(message, &self.server.endpoint().token)
    }

    /// `sendNativeMessageWithPairing`: the message with the stored pairing secret attached.
    fn ext(&self, message: Value) -> Value {
        let mut m = message;
        m["pairing"] = json!(self.secret);
        self.ask(&m)
    }

    fn lock(&self) {
        self.session.lock(LockReason::Manual);
        assert!(!self.session.is_unlocked());
    }

    fn unlock(&self) {
        self.session
            .unlock(SecretString::from("pw".to_owned()))
            .unwrap();
    }

    /// The vault as it is on disk now.
    fn reopen(&self) -> Database {
        let bytes = std::fs::read(&self.vault).unwrap();
        Database::open(&bytes, &SecretString::from("pw".to_owned())).unwrap()
    }
}

fn send(c: &mut TcpStream, v: &Value) {
    c.write_all(&encode_frame(v.to_string().as_bytes()).unwrap())
        .unwrap();
}

fn recv(c: &mut TcpStream) -> Option<Value> {
    read_frame(c, MAX_FRAME_BYTES)
        .ok()
        .flatten()
        .map(|b| serde_json::from_slice(&b).unwrap())
}

/// True only for a real close (end of stream, reset or abort); a read timeout is a failure.
fn closed(c: &mut TcpStream) -> bool {
    let mut b = [0u8; 1];
    match c.read(&mut b) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// The extension's messages (background.js `handlePasskeyRequest`, popup.js approvals)

/// Chrome's `requestDetailsJson` for a create: the true origin in the proxy override.
fn create_details() -> String {
    json!({
        "rp": {"id": RP, "name": "Example"},
        "user": {"id": b64(USER_HANDLE), "name": "alice", "displayName": "Alice"},
        "challenge": b64(CREATE_CHALLENGE),
        "pubKeyCredParams": [{"type": "public-key", "alg": -7}, {"type": "public-key", "alg": -257}],
        "timeout": 60000,
        "excludeCredentials": [],
        "authenticatorSelection": {"residentKey": "required", "userVerification": "preferred"},
        "attestation": "direct",
        "extensions": {
            "credProps": true,
            "remoteDesktopClientOverride": {"origin": ORIGIN, "sameOriginWithAncestors": true}
        }
    })
    .to_string()
}

/// Chrome's `requestDetailsJson` for a get naming `allow`.
fn get_details(allow: &[&[u8]]) -> String {
    json!({
        "challenge": b64(GET_CHALLENGE),
        "timeout": 60000,
        "rpId": RP,
        "allowCredentials": allow.iter()
            .map(|id| json!({"type": "public-key", "id": b64(id)}))
            .collect::<Vec<_>>(),
        "userVerification": "preferred",
        "extensions": {
            "remoteDesktopClientOverride": {"origin": ORIGIN, "sameOriginWithAncestors": true}
        }
    })
    .to_string()
}

fn passkey_create(request_id: i64) -> Value {
    json!({"type": "passkeyCreate", "requestId": request_id,
           "requestDetailsJson": create_details(), "originUrl": ORIGIN})
}

/// popup.js `sendPasskeyCreateConfirm` (a new entry: `entryId` null).
fn passkey_create_confirm(request_id: i64, entry_id: Option<&str>) -> Value {
    json!({"type": "passkeyCreateConfirm", "requestId": request_id,
           "entryId": entry_id, "replace": false})
}

fn passkey_get(request_id: i64, allow: &[&[u8]]) -> Value {
    json!({"type": "passkeyGet", "requestId": request_id,
           "requestDetailsJson": get_details(allow), "originUrl": ORIGIN})
}

fn passkey_get_approve(request_id: i64, entry_id: &str) -> Value {
    json!({"type": "passkeyGetApprove", "requestId": request_id, "entryId": entry_id})
}

/// All seven passkey messages, each well formed.
fn all_seven() -> Vec<Value> {
    vec![
        json!({"type": "getPasskeyProxyStatus"}),
        json!({"type": "setPasskeyProxyEnabled", "enabled": false}),
        passkey_create(1),
        passkey_create_confirm(1, None),
        passkey_get(2, &[]),
        passkey_get_approve(2, "00000000-0000-0000-0000-000000000000"),
        json!({"type": "passkeyCancel", "requestId": 1}),
    ]
}

fn keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

fn failure(r: &Value, complete: &str, request_id: i64) -> (String, String) {
    assert_eq!(r["type"], complete, "{r}");
    assert_eq!(r["requestId"], request_id, "{r}");
    assert!(r.get("responseJson").is_none(), "{r}");
    (
        r["error"]["name"].as_str().unwrap().to_owned(),
        r["error"]["message"].as_str().unwrap().to_owned(),
    )
}

/// The `responseJson` of a successful completion, parsed.
fn success(r: &Value, complete: &str, request_id: i64) -> Value {
    assert_eq!(r["type"], complete, "{r}");
    assert_eq!(r["requestId"], request_id, "{r}");
    assert_eq!(keys(r), ["requestId", "responseJson", "type"], "{r}");
    serde_json::from_str(r["responseJson"].as_str().unwrap()).unwrap()
}

// ---------------------------------------------------------------------------------------------
// Independent verification

/// The CBOR map value under the text key `k`.
fn cbor_get<'a>(map: &'a ciborium::value::Value, k: &str) -> &'a ciborium::value::Value {
    map.as_map()
        .unwrap()
        .iter()
        .find(|(key, _)| key.as_text() == Some(k))
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no {k}"))
}

/// The CBOR map value under the integer key `k`.
fn cose_get(map: &ciborium::value::Value, k: i64) -> &ciborium::value::Value {
    map.as_map()
        .unwrap()
        .iter()
        .find(|(key, _)| key.as_integer().map(i128::from) == Some(i128::from(k)))
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no COSE key {k}"))
}

fn client_data(resp: &Value, ty: &str, challenge: &[u8]) -> Vec<u8> {
    let raw = unb64(&resp["response"]["clientDataJSON"]);
    let cd: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(cd["type"], ty);
    assert_eq!(cd["origin"], ORIGIN);
    assert_eq!(unb64(&cd["challenge"]), challenge);
    raw
}

/// Verifies a registration response and returns `(credential id, verifying key)`.
fn verify_registration(resp: &Value) -> (Vec<u8>, VerifyingKey) {
    assert_eq!(resp["type"], "public-key");
    assert_eq!(resp["id"], resp["rawId"]);
    let credential = unb64(&resp["rawId"]);
    assert!(!credential.is_empty());
    let client_data = client_data(resp, "webauthn.create", CREATE_CHALLENGE);
    let r = &resp["response"];

    // The SPKI public key and the COSE key inside authData are the same point.
    let spki = unb64(&r["publicKey"]);
    let key = VerifyingKey::from_public_key_der(&spki).unwrap();
    assert_eq!(r["publicKeyAlgorithm"], -7);

    let object: ciborium::value::Value =
        ciborium::from_reader(unb64(&r["attestationObject"]).as_slice()).unwrap();
    let auth = cbor_get(&object, "authData").as_bytes().unwrap().clone();
    assert_eq!(auth, unb64(&r["authenticatorData"]));
    assert_eq!(&auth[..32], Sha256::digest(RP.as_bytes()).as_slice());
    assert_eq!(auth[32], 0x5D, "UP|UV|BE|BS|AT");
    assert_eq!(&auth[33..37], [0, 0, 0, 0], "the counter is always 0");
    assert_eq!(&auth[37..53], AAGUID);
    let len = usize::from(u16::from_be_bytes([auth[53], auth[54]]));
    assert_eq!(&auth[55..55 + len], credential.as_slice());
    let cose: ciborium::value::Value = ciborium::from_reader(&auth[55 + len..]).unwrap();
    // Uncompressed SEC1: 0x04 || x || y.
    let point = key.to_sec1_point(false);
    let point = point.as_bytes();
    assert_eq!((point.len(), point[0]), (65, 0x04));
    assert_eq!(
        cose_get(&cose, 1).as_integer().map(i128::from),
        Some(2),
        "EC2"
    );
    assert_eq!(
        cose_get(&cose, 3).as_integer().map(i128::from),
        Some(-7),
        "ES256"
    );
    assert_eq!(
        cose_get(&cose, -2).as_bytes().unwrap().as_slice(),
        &point[1..33]
    );
    assert_eq!(
        cose_get(&cose, -3).as_bytes().unwrap().as_slice(),
        &point[33..65]
    );

    // Packed self-attestation (the request asked for "direct"): signed by the credential key.
    assert_eq!(cbor_get(&object, "fmt").as_text(), Some("packed"));
    let stmt = cbor_get(&object, "attStmt");
    assert_eq!(cbor_get(stmt, "alg").as_integer().map(i128::from), Some(-7));
    let sig = DerSignature::try_from(cbor_get(stmt, "sig").as_bytes().unwrap().as_slice()).unwrap();
    let mut signed = auth.clone();
    signed.extend_from_slice(&Sha256::digest(&client_data));
    key.verify(&signed, &sig).expect("attestation signature");

    // credProps answered truthfully: the credential is discoverable.
    assert_eq!(resp["clientExtensionResults"]["credProps"]["rk"], true);
    (credential, key)
}

/// Verifies an assertion against `key` for `credential`.
fn verify_assertion(resp: &Value, credential: &[u8], key: &VerifyingKey) {
    assert_eq!(resp["type"], "public-key");
    assert_eq!(unb64(&resp["rawId"]), credential);
    assert_eq!(resp["id"], resp["rawId"]);
    let client_data = client_data(resp, "webauthn.get", GET_CHALLENGE);
    let r = &resp["response"];
    let auth = unb64(&r["authenticatorData"]);
    assert_eq!(
        auth.len(),
        37,
        "no attested credential data on an assertion"
    );
    assert_eq!(&auth[..32], Sha256::digest(RP.as_bytes()).as_slice());
    assert_eq!(auth[32], 0x1D, "UP|UV|BE|BS");
    assert_eq!(&auth[33..37], [0, 0, 0, 0]);
    assert_eq!(unb64(&r["userHandle"]), USER_HANDLE);
    let sig = DerSignature::try_from(unb64(&r["signature"]).as_slice()).unwrap();
    let mut signed = auth;
    signed.extend_from_slice(&Sha256::digest(&client_data));
    key.verify(&signed, &sig).expect("assertion signature");
}

/// The entry on disk holding `credential`.
fn stored(db: &Database, credential: &[u8]) -> Option<Entry> {
    db.entries_excluding(&[])
        .into_iter()
        .find(|e| PasskeyRecord::credential_id(e).as_deref() == Some(credential))
        .cloned()
}

/// Runs a create ceremony to the end and returns `(credential, key)`.
fn register(app: &App, request_id: i64) -> (Vec<u8>, VerifyingKey) {
    let r = app.ext(passkey_create(request_id));
    assert_eq!(r["type"], "passkeyCreatePending", "{r}");
    let r = app.ext(passkey_create_confirm(request_id, None));
    verify_registration(&success(&r, "passkeyCreateComplete", request_id))
}

// ---------------------------------------------------------------------------------------------
// Tests

#[test]
fn full_create_then_get_ceremony_over_tcp() {
    let app = App::start();
    let status = app.ext(json!({"type": "getPasskeyProxyStatus"}));
    assert_eq!(
        status,
        json!({"type": "passkeyProxyStatus", "unlocked": true, "paired": true,
               "attachAllowed": true, "enabled": true})
    );

    // Create, phase one: no candidate on this site, nothing written.
    let r = app.ext(passkey_create(41));
    assert_eq!(
        r,
        json!({"type": "passkeyCreatePending", "requestId": 41, "rpId": RP,
               "rpName": "Example", "matches": []})
    );
    assert!(app
        .reopen()
        .entries_excluding(&[])
        .iter()
        .all(|e| !PasskeyRecord::has_passkey(e)));

    // Create, phase two: a new entry.
    let r = app.ext(passkey_create_confirm(41, None));
    let resp = success(&r, "passkeyCreateComplete", 41);
    let (credential, key) = verify_registration(&resp);

    // Durable before the answer: the file on disk holds it, with the same key.
    let db = app.reopen();
    let entry = stored(&db, &credential).expect("the passkey is in the vault file");
    assert_eq!(entry.title(), "Example");
    assert_eq!(entry.field("UserName"), Some("alice"));
    assert_eq!(entry.field("URL"), Some(ORIGIN));
    let record = PasskeyRecord::read(&entry).unwrap();
    assert_eq!(record.rp_id, RP);
    assert_eq!(record.user_handle.as_deref(), Some(USER_HANDLE));
    let pem_key = p256::SecretKey::from_pkcs8_pem(&record.private_key_pem)
        .unwrap()
        .public_key();
    assert_eq!(VerifyingKey::from(&pem_key), key);
    let entry_id = entry.uuid.as_hyphenated().to_string().to_uppercase();

    // Get, phase one, naming the credential: the one entry is offered, no key material.
    let r = app.ext(passkey_get(42, &[&credential]));
    assert_eq!(r["type"], "passkeyGetPending", "{r}");
    assert_eq!(keys(&r), ["matches", "requestId", "rpId", "type"]);
    assert_eq!(
        (r["requestId"].as_i64(), r["rpId"].as_str()),
        (Some(42), Some(RP))
    );
    let matches = r["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["id"], entry_id.as_str());
    assert_eq!(matches[0]["hasPasskey"], true);
    assert!(!r.to_string().contains("PRIVATE"));

    // Get, phase two: an assertion that verifies with the registered key.
    let r = app.ext(passkey_get_approve(42, &entry_id));
    verify_assertion(&success(&r, "passkeyGetComplete", 42), &credential, &key);

    // A discoverable sign-in (empty allow list) finds it by the rpId.
    let r = app.ext(passkey_get(43, &[]));
    assert_eq!(r["matches"][0]["id"], entry_id.as_str(), "{r}");
    let r = app.ext(passkey_get_approve(43, &entry_id));
    verify_assertion(&success(&r, "passkeyGetComplete", 43), &credential, &key);

    // Single use: the ceremony is gone.
    let r = app.ext(passkey_get_approve(43, &entry_id));
    assert_eq!(
        failure(&r, "passkeyGetComplete", 43),
        ("NotAllowedError".into(), "No pending request".into())
    );
    // A cancel answers with its id.
    assert_eq!(
        app.ext(json!({"type": "passkeyCancel", "requestId": 44})),
        json!({"type": "passkeyCancelled", "requestId": 44})
    );
}

#[test]
fn ceremony_survives_nothing_across_lock_over_tcp() {
    let app = App::start();

    // Create: pending, lock, the confirm answers locked; after the unlock it is gone.
    assert_eq!(app.ext(passkey_create(51))["type"], "passkeyCreatePending");
    app.lock();
    assert_eq!(
        app.ext(passkey_create_confirm(51, None)),
        json!({"type": "locked"})
    );
    app.unlock();
    let r = app.ext(passkey_create_confirm(51, None));
    assert_eq!(
        failure(&r, "passkeyCreateComplete", 51),
        ("NotAllowedError".into(), "No pending request".into())
    );
    assert!(app
        .reopen()
        .entries_excluding(&[])
        .iter()
        .all(|e| !PasskeyRecord::has_passkey(e)));

    // Get: the same, over a passkey that exists.
    let (credential, _key) = register(&app, 52);
    let entry_id = stored(&app.reopen(), &credential)
        .unwrap()
        .uuid
        .as_hyphenated()
        .to_string()
        .to_uppercase();
    assert_eq!(
        app.ext(passkey_get(53, &[&credential]))["type"],
        "passkeyGetPending"
    );
    app.lock();
    assert_eq!(
        app.ext(passkey_get_approve(53, &entry_id)),
        json!({"type": "locked"})
    );
    app.unlock();
    let r = app.ext(passkey_get_approve(53, &entry_id));
    assert_eq!(
        failure(&r, "passkeyGetComplete", 53),
        ("NotAllowedError".into(), "No pending request".into())
    );
}

#[test]
fn locked_vault_answers_status_but_locks_ceremonies() {
    let app = App::start();
    app.lock();
    assert_eq!(
        app.ext(json!({"type": "getPasskeyProxyStatus"})),
        json!({"type": "passkeyProxyStatus", "unlocked": false, "paired": true,
               "attachAllowed": false, "enabled": true})
    );
    assert_eq!(app.handled(), 1);
    for message in all_seven().into_iter().skip(1) {
        let name = message["type"].clone();
        assert_eq!(app.ext(message), json!({"type": "locked"}), "{name}");
    }
    // Only the status reached the passkey service.
    assert_eq!(app.handled(), 1);
}

#[test]
fn unpaired_browser_gets_pairing_required_for_all_seven() {
    let app = App::start();
    for pairing in [None, Some("not-the-secret")] {
        for message in all_seven() {
            let name = message["type"].clone();
            let mut m = message;
            if let Some(p) = pairing {
                m["pairing"] = json!(p);
            }
            assert_eq!(app.ask(&m), json!({"type": "pairingRequired"}), "{name}");
        }
    }
    assert_eq!(app.handled(), 0);
}

#[test]
fn invalid_token_never_reaches_passkey_service() {
    let app = App::start();
    for message in all_seven() {
        let mut c = app.connect();
        let mut m = message;
        m["pairing"] = json!(app.secret);
        m["token"] = json!(generate_token());
        send(&mut c, &m);
        assert_eq!(
            recv(&mut c),
            Some(json!({"type": "error", "message": "invalid token"}))
        );
        assert!(closed(&mut c));
    }
    // A token-less frame is not a request at all: refused or dropped, never served.
    let mut c = app.connect();
    let mut m = passkey_create(9);
    m["pairing"] = json!(app.secret);
    send(&mut c, &m);
    if let Some(r) = recv(&mut c) {
        assert_eq!(r["type"], "error", "{r}");
    }
    assert_eq!(app.handled(), 0);
    // Sanity: the same message with the right token does reach it.
    assert_eq!(app.ext(passkey_create(9))["type"], "passkeyCreatePending");
    assert_eq!(app.handled(), 1);
    let _unused = app.ask_with_token(&json!({"type": "passkeyCancel", "requestId": 9}), "x");
    assert_eq!(app.handled(), 1);
}
