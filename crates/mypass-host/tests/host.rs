// test-only support code
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

// Release hosts intentionally ignore the data-root override: never launch one from a fixture.
#[cfg(all(windows, not(debug_assertions)))]
compile_error!("Windows host fixtures require the debug-only isolated root override");

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mypass_bridge::codec::{encode_frame, read_frame, MAX_FRAME_BYTES};
use mypass_bridge::endpoint::{generate_token, hello_proof, Endpoint, EndpointStore, Handshake};
#[cfg(not(windows))]
use mypass_platform::shared_store;
use mypass_platform::AppPaths;

fn shared_secrets(root: &Path) -> Arc<dyn mypass_platform::SecretStore> {
    #[cfg(windows)]
    return mypass_platform::host_shared_store(&paths(root), None).1;
    #[cfg(not(windows))]
    Arc::new(shared_store(&paths(root)))
}
fn require_native_opt_in() {
    #[cfg(windows)]
    assert_eq!(
        std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
        Ok("1")
    );
}

fn paths(root: &Path) -> AppPaths {
    #[cfg(windows)]
    {
        AppPaths::windows_data_root(root)
    }
    #[cfg(not(windows))]
    {
        AppPaths::macos(root)
    }
}

fn host(root: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mypass-host"));
    c.env("HOME", root)
        .env("MYPASS_WINDOWS_DATA_ROOT", root)
        .env("USERPROFILE", root)
        .env("APPDATA", root.join("AppData"))
        .env("LOCALAPPDATA", root.join("Local"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn frame_of(json: &str) -> Vec<u8> {
    encode_frame(json.as_bytes()).unwrap()
}

/// Runs the host with `input` on stdin; returns (exit success, stdout bytes, stderr bytes).
struct ReapChild(Child);
impl Drop for ReapChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn run(root: &Path, input: &[u8]) -> (bool, Vec<u8>, Vec<u8>) {
    let mut child = ReapChild(host(root).spawn().unwrap());
    let mut stdout = child.0.stdout.take().unwrap();
    let mut stderr = child.0.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut b = Vec::new();
        stdout.read_to_end(&mut b).unwrap();
        b
    });
    let err = thread::spawn(move || {
        let mut b = Vec::new();
        stderr.read_to_end(&mut b).unwrap();
        b
    });
    child.0.stdin.take().unwrap().write_all(input).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "host did not exit within deadline"
        );
        thread::sleep(Duration::from_millis(10));
    };
    (status.success(), out.join().unwrap(), err.join().unwrap())
}
fn accept_before_deadline(listener: &TcpListener) -> std::net::TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((s, _)) => {
                s.set_nonblocking(false).unwrap();
                return s;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("accept: {e}"),
        }
        assert!(
            Instant::now() < deadline,
            "host did not connect within deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// A loopback listener standing in for the app, so a record that points at it would be dialled if
/// the store accepted it. Non-blocking: `assert_no_connection` polls it after the host exited.
fn live_listener() -> TcpListener {
    let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    l.set_nonblocking(true).unwrap();
    l
}

fn assert_no_connection(l: &TcpListener) {
    match l.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok(_) => panic!("the host connected to a record the store must have refused"),
        Err(e) => panic!("accept failed: {e}"),
    }
}

