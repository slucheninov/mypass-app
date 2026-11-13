#![cfg(feature = "server")]
// test-only support code
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mypass_bridge::codec::{encode_frame, read_frame, MAX_FRAME_BYTES};
use mypass_bridge::endpoint::{
    generate_token, hello_proof, new_nonce, verify_proof, Endpoint, EndpointStore, Handshake,
};
use mypass_bridge::protocol::{Request, Response};
use mypass_bridge::server::{Handler, LoopbackServer, ServerConfig};
use mypass_platform::testing::InMemorySecretStore;

const GUARD: Duration = Duration::from_secs(10);

fn echo() -> (Handler, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&calls);
    let h: Handler = Arc::new(move |req| {
        c.fetch_add(1, Ordering::SeqCst);
        if matches!(req, Request::GetFavorites { .. }) {
            panic!("boom");
        }
        Response::Acknowledged
    });
    (h, calls)
}

fn start_with(cfg: ServerConfig) -> (LoopbackServer, Arc<EndpointStore>, Arc<AtomicUsize>) {
    let store = Arc::new(EndpointStore::new(InMemorySecretStore::new()));
    let (h, calls) = echo();
    let s = LoopbackServer::start(cfg, Arc::clone(&store), h).unwrap();
    (s, store, calls)
}

fn start() -> (LoopbackServer, Arc<EndpointStore>, Arc<AtomicUsize>) {
    start_with(ServerConfig::default())
}

fn connect(s: &LoopbackServer) -> TcpStream {
    let c = TcpStream::connect(("127.0.0.1", s.port())).unwrap();
    c.set_read_timeout(Some(GUARD)).unwrap();
    c
}

fn send(c: &mut TcpStream, json: &str) {
    c.write_all(&encode_frame(json.as_bytes()).unwrap())
        .unwrap();
}

fn recv(c: &mut TcpStream) -> Option<String> {
    read_frame(c, MAX_FRAME_BYTES)
        .ok()
        .flatten()
        .map(|b| String::from_utf8(b).unwrap())
}

