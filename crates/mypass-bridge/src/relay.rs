//! The native messaging host's relay: Chromium's stdin and stdout on one side, the app's loopback
//! server on the other.
//!
//! The host is the client of the server. It draws a fresh nonce per connection, sends `hello`, and
//! verifies the app's `helloAck` proof (role label `app-proves-to-host`) BEFORE anything that
//! carries the token leaves the process: a squatter on a stale port, or a peer that reflects the
//! host's own bytes, never learns the token. The host never answers a hello. After the handshake
//! each request from the extension gets the endpoint token injected (overwriting whatever the
//! extension sent) and is forwarded; a reader thread copies response frames to stdout.
//!
//! Output discipline: stdout carries frames only, nothing here prints, and no diagnostic names the
//! token, a request or a response. Buffers that hold requests or responses are wiped on drop
//! (`Zeroizing`); the serde value tree built while injecting the token is the one place that is not.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::codec::{encode_frame, encode_frame_capped, read_frame, MAX_FRAME_BYTES};
use crate::endpoint::{new_nonce, verify_proof, Endpoint, EndpointStore, Handshake};

/// Host-only error message: the app is not running, not reachable or not who it claims to be.
const APP_NOT_RUNNING: &str = "app_not_running";
/// Host-only error message: a frame from the extension was not a JSON object.
const MALFORMED_REQUEST: &str = "malformed_request";
/// A `helloAck` is under 100 bytes; anything bigger is not one.
const HANDSHAKE_MAX_FRAME: usize = 4096;

/// Limits of the relay.
#[derive(Debug, Clone)]
pub struct RelayOptions {
    /// Time allowed to connect to the app and to receive its `helloAck`.
    pub connect_timeout: Duration,
    /// Time allowed, after stdin closes, for responses to requests still in flight.
    pub drain_timeout: Duration,
    /// Largest frame accepted from either side.
    pub max_frame: usize,
}

impl Default for RelayOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            drain_timeout: Duration::from_secs(5),
            max_frame: MAX_FRAME_BYTES,
        }
    }
}

/// How a relay run ended. The host exits 0 in every case; Chromium treats anything else as a crash.
#[derive(Debug, PartialEq, Eq)]
pub enum RelayOutcome {
    /// Chromium closed stdin and the relay drained.
    StdinClosed,
    /// No endpoint, no connection, or a peer that failed the handshake.
    AppNotRunning,
    /// stdin broke the framing (oversized or truncated frame); nothing more can be read.
    Failed(String),
}

/// Why a request could not get a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RelayError {
    /// Not valid JSON.
    #[error("request is not valid JSON")]
    Malformed,
    /// Valid JSON, but not an object.
    #[error("request is not a JSON object")]
    NotAnObject,
}

/// Sets (or overwrites) the `token` member of a JSON object request.
///
/// # Errors
/// `Malformed` for bytes that are not JSON, `NotAnObject` for any other JSON value.
pub fn inject_token(request_json: &[u8], token: &str) -> Result<Vec<u8>, RelayError> {
    let mut value: serde_json::Value =
        serde_json::from_slice(request_json).map_err(|_| RelayError::Malformed)?;
    let serde_json::Value::Object(map) = &mut value else {
        return Err(RelayError::NotAnObject);
    };
    map.insert(
        "token".to_owned(),
        serde_json::Value::String(token.to_owned()),
    );
    serde_json::to_vec(&value).map_err(|_| RelayError::Malformed)
}

/// A framed `{"type":"error","message":..}`, the shape the app itself uses for errors.
#[must_use]
pub fn error_frame(message: &str) -> Vec<u8> {
    let body = serde_json::json!({ "type": "error", "message": message });
    serde_json::to_vec(&body)
        .ok()
        .and_then(|json| encode_frame(&json).ok())
        .unwrap_or_default()
}

/// The endpoint record, or `None` when it is missing, malformed or refused by the store (wrong
/// mode, symlink, foreign owner). Every such case means "MyPass is not running" to the extension.
#[must_use]
pub fn load_endpoint(store: &EndpointStore) -> Option<Endpoint> {
    store.load().ok().flatten()
}

