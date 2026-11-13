//! End-to-end tests: the real `mypass-host` binary against a real in-process `LoopbackServer`, a
//! real `BridgeService` and a real `VaultSession`.
//!
//! Isolation (no product-side override): the spawned host resolves its paths from `HOME` (macOS) or
//! `APPDATA`, `LOCALAPPDATA` and `USERPROFILE` (Windows), all pointed into one temp dir, and the
//! in-process server writes its endpoint through `AppPaths` rooted at the same dir, so both sides
//! meet at the same `secrets_shared` file. Nothing here touches the real Keychain, the real home or
//! any address but 127.0.0.1.
// test-only support code
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, UNIX_EPOCH};

use mypass_bridge::codec::{encode_frame, read_frame, HEADER_LEN, MAX_FRAME_BYTES};
use mypass_bridge::diagnostics::DiagnosticsStore;
use mypass_bridge::endpoint::{generate_token, Endpoint, EndpointStore};
use mypass_bridge::pairing::{PairedBrowserStore, PairingChallenge, PairingManager};
use mypass_bridge::passkeys::NotSupportedPasskeys;
use mypass_bridge::server::{Handler, LoopbackServer, ServerConfig};
use mypass_bridge::service::{BridgeDeps, BridgeService};
use mypass_bridge::ui::{BridgeUi, ProposalStore};
use mypass_core::TotpConfig;
use mypass_kdbx::{Database, Entry, PlainFileIo, Timestamp};
use mypass_platform::testing::InMemorySecretStore;
use mypass_platform::{shared_store, AppPaths};
use mypass_session::autolock::AutoLock;
use mypass_session::scheduler::ManualScheduler;
use mypass_session::session::{LockReason, SessionDeps, VaultSession};
use mypass_session::settings::SettingsStore;
use mypass_session::testing::fast_database;
use secrecy::SecretString;
use serde_json::{json, Value};
use uuid::Uuid;

const WALL_START: u64 = 1_700_000_000;
const OTP: &str = "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8";
const WAIT: Duration = Duration::from_secs(30);

fn paths(root: &Path) -> AppPaths {
    #[cfg(windows)]
    {
        AppPaths::windows(&root.join("AppData"), &root.join("Local"), root)
    }
    #[cfg(not(windows))]
    {
        AppPaths::macos(root)
    }
}

/// Polls `cond` until it holds or the deadline passes (never a fixed sleep as the mechanism).
fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Default)]
struct RecordingUi {
    codes: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
}

impl RecordingUi {
    fn last_code(&self) -> String {
        self.codes
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a pairing code was shown")
    }
    fn note(&self, s: &str) {
        self.calls.lock().unwrap().push(s.to_owned());
    }
}

impl BridgeUi for RecordingUi {
    fn pairing_code(&self, c: &PairingChallenge) {
        self.codes.lock().unwrap().push(c.code.clone());
    }
    fn pairing_finished(&self) {
        self.note("pairing_finished");
    }
    fn save_login_proposal(&self, _id: &str, _url: &str, _username: &str) {
        self.note("save_login_proposal");
    }
    fn save_totp_proposal(&self, _id: &str, _url: &str, _entry_id: Option<Uuid>) {
        self.note("save_totp_proposal");
    }
    fn unlock_requested(&self) {
        self.note("unlock_requested");
    }
    fn open_app_requested(&self) {
        self.note("open_app_requested");
    }
    fn open_browser_settings_requested(&self) {
        self.note("open_browser_settings_requested");
    }
    fn credential_filled(&self, _id: Uuid) {
        self.note("credential_filled");
    }
}

/// The app side: temp dir, vault session, bridge service, loopback server.
struct App {
    dir: tempfile::TempDir,
    root: PathBuf,
    vault: PathBuf,
    session: VaultSession,
    sched: Arc<ManualScheduler>,
    ui: Arc<RecordingUi>,
    server: LoopbackServer,
}

impl App {
    fn start() -> Self {
        Self::start_with(ServerConfig::default())
    }

    fn start_with(cfg: ServerConfig) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        let vault = dir.path().join("v.kdbx");