fn assert_not_running(root: &Path) {
    let (ok, stdout, stderr) = run(root, &frame_of(r#"{"type":"getStatus"}"#));
    assert!(ok, "exit code must be 0");
    assert!(stderr.is_empty(), "no stderr output");
    let mut s = &stdout[..];
    let reply = read_frame(&mut s, MAX_FRAME_BYTES).unwrap().unwrap();
    assert_eq!(
        String::from_utf8(reply).unwrap(),
        r#"{"message":"app_not_running","type":"error"}"#
    );
    assert!(s.is_empty(), "exactly one frame");
}

fn save_endpoint(root: &Path, ep: &Endpoint) {
    EndpointStore::new(shared_secrets(root)).save(ep).unwrap();
}

#[test]
fn missing_record_is_app_not_running_with_exit_zero() {
    let tmp = tempfile::tempdir().unwrap();
    assert_not_running(tmp.path());
}

#[test]
#[cfg_attr(
    windows,
    ignore = "protected endpoint; use isolated Windows integration runner"
)]
fn windows_integration_malformed_record_pointing_at_a_live_listener_is_not_dialled() {
    require_native_opt_in();
    let tmp = tempfile::tempdir().unwrap();
    let live = live_listener();
    let port = live.local_addr().unwrap().port();
    let store = shared_secrets(tmp.path());
    // Valid JSON and a live port, but an empty token: the record is invalid.
    let record = format!(r#"{{"port":{port},"token":"","pid":1,"startedAt":1}}"#);
    mypass_platform::SecretStore::set(
        store.as_ref(),
        mypass_bridge::endpoint::ENDPOINT_SERVICE,
        mypass_bridge::endpoint::ENDPOINT_ACCOUNT,
        record.as_bytes(),
    )
    .unwrap();
    assert_not_running(tmp.path());
    assert_no_connection(&live);
}

#[cfg(unix)]
#[test]
fn wrong_mode_record_pointing_at_a_live_listener_is_not_dialled() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let live = live_listener();
    let ep = Endpoint {
        port: live.local_addr().unwrap().port(),
        token: generate_token(),
        pid: std::process::id(),
        started_at: 1,
    };
    save_endpoint(tmp.path(), &ep);
    let dir = paths(tmp.path()).secrets_shared;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    assert_not_running(tmp.path());
    assert_no_connection(&live);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "protected endpoint; use isolated Windows integration runner"
)]
fn windows_integration_host_relays_one_request_end_to_end() {
    require_native_opt_in();
    let tmp = tempfile::tempdir().unwrap();
    let token = generate_token();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let ep = Endpoint {
        port: listener.local_addr().unwrap().port(),
        token: token.clone(),
        pid: std::process::id(),
        started_at: 1,
    };
    save_endpoint(tmp.path(), &ep);
    let app = thread::spawn(move || {
        let mut s = accept_before_deadline(&listener);
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let hello = read_frame(&mut s, MAX_FRAME_BYTES).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&hello).unwrap();
        let proof = hello_proof(&token, v["nonce"].as_str().unwrap());
        let ack = serde_json::to_vec(&Handshake::HelloAck { proof }).unwrap();
        s.write_all(&encode_frame(&ack).unwrap()).unwrap();
        let req = read_frame(&mut s, MAX_FRAME_BYTES).unwrap().unwrap();
        s.write_all(&frame_of(r#"{"type":"ok"}"#)).unwrap();
        let mut rest = Vec::new();
        let _ = s.read_to_end(&mut rest);
        (token, req)
    });
    let (ok, stdout, stderr) = run(
        tmp.path(),
        &frame_of(r#"{"type":"getStatus","token":"evil","note":"\r\n\u001a\u0000"}"#),
    );
    assert!(ok);
    assert!(stderr.is_empty());
    let mut s = &stdout[..];
    let reply = read_frame(&mut s, MAX_FRAME_BYTES).unwrap().unwrap();
    assert_eq!(reply, br#"{"type":"ok"}"#);
    let (token, req) = app.join().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&req).unwrap();
    assert_eq!(v["token"], token.as_str());
    assert_eq!(v["note"], "\r\n\u{001a}\0");
}

fn main_source() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).unwrap()
}

#[test]
fn main_names_no_env_override_and_never_builds_the_platform() {
    let src = main_source();
    assert!(!src.contains("debug_assertions"));
    assert!(!src.contains("MYPASS_"));
    assert!(!src.contains("platform::current"));
    assert!(!src.contains("dev_overrides"));
}

#[test]
fn main_has_no_println_of_request_data() {
    let src = main_source();
    for banned in ["println!", "print!", "dbg!"] {
        assert!(!src.contains(banned), "{banned} in main.rs");
    }
    for line in src.lines().filter(|l| l.contains("eprintln!")) {
        let arg = line.split("eprintln!(").nth(1).unwrap().trim_start();
        assert!(
            arg.starts_with('"') && !arg.contains('{'),
            "non-literal eprintln: {line}"
        );
    }
}

#[cfg(windows)]
#[test]
#[ignore = "protected endpoint; use isolated Windows integration runner"]
fn windows_integration_mismatched_endpoint_pid_receives_no_hello_or_token() {
    require_native_opt_in();
    let tmp = tempfile::tempdir().unwrap();
    let listener = live_listener();
    save_endpoint(
        tmp.path(),
        &Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: generate_token(),
            pid: u32::MAX,
            started_at: 1,
        },
    );
    assert_not_running(tmp.path());
    let mut stream = accept_before_deadline(&listener);
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut seen = Vec::new();
    stream.read_to_end(&mut seen).unwrap();
    assert!(
        seen.is_empty(),
        "refused peer received traffic before identity verification"
    );
}

#[cfg(windows)]
#[test]
fn legacy_plaintext_endpoint_is_never_trusted() {
    let tmp = tempfile::tempdir().unwrap();
    let listener = live_listener();
    // No DPAPI calls: a legacy plaintext file is not even an MPS1 envelope. The production
    // selector must refuse it, regardless of whether the listener could answer the HMAC proof.
    EndpointStore::new(Arc::new(mypass_platform::shared_store(&paths(tmp.path()))))
        .save(&Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: generate_token(),
            pid: std::process::id(),
            started_at: 1,
        })
        .unwrap();
    assert_not_running(tmp.path());
    assert_no_connection(&listener);
}

#[test]
fn host_binary_answers_launch_app_without_an_app() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, stdout, stderr) = run(dir.path(), &frame_of(r#"{"type":"launchApp"}"#));
    assert!(ok);
    assert!(stderr.is_empty());
    let mut output = stdout.as_slice();
    let answer = read_frame(&mut output, MAX_FRAME_BYTES).unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&answer).unwrap(),
        serde_json::json!({"type":"error","message":"launch_unavailable"})
    );
    assert!(output.is_empty());
}