/// Request bookkeeping shared by the stdin loop and the response pump.
#[derive(Default)]
struct Pending {
    /// Requests written to the app that have no response yet.
    count: usize,
    /// The app side is gone.
    closed: bool,
    /// The relay itself is tearing the connection down; a closed socket is then expected.
    closing: bool,
}

struct Shared {
    state: Mutex<Pending>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Writes one complete frame to stdout and flushes; `false` when Chromium is gone.
fn write_out<W: Write>(out: &Mutex<W>, frame: &[u8]) -> bool {
    let mut w = out.lock().unwrap_or_else(PoisonError::into_inner);
    w.write_all(frame).is_ok() && w.flush().is_ok()
}

/// Reads with an absolute deadline, so a peer that trickles bytes cannot stretch the handshake.
struct DeadlineReader<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(remaining))?;
        (&*self.stream).read(buf)
    }
}

/// Sends `hello` with a fresh nonce and checks the proof in the answer. Nothing that carries the
/// token is written here; `true` only for a correct `helloAck`.
fn handshake(stream: &mut TcpStream, token: &str, timeout: Duration) -> bool {
    let nonce = new_nonce();
    let Ok(hello) = serde_json::to_vec(&Handshake::Hello {
        nonce: nonce.clone(),
    }) else {
        return false;
    };
    let Ok(framed) = encode_frame(&hello) else {
        return false;
    };
    if stream.set_write_timeout(Some(timeout)).is_err()
        || stream.write_all(&framed).is_err()
        || stream.flush().is_err()
    {
        return false;
    }
    let mut reader = DeadlineReader {
        stream,
        deadline: Instant::now() + timeout,
    };
    let Ok(Some(reply)) = read_frame(&mut reader, HANDSHAKE_MAX_FRAME) else {
        return false;
    };
    match serde_json::from_slice::<Handshake>(&reply) {
        Ok(Handshake::HelloAck { proof }) => verify_proof(token, &nonce, &proof),
        // A Hello (the host's own frame reflected back) or junk is never a valid answer.
        Ok(Handshake::Hello { .. }) | Err(_) => false,
    }
}

/// Copies response frames from the app to stdout until the connection ends.
fn pump<W: Write + Send + 'static>(
    mut stream: TcpStream,
    out: &Mutex<W>,
    shared: &Shared,
    max_frame: usize,
) {
    while let Ok(Some(payload)) = read_frame(&mut stream, max_frame) {
        let payload = Zeroizing::new(payload);
        let framed = match encode_frame(&payload) {
            Ok(f) => Zeroizing::new(f),
            // Chromium refuses a host message above 1 MiB.
            Err(_) => Zeroizing::new(error_frame("response_too_large")),
        };
        let delivered = write_out(out, &framed);
        {
            let mut st = shared.lock();
            st.count = st.count.saturating_sub(1);
        }
        shared.changed.notify_all();
        if !delivered {
            break;
        }
    }
    let unexpected = {
        let mut st = shared.lock();
        st.closed = true;
        !st.closing
    };
    shared.changed.notify_all();
    if unexpected {
        // Tell the extension, so it reconnects; the app may have quit or the server may have
        // closed an idle connection.
        write_out(out, &error_frame(APP_NOT_RUNNING));
    }
}

/// Marks the teardown as intended, closes the socket and waits for the pump.
fn teardown(stream: &TcpStream, shared: &Shared, pump: JoinHandle<()>) {
    shared.lock().closing = true;
    let _ = stream.shutdown(Shutdown::Both);
    let _ = pump.join();
}