        let settings = SettingsStore::in_memory();
        let sched = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(WALL_START));
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
        let ui = Arc::new(RecordingUi::default());
        let svc = BridgeService::new(BridgeDeps {
            session: session.clone(),
            autolock,
            settings,
            scheduler: sched.clone(),
            pairing,
            diagnostics: Arc::new(DiagnosticsStore::new()),
            proposals: Arc::new(ProposalStore::new(sched.clone())),
            ui: ui.clone(),
            passkeys: Arc::new(NotSupportedPasskeys),
        });

        let mut db = fast_database("E2E");
        let now = Timestamp::now();
        let mut e = Entry::new(now);
        e.set_field("Title", "GitHub", false);
        e.set_field("URL", "https://github.com", false);
        e.set_field("UserName", "alice", false);
        e.set_field("Password", "hunter2", true);
        e.set_field("otp", OTP, true);
        let root_group = db.root_id();
        db.add_entry(root_group, e, now).unwrap();
        session
            .create_vault_with(&vault, SecretString::from("pw".to_owned()), db)
            .unwrap();

        let endpoints = Arc::new(EndpointStore::new(Arc::new(shared_store(&paths(&root)))));
        let handler: Handler = Arc::new(move |r| svc.handle(r));
        let server = LoopbackServer::start(cfg, endpoints, handler).unwrap();
        Self {
            dir,
            root,
            vault,
            session,
            sched,
            ui,
            server,
        }
    }

    fn token(&self) -> String {
        self.server.endpoint().token.clone()
    }

    fn spawn(&self) -> Host {
        Host::spawn(&self.root)
    }

    /// Pairs a browser through a real host and returns the pairing secret.
    fn pair(&self) -> String {
        let mut h = self.spawn();
        let r = h.call(json!({"type":"startPairing","browser":"Chrome"}));
        assert_eq!(r["type"], "pairingStarted");
        let code = self.ui.last_code();
        let r = h.call(json!({"type":"completePairing","code":code}));
        assert_eq!(r["type"], "paired", "{r}");
        let secret = r["secret"].as_str().unwrap().to_owned();
        h.finish();
        secret
    }
}

struct Exit {
    stdout_frames: Vec<String>,
    stderr: Vec<u8>,
    leftover_stdout: Vec<u8>,
}

/// A spawned `mypass-host` with framed stdin and stdout.
struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    frames: Receiver<Vec<u8>>,
    reader: JoinHandle<Vec<u8>>,
    stderr: JoinHandle<Vec<u8>>,
    seen: Vec<String>,
}