fn hello(c: &mut TcpStream) -> (String, String) {
    let nonce = new_nonce();
    send(c, &format!(r#"{{"type":"hello","nonce":"{nonce}"}}"#));
    let reply = recv(c).unwrap();
    (nonce, reply)
}

fn authed(s: &LoopbackServer) -> TcpStream {
    let mut c = connect(s);
    hello(&mut c);
    c
}

fn fav(token: &str) -> String {
    format!(r#"{{"type":"getFavorites","token":"{token}"}}"#)
}

fn list(token: &str) -> String {
    format!(r#"{{"type":"searchEntries","token":"{token}","query":"x"}}"#)
}

fn eof(c: &mut TcpStream) -> bool {
    let mut b = [0u8; 1];
    matches!(c.read(&mut b), Ok(0) | Err(_))
}

fn wait_until(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + GUARD;
    while !f() {
        assert!(Instant::now() < end, "condition not reached");
        std::thread::yield_now();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn start_binds_loopback_and_publishes_endpoint() {
    let (s, store, _) = start();
    let e = store.load().unwrap().unwrap();
    assert_eq!(e.port, s.port());
    assert_eq!(e.token.len(), 44);
    assert_eq!(e.pid, std::process::id());
    assert_eq!(e, s.endpoint());
}

#[test]
fn fresh_token_on_every_start() {
    let (a, _, _) = start();
    let (b, _, _) = start();
    assert_ne!(a.endpoint().token, b.endpoint().token);
}

#[test]
fn valid_request_gets_handler_response() {
    let (s, _, calls) = start();
    let mut c = authed(&s);
    send(&mut c, &list(&s.endpoint().token));
    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn request_before_hello_is_refused_and_closed() {
    let (s, _, calls) = start();
    let mut c = connect(&s);
    send(&mut c, &list(&s.endpoint().token));
    assert_eq!(
        recv(&mut c).unwrap(),
        r#"{"type":"error","message":"hello required"}"#
    );
    assert!(eof(&mut c));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn invalid_token_gets_error_and_close() {
    let (s, _, calls) = start();
    let mut c = authed(&s);
    send(&mut c, &list(&generate_token()));
    assert_eq!(
        recv(&mut c).unwrap(),
        r#"{"type":"error","message":"invalid token"}"#
    );
    assert!(eof(&mut c));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn malformed_json_gets_malformed_request_and_stays_open() {
    let (s, _, _) = start();
    let mut c = authed(&s);
    send(&mut c, "{not json");
    assert_eq!(
        recv(&mut c).unwrap(),
        r#"{"type":"error","message":"malformed request"}"#
    );
    send(&mut c, &list(&s.endpoint().token));
    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
}

#[test]
fn oversized_frame_gets_error_and_close() {
    let (s, _, _) = start_with(ServerConfig {
        max_frame: 1024,
        ..ServerConfig::default()
    });
    let mut c = authed(&s);
    c.write_all(&2048u32.to_le_bytes()).unwrap();
    assert_eq!(
        recv(&mut c).unwrap(),
        r#"{"type":"error","message":"frame too large"}"#
    );
    assert!(eof(&mut c));
}

#[test]
fn frames_split_across_writes_and_pipelined_frames_both_work() {
    let (s, _, _) = start();
    let mut c = authed(&s);
    let one = encode_frame(list(&s.endpoint().token).as_bytes()).unwrap();
    let (a, b) = one.split_at(7);
    c.write_all(a).unwrap();
    c.flush().unwrap();
    c.write_all(b).unwrap();
    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
    let mut both = one.clone();
    both.extend_from_slice(&one);
    c.write_all(&both).unwrap();
    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
}

#[test]
fn seventeenth_connection_is_closed_unread() {
    let (s, _, _) = start();
    let held: Vec<TcpStream> = (0..16).map(|_| connect(&s)).collect();
    wait_until(|| s.active_connections() == 16);
    let mut extra = connect(&s);
    let mut b = [0u8; 1];
    assert!(matches!(extra.read(&mut b), Ok(0) | Err(_)));
    assert_eq!(s.active_connections(), 16);
    drop(held);
    wait_until(|| s.active_connections() == 0);
    let mut c = authed(&s);
    send(&mut c, &list(&s.endpoint().token));
    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
}

#[test]
fn silent_connection_is_closed_after_first_frame_deadline() {
    let (s, _, _) = start_with(ServerConfig {
        first_frame_deadline: Duration::from_millis(200),
        ..ServerConfig::default()
    });
    let mut c = connect(&s);
    assert!(eof(&mut c));
    wait_until(|| s.active_connections() == 0);
}

#[test]
fn half_frame_stall_is_closed() {
    let (s, _, _) = start_with(ServerConfig {
        first_frame_deadline: Duration::from_millis(200),
        ..ServerConfig::default()
    });
    let mut c = authed(&s);
    c.write_all(&[9, 0]).unwrap();
    assert!(eof(&mut c));
}

#[test]
fn connection_that_sent_a_valid_frame_is_not_subject_to_deadline() {
    let (s, _, _) = start_with(ServerConfig {
        first_frame_deadline: Duration::from_millis(200),
        ..ServerConfig::default()
    });
    let mut c = authed(&s);
    std::thread::sleep(Duration::from_millis(600));
    send(&mut c, &list(&s.endpoint().token));
    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
}

#[test]
fn hello_gets_ack_with_valid_proof_and_is_not_a_request() {
    let (s, _, calls) = start();
    let mut c = connect(&s);
    let (nonce, reply) = hello(&mut c);
    let Handshake::HelloAck { proof } = serde_json::from_str(&reply).unwrap() else {
        panic!("not an ack");
    };
    assert!(verify_proof(&s.endpoint().token, &nonce, &proof));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn hello_proof_fails_for_wrong_token() {
    let (s, _, _) = start();
    let mut c = connect(&s);
    let (nonce, reply) = hello(&mut c);
    let Handshake::HelloAck { proof } = serde_json::from_str(&reply).unwrap() else {
        panic!("not an ack");
    };
    assert!(!verify_proof(&generate_token(), &nonce, &proof));
    assert_ne!(proof, hello_proof(&generate_token(), &nonce));
}

#[test]
fn hello_with_bad_nonce_closes_the_connection() {
    let (s, _, _) = start();
    for bad in [
        "",
        "abc",
        "!!!!!!!!!!!!!!!!!!!!!!!!",
        "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=",
    ] {
        let mut c = connect(&s);
        send(&mut c, &format!(r#"{{"type":"hello","nonce":"{bad}"}}"#));
        assert!(eof(&mut c), "nonce {bad:?}");
    }
}

#[test]
fn listener_is_ipv4_loopback_only() {
    let (s, _, _) = start();
    let c = connect(&s);
    let local: SocketAddr = c.peer_addr().unwrap();
    assert_eq!(local, SocketAddr::from(([127, 0, 0, 1], s.port())));
}

#[test]
fn shutdown_closes_connections_and_clears_own_endpoint() {
    let (s, store, _) = start();
    let mut c = authed(&s);
    s.shutdown();
    assert!(eof(&mut c));
    assert_eq!(store.load().unwrap(), None);
    s.shutdown();
}

#[test]
fn shutdown_removes_only_its_own_endpoint() {
    let (s, store, _) = start();
    let other = Endpoint {
        port: 4242,
        token: generate_token(),
        pid: 1,
        started_at: 1,
    };
    store.save(&other).unwrap();
    s.shutdown();
    assert_eq!(store.load().unwrap(), Some(other));
}

#[test]
fn handler_panic_closes_only_that_connection_and_server_keeps_serving() {
    let (s, _, _) = start();
    let mut bad = authed(&s);
    send(&mut bad, &fav(&s.endpoint().token));
    assert!(eof(&mut bad));
    wait_until(|| s.active_connections() == 0);
    let mut ok = authed(&s);
    send(&mut ok, &list(&s.endpoint().token));
    assert_eq!(recv(&mut ok).unwrap(), r#"{"type":"ok"}"#);
}

#[test]
fn concurrent_clients_get_their_own_responses() {
    let (s, _, calls) = start();
    let token = s.endpoint().token.clone();
    let port = s.port();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let token = token.clone();
            std::thread::spawn(move || {
                let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
                c.set_read_timeout(Some(GUARD)).unwrap();
                hello(&mut c);
                for _ in 0..50 {
                    send(&mut c, &list(&token));
                    assert_eq!(recv(&mut c).unwrap(), r#"{"type":"ok"}"#);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 400);
}