/// Waits until every request has its response, the app closed, or `limit` ran out.
fn drain(shared: &Shared, limit: Duration) {
    let deadline = Instant::now() + limit;
    let mut st = shared.lock();
    while st.count > 0 && !st.closed {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        st = shared
            .changed
            .wait_timeout(st, remaining)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// Runs the relay until stdin closes, the framing breaks, or the app goes away.
///
/// `endpoint` of `None` (no record, or one the store refused) answers `app_not_running` at once.
pub fn run_relay<R: Read, W: Write + Send + 'static>(
    mut stdin: R,
    stdout: W,
    endpoint: Option<Endpoint>,
    opts: &RelayOptions,
) -> RelayOutcome {
    let out = Arc::new(Mutex::new(stdout));
    let not_running = |out: &Mutex<W>| {
        write_out(out, &error_frame(APP_NOT_RUNNING));
        RelayOutcome::AppNotRunning
    };
    let Some(endpoint) = endpoint else {
        return not_running(&out);
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, opts.connect_timeout) else {
        return not_running(&out);
    };
    let _ = stream.set_nodelay(true);
    if !handshake(&mut stream, &endpoint.token, opts.connect_timeout) {
        let _ = stream.shutdown(Shutdown::Both);
        return not_running(&out);
    }
    // The handshake left a read deadline on the socket; the pump blocks until a response arrives.
    let _ = stream.set_read_timeout(None);
    let Ok(read_half) = stream.try_clone() else {
        return not_running(&out);
    };
    let shared = Arc::new(Shared {
        state: Mutex::new(Pending::default()),
        changed: Condvar::new(),
    });
    let pump_handle = {
        let (out, shared, max) = (Arc::clone(&out), Arc::clone(&shared), opts.max_frame);
        thread::spawn(move || pump(read_half, &out, &shared, max))
    };

    let outcome = loop {
        let frame = match read_frame(&mut stdin, opts.max_frame) {
            Ok(Some(frame)) => Zeroizing::new(frame),
            Ok(None) => {
                drain(&shared, opts.drain_timeout);
                break RelayOutcome::StdinClosed;
            }
            // An oversized or truncated frame leaves the stream unframed: stop.
            Err(e) => break RelayOutcome::Failed(format!("stdin: {}", e.kind())),
        };
        let framed = inject_token(&frame, &endpoint.token)
            .map(Zeroizing::new)
            .ok()
            .and_then(|json| encode_frame_capped(&json, opts.max_frame).ok())
            .map(Zeroizing::new);
        let Some(framed) = framed else {
            write_out(&out, &error_frame(MALFORMED_REQUEST));
            continue;
        };
        {
            let mut st = shared.lock();
            if st.closed {
                drop(st);
                write_out(&out, &error_frame(APP_NOT_RUNNING));
                break RelayOutcome::AppNotRunning;
            }
            st.count = st.count.saturating_add(1);
        }
        if stream.write_all(&framed).is_err() || stream.flush().is_err() {
            write_out(&out, &error_frame(APP_NOT_RUNNING));
            break RelayOutcome::AppNotRunning;
        }
    };
    teardown(&stream, &shared, pump_handle);
    outcome
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test-only
mod tests {
    use super::*;
    use crate::codec::FrameReader;
    use crate::endpoint::{generate_token, hello_proof};
    use std::io::Cursor;
    use std::net::TcpListener;

    const TOKEN_FRAME_TYPE: &str = r#"{"type":"getStatus"}"#;

    // ---- helpers -------------------------------------------------------------------------

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SharedBuf {
        fn frames(&self) -> Vec<String> {
            let mut r = FrameReader::default_max();
            r.feed(&self.0.lock().unwrap())
                .unwrap()
                .into_iter()
                .map(|f| String::from_utf8(f).unwrap())
                .collect()
        }
    }

    fn frame(json: &str) -> Vec<u8> {
        encode_frame(json.as_bytes()).unwrap()
    }

    fn opts() -> RelayOptions {
        RelayOptions {
            connect_timeout: Duration::from_secs(2),
            drain_timeout: Duration::from_secs(2),
            max_frame: 64 * 1024,
        }
    }

    fn read_str(s: &mut TcpStream) -> Option<String> {
        read_frame(s, 64 * 1024)
            .ok()
            .flatten()
            .map(|b| String::from_utf8(b).unwrap())
    }

    fn nonce_of(hello: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(hello).unwrap();
        assert_eq!(v["type"], "hello");
        v["nonce"].as_str().unwrap().to_owned()
    }

    fn ack(s: &mut TcpStream, proof: &str) {
        let json = serde_json::to_vec(&Handshake::HelloAck {
            proof: proof.to_owned(),
        })
        .unwrap();
        s.write_all(&encode_frame(&json).unwrap()).unwrap();
    }

    /// An in-process fake app: accepts one connection and hands it to `behave`.
    fn fake_app<T: Send + 'static>(
        token: &str,
        behave: impl FnOnce(TcpStream, String) -> T + Send + 'static,
    ) -> (Endpoint, JoinHandle<T>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let tok = token.to_owned();
        let handle = thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            behave(s, tok)
        });
        let ep = Endpoint {
            port,
            token: token.to_owned(),
            pid: 1,
            started_at: 1,
        };
        (ep, handle)
    }

    /// A correct app: proves itself, then answers every request with `{"type":"ok"}` after
    /// `delay`. Returns the request frames it received.
    fn good_app(token: &str, delay: Duration) -> (Endpoint, JoinHandle<Vec<String>>) {
        fake_app(token, move |mut s, token| {
            let nonce = nonce_of(&read_str(&mut s).unwrap());
            ack(&mut s, &hello_proof(&token, &nonce));
            let mut seen = Vec::new();
            while let Some(req) = read_str(&mut s) {
                seen.push(req);
                thread::sleep(delay);
                s.write_all(&frame(r#"{"type":"ok"}"#)).unwrap();
            }
            seen
        })
    }

    /// A peer that fails the proof, then records everything it is sent until the host closes.
    fn recording_peer(
        token: &str,
        reply: impl FnOnce(&mut TcpStream, &str) + Send + 'static,
    ) -> (Endpoint, JoinHandle<Vec<u8>>) {
        fake_app(token, move |mut s, _| {
            let hello = read_str(&mut s).unwrap();
            reply(&mut s, &hello);
            let mut rest = Vec::new();
            let _ = s.read_to_end(&mut rest);
            let mut all = frame(&hello);
            all.extend(rest);
            all
        })
    }

    // ---- injector ------------------------------------------------------------------------

    #[test]
    fn injector_adds_token_to_object() {
        let out = inject_token(br#"{"type":"x","n":1}"#, "real").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["token"], "real");
        assert_eq!(v["type"], "x");
        assert_eq!(v["n"], 1);
    }

    #[test]
    fn injector_overwrites_extension_supplied_token() {
        let out = inject_token(br#"{"type":"x","token":"evil"}"#, "real").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["token"], "real");
        let out = inject_token(br#"{"token":"a","token":"b"}"#, "real").unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out).unwrap()["token"],
            "real"
        );
    }

    #[test]
    fn injector_rejects_malformed_json_and_non_object() {
        for bad in [&b"[1,2]"[..], b"\"s\"", b"42", b"null"] {
            assert_eq!(inject_token(bad, "t"), Err(RelayError::NotAnObject));
        }
        for bad in [&b"{\"type\":"[..], b"", b"not json", b"{\"a\":1} x"] {
            assert_eq!(inject_token(bad, "t"), Err(RelayError::Malformed));
        }
    }

    #[test]
    fn error_frame_is_a_framed_error_object() {
        let f = error_frame("app_not_running");
        let mut r = FrameReader::default_max();
        let frames = r.feed(&f).unwrap();
        assert_eq!(
            frames,
            vec![br#"{"message":"app_not_running","type":"error"}"#.to_vec()]
        );
    }

    // ---- connection failures ---------------------------------------------------------------

    #[test]
    fn no_endpoint_answers_app_not_running() {
        let out = SharedBuf::default();
        let r = run_relay(Cursor::new(Vec::new()), out.clone(), None, &opts());
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert_eq!(out.frames().len(), 1);
        assert!(out.frames()[0].contains("app_not_running"));
    }

    #[test]
    fn refused_port_answers_app_not_running_within_connect_timeout() {
        let port = {
            let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        let ep = Endpoint {
            port,
            token: generate_token(),
            pid: 1,
            started_at: 1,
        };
        let out = SharedBuf::default();
        let start = Instant::now();
        let o = RelayOptions {
            connect_timeout: Duration::from_secs(1),
            ..opts()
        };
        let r = run_relay(Cursor::new(Vec::new()), out.clone(), Some(ep), &o);
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(out.frames()[0].contains("app_not_running"));
    }

    #[test]
    fn peer_that_closes_before_ack_is_app_not_running() {
        let (ep, peer) = fake_app(&generate_token(), |mut s, _| {
            read_str(&mut s).unwrap();
        });
        let out = SharedBuf::default();
        let r = run_relay(
            Cursor::new(frame(TOKEN_FRAME_TYPE)),
            out.clone(),
            Some(ep),
            &opts(),
        );
        peer.join().unwrap();
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert_eq!(out.frames().len(), 1);
        assert!(out.frames()[0].contains("app_not_running"));
    }

    #[test]
    fn silent_peer_times_out_at_connect_timeout() {
        let (ep, peer) = fake_app(&generate_token(), |mut s, _| {
            read_str(&mut s).unwrap();
            let mut sink = Vec::new();
            let _ = s.read_to_end(&mut sink);
        });
        let out = SharedBuf::default();
        let o = RelayOptions {
            connect_timeout: Duration::from_millis(300),
            ..opts()
        };
        let start = Instant::now();
        let r = run_relay(Cursor::new(Vec::new()), out.clone(), Some(ep), &o);
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert!(start.elapsed() < Duration::from_secs(2));
        peer.join().unwrap();
    }

    // ---- the handshake: the token never reaches a peer that fails it -------------------------

    fn assert_peer_saw_only_the_hello(seen: &[u8], token: &str) {
        let mut r = FrameReader::default_max();
        assert_eq!(
            r.feed(seen).unwrap().len(),
            1,
            "the peer saw more than the hello"
        );
        assert!(!String::from_utf8_lossy(seen).contains(token));
    }

    fn run_against(ep: Endpoint) -> (RelayOutcome, Vec<String>) {
        let out = SharedBuf::default();
        let input = frame(&format!(
            r#"{{"type":"getStatus","note":"{TOKEN_FRAME_TYPE}"}}"#
        ));
        let r = run_relay(Cursor::new(input), out.clone(), Some(ep), &opts());
        (r, out.frames())
    }

    #[test]
    fn host_never_sends_token_to_peer_that_fails_the_hello_proof() {
        let token = generate_token();
        let (ep, peer) = recording_peer(&token, |s, _| ack(s, "00".repeat(32).as_str()));
        let (r, out) = run_against(ep);
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert!(out[0].contains("app_not_running"));
        assert_peer_saw_only_the_hello(&peer.join().unwrap(), &token);
    }

    #[test]
    fn squatter_that_does_not_know_the_token_gets_no_token() {
        let token = generate_token();
        let (ep, peer) = recording_peer(&token, |s, hello| {
            ack(s, &hello_proof(&generate_token(), &nonce_of(hello)));
        });
        let (r, _) = run_against(ep);
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert_peer_saw_only_the_hello(&peer.join().unwrap(), &token);
    }

    #[test]
    fn reflecting_the_hello_back_is_rejected() {
        let token = generate_token();
        let (ep, peer) = recording_peer(&token, |s, hello| {
            s.write_all(&frame(hello)).unwrap();
        });
        let (r, _) = run_against(ep);
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert_peer_saw_only_the_hello(&peer.join().unwrap(), &token);
    }

    #[test]
    fn echoing_the_hosts_nonce_as_the_proof_is_rejected() {
        let token = generate_token();
        let (ep, peer) = recording_peer(&token, |s, hello| ack(s, &nonce_of(hello)));
        let (r, _) = run_against(ep);
        assert_eq!(r, RelayOutcome::AppNotRunning);
        assert_peer_saw_only_the_hello(&peer.join().unwrap(), &token);
    }

    #[test]
    fn every_connection_draws_a_fresh_nonce() {
        let mut nonces = Vec::new();
        for _ in 0..2 {
            let token = generate_token();
            let (ep, peer) = recording_peer(&token, |s, hello| ack(s, &nonce_of(hello)));
            run_against(ep);
            let seen = peer.join().unwrap();
            let mut r = FrameReader::default_max();
            let hello = String::from_utf8(r.feed(&seen).unwrap().remove(0)).unwrap();
            nonces.push(nonce_of(&hello));
        }
        assert_ne!(nonces[0], nonces[1]);
    }

    // ---- forwarding ------------------------------------------------------------------------

    #[test]
    fn relay_forwards_request_with_injected_token_and_returns_response() {
        let token = generate_token();
        let (ep, app) = good_app(&token, Duration::ZERO);
        let out = SharedBuf::default();
        let input = frame(r#"{"type":"getStatus","token":"evil"}"#);
        let r = run_relay(Cursor::new(input), out.clone(), Some(ep), &opts());
        assert_eq!(r, RelayOutcome::StdinClosed);
        assert_eq!(out.frames(), vec![r#"{"type":"ok"}"#.to_owned()]);
        let seen = app.join().unwrap();
        assert_eq!(seen.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&seen[0]).unwrap();
        assert_eq!(v["token"], token.as_str());
        assert_eq!(v["type"], "getStatus");
    }

    #[test]
    fn malformed_stdin_frame_answers_malformed_request_and_relay_continues() {
        let (ep, app) = good_app(&generate_token(), Duration::ZERO);
        let out = SharedBuf::default();
        let mut input = frame("[1,2]");
        input.extend(frame("not json"));
        input.extend(frame(TOKEN_FRAME_TYPE));
        let r = run_relay(Cursor::new(input), out.clone(), Some(ep), &opts());
        assert_eq!(r, RelayOutcome::StdinClosed);
        let frames = out.frames();
        assert_eq!(frames.len(), 3);
        assert_eq!(
            frames
                .iter()
                .filter(|f| f.contains("malformed_request"))
                .count(),
            2
        );
        assert!(frames.contains(&r#"{"type":"ok"}"#.to_owned()));
        assert_eq!(app.join().unwrap().len(), 1);
    }

    #[test]
    fn stdin_eof_waits_for_inflight_response_then_exits() {
        let (ep, _app) = good_app(&generate_token(), Duration::from_millis(300));
        let out = SharedBuf::default();
        let start = Instant::now();
        let r = run_relay(
            Cursor::new(frame(TOKEN_FRAME_TYPE)),
            out.clone(),
            Some(ep),
            &opts(),
        );
        assert_eq!(r, RelayOutcome::StdinClosed);
        assert!(start.elapsed() >= Duration::from_millis(250));
        assert_eq!(out.frames(), vec![r#"{"type":"ok"}"#.to_owned()]);
    }

    #[test]
    fn drain_timeout_bounds_the_wait() {
        let (ep, _app) = good_app(&generate_token(), Duration::from_secs(30));
        let out = SharedBuf::default();
        let o = RelayOptions {
            drain_timeout: Duration::from_millis(300),
            ..opts()
        };
        let start = Instant::now();
        let r = run_relay(
            Cursor::new(frame(TOKEN_FRAME_TYPE)),
            out.clone(),
            Some(ep),
            &o,
        );
        assert_eq!(r, RelayOutcome::StdinClosed);
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(250) && took < Duration::from_secs(2));
        assert!(out.frames().is_empty());
    }

    #[test]
    fn oversized_stdin_frame_stops_relay_without_writing_to_peer() {
        let token = generate_token();
        let (ep, app) = fake_app(&token, |mut s, token| {
            let nonce = nonce_of(&read_str(&mut s).unwrap());
            ack(&mut s, &hello_proof(&token, &nonce));
            let mut rest = Vec::new();
            let _ = s.read_to_end(&mut rest);
            rest
        });
        let o = RelayOptions {
            max_frame: 1024,
            ..opts()
        };
        let mut input = 2000u32.to_le_bytes().to_vec();
        input.extend(vec![b'x'; 2000]);
        let out = SharedBuf::default();
        let r = run_relay(Cursor::new(input), out.clone(), Some(ep), &o);
        assert!(matches!(r, RelayOutcome::Failed(_)));
        assert!(app.join().unwrap().is_empty());
    }

    #[test]
    fn peer_that_closes_after_the_handshake_is_reported_once_and_ends_the_relay() {
        let (ep, app) = fake_app(&generate_token(), |mut s, token| {
            let nonce = nonce_of(&read_str(&mut s).unwrap());
            ack(&mut s, &hello_proof(&token, &nonce));
            read_str(&mut s).unwrap();
        });
        let out = SharedBuf::default();
        let r = run_relay(
            Cursor::new(frame(TOKEN_FRAME_TYPE)),
            out.clone(),
            Some(ep),
            &opts(),
        );
        app.join().unwrap();
        assert_eq!(r, RelayOutcome::StdinClosed);
        assert_eq!(out.frames().len(), 1);
        assert!(out.frames()[0].contains("app_not_running"));
    }
}