impl Host {
    fn spawn(root: &Path) -> Self {
        let mut c = Command::new(env!("CARGO_BIN_EXE_mypass-host"));
        c.env("HOME", root)
            .env("USERPROFILE", root)
            .env("APPDATA", root.join("AppData"))
            .env("LOCALAPPDATA", root.join("Local"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (tx, frames) = channel();
        let reader = thread::spawn(move || {
            while let Ok(Some(f)) = read_frame(&mut stdout, MAX_FRAME_BYTES) {
                if tx.send(f).is_err() {
                    break;
                }
            }
            // Whatever follows the last whole frame (nothing, for a well-behaved host).
            let mut rest = Vec::new();
            let _ = stdout.read_to_end(&mut rest);
            rest
        });
        let stderr = thread::spawn(move || {
            let mut b = Vec::new();
            let _ = stderr.read_to_end(&mut b);
            b
        });
        Self {
            child,
            stdin,
            frames,
            reader,
            stderr,
            seen: Vec::new(),
        }
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().unwrap();
        // A host that already exited (oversized frame) closes the pipe: not an error here.
        let _ = stdin.write_all(bytes).and_then(|()| stdin.flush());
    }

    fn send(&mut self, v: &Value) {
        self.send_payload(v.to_string().as_bytes());
    }

    fn send_payload(&mut self, payload: &[u8]) {
        self.send_raw(&encode_frame(payload).unwrap());
    }

    fn recv(&mut self) -> Value {
        match self.frames.recv_timeout(WAIT) {
            Ok(f) => {
                let s = String::from_utf8(f).unwrap();
                self.seen.push(s.clone());
                serde_json::from_str(&s).unwrap()
            }
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for a host frame"),
            Err(RecvTimeoutError::Disconnected) => panic!("host closed stdout without a frame"),
        }
    }

    fn call(&mut self, v: Value) -> Value {
        self.send(&v);
        self.recv()
    }

    /// Closes stdin, waits for the host and asserts it exited 0 (Chromium treats anything else as a
    /// crash).
    fn finish(mut self) -> Exit {
        drop(self.stdin.take());
        let deadline = Instant::now() + WAIT;
        let status = loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                break s;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                panic!("host did not exit");
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "host exit status {status:?}");
        let leftover_stdout = self.reader.join().unwrap();
        while let Ok(f) = self.frames.try_recv() {
            self.seen.push(String::from_utf8(f).unwrap());
        }
        Exit {
            stdout_frames: self.seen,
            stderr: self.stderr.join().unwrap(),
            leftover_stdout,
        }
    }
}

fn not_running(v: &Value) {
    assert_eq!(*v, json!({"type":"error","message":"app_not_running"}));
}

fn get_credentials(secret: &str) -> Value {
    json!({"type":"getCredentials","url":"https://github.com/login","pairing":secret})
}

#[test]
fn pairing_then_autofill_end_to_end() {
    let app = App::start();
    let secret = app.pair();

    let mut h = app.spawn();
    let r = h.call(get_credentials(&secret));
    assert_eq!(r["type"], "credentialList", "{r}");
    let list = r["credentials"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["title"], "GitHub");
    assert_eq!(list[0]["username"], "alice");
    let id = list[0]["id"].as_str().unwrap().to_owned();

    let r = h.call(json!({
        "type":"fillCredential","id":id,"url":"https://github.com/login","pairing":secret
    }));
    assert_eq!(r["type"], "credential", "{r}");
    assert_eq!(r["username"], "alice");
    assert_eq!(r["password"], "hunter2");
    let code = r["totp"].as_str().unwrap();
    assert_eq!(code.len(), 8);
    assert_eq!(
        code,
        TotpConfig::parse(OTP).unwrap().code_at(WALL_START).unwrap()
    );
    h.finish();
}

#[test]
fn extension_supplied_token_is_ignored_by_real_host() {
    let app = App::start();
    let secret = app.pair();
    let mut h = app.spawn();
    let mut req = get_credentials(&secret);
    req["token"] = json!("evil");
    let r = h.call(req);
    assert_eq!(r["type"], "credentialList", "{r}");
    h.finish();
}

#[test]
fn locked_vault_gets_locked_then_unlock_then_served() {
    let app = App::start();
    let secret = app.pair();
    app.session.lock(LockReason::Manual);

    let mut h = app.spawn();
    assert_eq!(h.call(get_credentials(&secret))["type"], "locked");
    let r = h.call(json!({"type":"requestUnlock","pairing":secret}));
    assert_eq!(r["type"], "unlockPresented", "{r}");
    assert!(app
        .ui
        .calls
        .lock()
        .unwrap()
        .contains(&"unlock_requested".to_owned()));

    app.session
        .unlock(SecretString::from("pw".to_owned()))
        .unwrap();
    let r = h.call(get_credentials(&secret));
    assert_eq!(r["type"], "credentialList", "{r}");
    h.finish();
}

#[test]
fn host_output_and_stderr_never_contain_the_token() {
    let app = App::start();
    let token = app.token();
    let secret = app.pair();
    let mut h = app.spawn();
    for i in 0..10 {
        let mut req = match i % 3 {
            0 => get_credentials(&secret),
            1 => json!({"type":"getGeneratedPasswordHistory","pairing":secret}),
            _ => json!({"type":"getPasskeyProxyStatus","pairing":secret}),
        };
        if i == 4 {
            req["token"] = json!("evil");
        }
        let r = h.call(req);
        assert_ne!(r["type"], "error", "request {i}: {r}");
    }
    // A request the app cannot decode still answers with an error frame that never echoes the token.
    h.call(json!({"type":"noSuchRequest"}));
    let exit = h.finish();
    assert!(!exit.stdout_frames.is_empty());
    for f in &exit.stdout_frames {
        assert!(!f.contains(&token), "token leaked on stdout");
    }
    assert!(exit.leftover_stdout.is_empty());
    assert!(
        !String::from_utf8_lossy(&exit.stderr).contains(&token),
        "token leaked on stderr"
    );
    assert!(exit.stderr.is_empty(), "stderr must be silent");
}

fn save_endpoint(root: &Path, ep: &Endpoint) {
    EndpointStore::new(Arc::new(shared_store(&paths(root))))
        .save(ep)
        .unwrap();
}

fn temp_root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    (dir, root)
}

