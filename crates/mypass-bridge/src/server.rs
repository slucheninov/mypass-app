//! The app-side loopback server the native messaging host connects to.
//!
//! Hardening, in the order a hostile local process meets it: the listener binds `127.0.0.1` only
//! (never a wildcard, never a name that resolves), a connection beyond the cap is closed before a
//! byte is read, a connection must deliver its first complete frame (and every later frame, once
//! started) within a deadline so a stalled client cannot hold a slot, inbound frames are capped,
//! no request is served before the connection has completed a `hello` exchange, and every request
//! carries the endpoint token, compared in constant time. Malformed framing closes the connection;
//! nothing here panics on input, and nothing logs a token or a request body.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{prelude::BASE64_STANDARD as B64, Engine};
use zeroize::Zeroizing;

use crate::codec::{encode_frame, FrameReader, MAX_FRAME_BYTES};
use crate::endpoint::{
    generate_token, hello_proof, tokens_equal, Endpoint, EndpointStore, Handshake,
};
use crate::protocol::{Request, Response};

/// A peer nonce is 16 random bytes: 24 base64 characters with padding.
const NONCE_CHARS: usize = 24;
const NONCE_BYTES: usize = 16;
/// How often a blocked read wakes to check the shutdown flag and the deadlines.
const POLL: Duration = Duration::from_millis(100);

/// Limits of the server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Simultaneous connections; the next one is closed unread.
    pub max_connections: usize,
    /// Largest inbound frame.
    pub max_frame: usize,
    /// Time allowed for the first complete frame, and for finishing any frame once it has started.
    pub first_frame_deadline: Duration,
    /// Time allowed for a write to a peer that does not read.
    pub write_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 16,
            max_frame: MAX_FRAME_BYTES,
            first_frame_deadline: Duration::from_secs(10),
            write_timeout: Duration::from_secs(5),
        }
    }
}

/// Answers one authenticated request.
pub type Handler = Arc<dyn Fn(Request) -> Response + Send + Sync>;

/// Failures of `start`.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The loopback listener could not be created.
    #[error("bind failed: {0}")]
    Bind(String),
    /// The endpoint record could not be published.
    #[error("endpoint store: {0}")]
    Endpoint(String),
}

struct Shared {
    cfg: ServerConfig,
    token: Zeroizing<String>,
    handler: Handler,
    shutdown: AtomicBool,
    active: AtomicUsize,
    next_id: AtomicU64,
    conns: Mutex<HashMap<u64, TcpStream>>,
}

/// Releases the connection slot and forgets the stream, also when the thread panics.
struct ConnGuard {
    shared: Arc<Shared>,
    id: u64,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.shared
            .conns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
        self.shared.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The running server.
pub struct LoopbackServer {
    shared: Arc<Shared>,
    endpoints: Arc<EndpointStore>,
    published: Endpoint,
    addr: SocketAddr,
    listener: Mutex<Option<JoinHandle<()>>>,
}

impl LoopbackServer {
    /// Binds `127.0.0.1` on an ephemeral port, makes a fresh token and publishes the endpoint.
    ///
    /// # Errors
    /// `Bind` when the listener fails, `Endpoint` when the record cannot be stored.
    pub fn start(
        cfg: ServerConfig,
        endpoints: Arc<EndpointStore>,
        handler: Handler,
    ) -> Result<Self, ServerError> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .map_err(|e| ServerError::Bind(e.to_string()))?;
        let addr = listener
            .local_addr()
            .map_err(|e| ServerError::Bind(e.to_string()))?;
        let token = generate_token();
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok())
            .unwrap_or(0);
        let published = Endpoint {
            port: addr.port(),
            token: token.clone(),
            pid: std::process::id(),
            started_at,
        };
        endpoints
            .save(&published)
            .map_err(|e| ServerError::Endpoint(e.to_string()))?;
        let shared = Arc::new(Shared {
            cfg,
            token: Zeroizing::new(token),
            handler,
            shutdown: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            next_id: AtomicU64::new(0),
            conns: Mutex::new(HashMap::new()),
        });
        let accept_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("bridge-accept".into())
            .spawn(move || accept_loop(&listener, &accept_shared))
            .map_err(|e| {
                let _ = endpoints.clear();
                ServerError::Bind(e.to_string())
            })?;
        Ok(Self {
            shared,
            endpoints,
            published,
            addr,
            listener: Mutex::new(Some(handle)),
        })
    }

    /// The listening port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The record this server published.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.published.clone()
    }

    /// Connections currently held.
    #[must_use]
    pub fn active_connections(&self) -> usize {
        self.shared.active.load(Ordering::SeqCst)
    }

    /// Stops accepting, closes every connection and clears the stored endpoint when, and only when,
    /// it is still the one this server published (a newer launch's record is left alone).
    pub fn shutdown(&self) {
        if self.shared.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        // Wake the blocking accept so the listener thread sees the flag.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        let handle = self
            .listener
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(h) = handle {
            let _ = h.join();
        }
        for stream in self
            .shared
            .conns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        // `Endpoint` equality compares the token with `tokens_equal`.
        if let Ok(Some(stored)) = self.endpoints.load() {
            if stored == self.published {
                let _ = self.endpoints.clear();
            }
        }
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    for incoming in listener.incoming() {
        if shared.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = incoming else { continue };
        let loopback = stream.peer_addr().is_ok_and(|p| p.ip().is_loopback());
        if !loopback || shared.active.load(Ordering::SeqCst) >= shared.cfg.max_connections {
            drop(stream);
            continue;
        }
        let Ok(registered) = stream.try_clone() else {
            continue;
        };
        let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
        shared.active.fetch_add(1, Ordering::SeqCst);
        shared
            .conns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, registered);
        let guard = ConnGuard {
            shared: Arc::clone(shared),
            id,
        };
        let conn_shared = Arc::clone(shared);
        // A failed spawn drops the closure and with it the guard, which frees the slot.
        let _ = std::thread::Builder::new()
            .name("bridge-conn".into())
            .spawn(move || {
                let _guard = guard;
                serve(stream, &conn_shared);
            });
    }
}

