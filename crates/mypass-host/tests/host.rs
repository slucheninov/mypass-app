// test-only support code
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mypass_bridge::codec::{encode_frame, read_frame, MAX_FRAME_BYTES};
use mypass_bridge::endpoint::{generate_token, hello_proof, Endpoint, EndpointStore, Handshake};
use mypass_platform::{shared_store, AppPaths};

fn paths(root: &Path) -> AppPaths {
    #[cfg(windows)]
    {
        let (a, l) = (root.join("AppData"), root.join("Local"));
        AppPaths::windows(&a, &l, root)
    }
    #[cfg(not(windows))]
    {
        AppPaths::macos(root)
    }
}

fn host(root: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mypass-host"));
    c.env("HOME", root)
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
fn run(root: &Path, input: &[u8]) -> (bool, Vec<u8>, Vec<u8>) {
    let mut child = host(root).spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.success(), out.stdout, out.stderr)
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
    EndpointStore::new(Arc::new(shared_store(&paths(root))))
        .save(ep)
        .unwrap();
}

#[test]
fn missing_record_is_app_not_running_with_exit_zero() {
    let tmp = tempfile::tempdir().unwrap();
    assert_not_running(tmp.path());
}

#[test]
fn malformed_record_pointing_at_a_live_listener_is_not_dialled() {
    let tmp = tempfile::tempdir().unwrap();
    let live = live_listener();
    let port = live.local_addr().unwrap().port();
    let store = shared_store(&paths(tmp.path()));
    // Valid JSON and a live port, but an empty token: the record is invalid.
    let record = format!(r#"{{"port":{port},"token":"","pid":1,"startedAt":1}}"#);
    mypass_platform::SecretStore::set(
        &store,
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
        pid: 1,
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
fn host_relays_one_request_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let token = generate_token();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let ep = Endpoint {
        port: listener.local_addr().unwrap().port(),
        token: token.clone(),
        pid: 1,
        started_at: 1,
    };
    save_endpoint(tmp.path(), &ep);
    let app = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
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
        &frame_of(r#"{"type":"getStatus","token":"evil"}"#),
    );
    assert!(ok);
    assert!(stderr.is_empty());
    let mut s = &stdout[..];
    let reply = read_frame(&mut s, MAX_FRAME_BYTES).unwrap().unwrap();
    assert_eq!(reply, br#"{"type":"ok"}"#);
    let (token, req) = app.join().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&req).unwrap();
    assert_eq!(v["token"], token.as_str());
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