#[test]
fn stale_endpoint_with_closed_port_answers_app_not_running() {
    let (_dir, root) = temp_root();
    let port = {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    save_endpoint(
        &root,
        &Endpoint {
            port,
            token: generate_token(),
            pid: 1,
            started_at: 1,
        },
    );
    let mut h = Host::spawn(&root);
    not_running(&h.call(json!({"type":"getCredentials","url":"https://github.com"})));
    h.finish();
}

#[test]
fn foreign_listener_on_stale_port_never_receives_token_or_request() {
    let (_dir, root) = temp_root();
    let token = generate_token();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    save_endpoint(
        &root,
        &Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: token.clone(),
            pid: 1,
            started_at: 1,
        },
    );
    // The squatter reads the host's hello, answers with a bogus proof, then records everything
    // until the host hangs up.
    let squatter = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(WAIT)).unwrap();
        let hello = read_frame(&mut s, MAX_FRAME_BYTES).unwrap().unwrap();
        let bogus = json!({"type":"helloAck","proof":"AAAA"}).to_string();
        let _ = s.write_all(&encode_frame(bogus.as_bytes()).unwrap());
        let mut rest = Vec::new();
        let _ = s.read_to_end(&mut rest);
        (hello, rest)
    });
    let mut h = Host::spawn(&root);
    not_running(&h.call(json!({"type":"getCredentials","url":"https://github.com"})));
    h.finish();
    let (hello, rest) = squatter.join().unwrap();
    let v: Value = serde_json::from_slice(&hello).unwrap();
    assert_eq!(v["type"], "hello");
    assert!(
        rest.is_empty(),
        "nothing may follow the hello: {} bytes",
        rest.len()
    );
    let all = String::from_utf8_lossy(&hello).into_owned();
    assert!(!all.contains(&token) && !all.contains("getCredentials"));
}

#[test]
fn no_endpoint_answers_app_not_running() {
    let (_dir, root) = temp_root();
    let mut h = Host::spawn(&root);
    not_running(&h.call(json!({"type":"getCredentials","url":"https://github.com"})));
    h.finish();
}

#[test]
fn malformed_extension_frame_answers_malformed_request() {
    let app = App::start();
    let secret = app.pair();
    let mut h = app.spawn();
    let expected = json!({"type":"error","message":"malformed_request"});
    h.send_payload(b"this is not json");
    assert_eq!(h.recv(), expected);
    h.send_payload(b"[1,2,3]");
    assert_eq!(h.recv(), expected);
    // The relay keeps serving after a bad frame.
    let r = h.call(get_credentials(&secret));
    assert_eq!(r["type"], "credentialList", "{r}");
    h.finish();
}

#[test]
fn oversized_extension_frame_terminates_host_cleanly() {
    let app = App::start();
    let mut h = app.spawn();
    // Only a header that claims one byte more than the cap: the host must stop without reading it.
    let claimed = u32::try_from(MAX_FRAME_BYTES + 1).unwrap();
    let mut header = claimed.to_le_bytes().to_vec();
    assert_eq!(header.len(), HEADER_LEN);
    header.extend_from_slice(b"xxxx");
    h.send_raw(&header);
    let exit = h.finish(); // asserts exit code 0
    assert!(exit.stderr.is_empty());
    assert!(exit.leftover_stdout.is_empty());
}