enum Flow {
    Continue,
    Close,
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let poll = POLL.min(
        shared
            .cfg
            .first_frame_deadline
            .max(Duration::from_millis(1)),
    );
    if stream.set_read_timeout(Some(poll)).is_err()
        || stream
            .set_write_timeout(Some(shared.cfg.write_timeout))
            .is_err()
    {
        return;
    }
    let _ = stream.set_nodelay(true);
    let mut reader = FrameReader::new(shared.cfg.max_frame);
    let mut hello_done = false;
    // Set while a frame is awaited for the first time or half received; idle between frames is free.
    let mut waiting_since = Some(Instant::now());
    let mut fed: usize = 0;
    let mut consumed: usize = 0;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        if shared.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let n = match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if waiting_since.is_some_and(|t| t.elapsed() >= shared.cfg.first_frame_deadline) {
                    return;
                }
                continue;
            }
            Err(_) => return,
        };
        let Some(chunk) = buf.get(..n) else { return };
        fed = fed.saturating_add(n);
        let frames = match reader.feed(chunk) {
            Ok(f) => f,
            Err(_) => {
                send(&mut stream, &error("frame too large"));
                return;
            }
        };
        let got_frames = !frames.is_empty();
        for frame in &frames {
            consumed = consumed.saturating_add(frame.len().saturating_add(4));
        }
        if got_frames {
            waiting_since = None;
        }
        if fed > consumed && waiting_since.is_none() {
            waiting_since = Some(Instant::now());
        }
        for frame in frames {
            if matches!(
                handle_frame(&mut stream, shared, &frame, &mut hello_done),
                Flow::Close
            ) {
                return;
            }
        }
    }
}

fn handle_frame(
    stream: &mut TcpStream,
    shared: &Shared,
    frame: &[u8],
    hello_done: &mut bool,
) -> Flow {
    match serde_json::from_slice::<Handshake>(frame) {
        Ok(Handshake::Hello { nonce }) => {
            let valid = nonce.len() == NONCE_CHARS
                && B64
                    .decode(nonce.as_bytes())
                    .is_ok_and(|b| b.len() == NONCE_BYTES);
            if !valid {
                return Flow::Close;
            }
            let proof = hello_proof(&shared.token, &nonce);
            *hello_done = true;
            return flow(send(
                stream,
                &HelloReply::Ack(Handshake::HelloAck { proof }),
            ));
        }
        // A client never sends the ack.
        Ok(Handshake::HelloAck { .. }) => {
            return flow(send(stream, &error("malformed request")));
        }
        Err(_) => {}
    }
    if !*hello_done {
        send(stream, &error("hello required"));
        return Flow::Close;
    }
    let Ok(request) = serde_json::from_slice::<Request>(frame) else {
        return flow(send(stream, &error("malformed request")));
    };
    if !tokens_equal(request.token(), &shared.token) {
        send(stream, &error("invalid token"));
        return Flow::Close;
    }
    let handler = Arc::clone(&shared.handler);
    match catch_unwind(AssertUnwindSafe(|| handler(request))) {
        Ok(response) => flow(send(stream, &HelloReply::Resp(response))),
        Err(_) => Flow::Close,
    }
}

fn flow(ok: bool) -> Flow {
    if ok {
        Flow::Continue
    } else {
        Flow::Close
    }
}

fn error(message: &str) -> HelloReply {
    HelloReply::Resp(Response::Error {
        message: message.to_owned(),
    })
}

/// What the server writes: a handshake answer or a response.
enum HelloReply {
    Ack(Handshake),
    Resp(Response),
}

/// Frames and writes `reply`; `false` when the peer is gone or stalled. A response too large for
/// the outbound cap is replaced by an error instead of being dropped silently.
fn send(stream: &mut TcpStream, reply: &HelloReply) -> bool {
    let json = match reply {
        HelloReply::Ack(h) => serde_json::to_vec(h),
        HelloReply::Resp(r) => serde_json::to_vec(r),
    };
    let Ok(json) = json else { return false };
    let framed = match encode_frame(&json) {
        Ok(f) => f,
        Err(_) => {
            let Ok(small) = serde_json::to_vec(&Response::Error {
                message: "response too large".into(),
            }) else {
                return false;
            };
            match encode_frame(&small) {
                Ok(f) => f,
                Err(_) => return false,
            }
        }
    };
    stream.write_all(&framed).is_ok() && stream.flush().is_ok()
}