#[test]
fn host_exits_zero_in_every_scenario() {
    // No endpoint, stale endpoint, malformed frame, oversized frame, a served request and stdin
    // closed at once: every one of them is a clean exit 0 with a silent stderr.
    let (_d1, no_endpoint) = temp_root();
    let (_d2, stale) = temp_root();
    let port = {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    save_endpoint(
        &stale,
        &Endpoint {
            port,
            token: generate_token(),
            pid: 1,
            started_at: 1,
        },
    );
    let app = App::start();
    let secret = app.pair();

    let mut exits = Vec::new();
    for root in [&no_endpoint, &stale, &app.root] {
        let h = Host::spawn(root);
        exits.push(h.finish()); // stdin closed with nothing sent
    }
    let mut h = Host::spawn(&no_endpoint);
    h.send_payload(b"junk");
    exits.push(h.finish());
    let mut h = app.spawn();
    h.send_payload(b"junk");
    h.recv();
    exits.push(h.finish());
    let mut h = app.spawn();
    h.send_raw(&u32::MAX.to_le_bytes());
    exits.push(h.finish());
    let mut h = app.spawn();
    h.call(get_credentials(&secret));
    exits.push(h.finish());
    for e in exits {
        assert!(e.stderr.is_empty());
    }
}

#[test]
fn sixteen_hosts_hold_the_cap_and_the_seventeenth_is_refused_until_a_slot_frees() {
    // The held hosts connect eagerly and send nothing, so each keeps its slot until its stdin
    // closes; the generous first-frame deadline keeps the server from reaping them meanwhile.
    let app = App::start_with(ServerConfig {
        first_frame_deadline: Duration::from_secs(120),
        ..ServerConfig::default()
    });
    let secret = app.pair();
    wait_until("the pairing connection to close", || {
        app.server.active_connections() == 0
    });

    let mut held: Vec<Host> = (0..16).map(|_| app.spawn()).collect();
    wait_until("16 held connections", || {
        app.server.active_connections() == 16
    });

    // The server closes the 17th connection unread: the host's handshake fails.
    let mut seventeenth = app.spawn();
    not_running(&seventeenth.call(get_credentials(&secret)));
    seventeenth.finish();
    assert_eq!(app.server.active_connections(), 16);

    // One of the 16 leaves; its slot frees and a new host is served.
    held.pop().unwrap().finish();
    wait_until("a freed slot", || app.server.active_connections() == 15);
    let mut next = app.spawn();
    let r = next.call(get_credentials(&secret));
    assert_eq!(r["type"], "credentialList", "{r}");
    next.finish();

    for h in held {
        h.finish();
    }
    wait_until("all connections closed", || {
        app.server.active_connections() == 0
    });
}

#[test]
fn passkey_status_is_disabled_placeholder_end_to_end() {
    let app = App::start();
    let secret = app.pair();
    let mut h = app.spawn();
    let r = h.call(json!({"type":"getPasskeyProxyStatus","pairing":secret}));
    assert_eq!(
        r,
        json!({
            "type":"passkeyProxyStatus","unlocked":true,"paired":true,
            "attachAllowed":false,"enabled":false
        })
    );
    h.finish();
}

fn vault_has_title(path: &Path, title: &str) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let Ok(db) = Database::open(&bytes, &SecretString::from("pw".to_owned())) else {
        return false;
    };
    db.entries_excluding(&[]).iter().any(|e| e.title() == title)
}

#[test]
fn create_entry_persists_through_save_pipeline() {
    let app = App::start();
    let secret = app.pair();
    let mut h = app.spawn();
    let r = h.call(json!({
        "type":"createEntry","entryType":"Login","title":"Created By Extension",
        "username":"bob","password":"s3cret","url":"https://example.org","pairing":secret
    }));
    assert_eq!(r["type"], "entryCreated", "{r}");
    h.finish();
    assert!(
        !vault_has_title(&app.vault, "Created By Extension"),
        "not saved before the debounce"
    );
    // The autosave is debounced by 1 s of the manual clock.
    app.sched.advance(Duration::from_secs(1));
    wait_until("the autosave", || {
        vault_has_title(&app.vault, "Created By Extension")
    });
    let _ = &app.dir;
}

#[test]
fn app_quit_gate_rejects_requests_end_to_end() {
    let app = App::start();
    let secret = app.pair();
    let mut h = app.spawn();
    app.session.begin_quit();
    let r = h.call(get_credentials(&secret));
    assert_eq!(r, json!({"type":"error","message":"MyPass is quitting"}));
    app.session.cancel_quit();
    let r = h.call(get_credentials(&secret));
    assert_eq!(r["type"], "credentialList", "{r}");
    h.finish();
}
