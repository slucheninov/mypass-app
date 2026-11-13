//! The endpoint record (port and token), the token generator and the hello proof.
//!
//! The app writes the record into the host-shared secret store (`Platform.shared_secrets`, a 0600
//! file on macOS and Windows) once its loopback server is listening; the native messaging host
//! reads it and connects.
//!
//! Why the handshake exists: a crashed app leaves its record behind and another local process can
//! bind the same port. Without a proof the host would hand that process the token and the
//! extension's requests. So before any request leaves the host, the host sends a fresh nonce and the
//! peer must answer with `HMAC-SHA256(token, domain || nonce)`. The token itself never crosses the
//! wire in the handshake, and a squatter that does not hold the current token cannot answer.

use std::sync::Arc;

use base64::{prelude::BASE64_STANDARD as B64, Engine};
use hmac::{Hmac, KeyInit, Mac};
use mypass_platform::SecretStore;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

/// Secret-store service of the endpoint record.
pub const ENDPOINT_SERVICE: &str = "com.mypass.extensionbridge.endpoint";
/// Secret-store account of the endpoint record.
pub const ENDPOINT_ACCOUNT: &str = "loopback";

/// Domain separation prefix of the hello proof.
const PROOF_DOMAIN: &[u8] = b"mypass-bridge-hello-v1";
/// A real record is under 150 bytes; anything larger is hostile or corrupt.
const MAX_RECORD_BYTES: usize = 4096;
/// A generated token is 44 characters; the cap leaves room without accepting junk.
const MAX_TOKEN_CHARS: usize = 256;

/// Failures of the endpoint store. No variant carries record content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// The underlying secret store failed.
    #[error("endpoint store unavailable: {0}")]
    Store(String),
    /// The stored record is not a valid endpoint record.
    #[error("the stored endpoint record is invalid")]
    Invalid,
}

/// Where the app's loopback server listens and the token that authenticates both ends.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    /// Loopback TCP port.
    pub port: u16,
    /// Shared secret, standard base64 of 32 random bytes.
    pub token: String,
    /// Process id of the app that wrote the record (0 when unknown).
    #[serde(default)]
    pub pid: u32,
    /// Unix seconds when the server started (0 when unknown).
    #[serde(default)]
    pub started_at: i64,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .field("pid", &self.pid)
            .field("started_at", &self.started_at)
            .finish()
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

impl Endpoint {
    /// Hostile-input checks applied to every record read back.
    fn is_valid(&self) -> bool {
        self.port != 0
            && !self.token.is_empty()
            && self.token.len() <= MAX_TOKEN_CHARS
            && self.token.is_ascii()
    }
}

/// A fresh token: 32 bytes from the OS CSPRNG, standard base64 (44 characters).
#[must_use]
pub fn generate_token() -> String {
    random_b64::<32>()
}

/// A fresh handshake nonce: 16 bytes from the OS CSPRNG, base64.
#[must_use]
pub fn new_nonce() -> String {
    random_b64::<16>()
}

fn random_b64<const N: usize>() -> String {
    let mut bytes = Zeroizing::new([0u8; N]);
    // `OsRng` panics if the OS entropy source fails, the same contract as the generator.
    rand::rngs::OsRng.fill_bytes(&mut *bytes);
    B64.encode(*bytes)
}

/// Constant-time token comparison; empty or different lengths never match.
#[must_use]
pub fn tokens_equal(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// Stores the endpoint record in a host-shared secret store.
pub struct EndpointStore {
    secrets: Arc<dyn SecretStore>,
}

impl EndpointStore {
    /// Wraps the shared secret store.
    #[must_use]
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }

    /// Writes (or replaces) the record.
    ///
    /// # Errors
    /// `Invalid` for a record that would not read back, `Store` when the store fails.
    pub fn save(&self, e: &Endpoint) -> Result<(), EndpointError> {
        if !e.is_valid() {
            return Err(EndpointError::Invalid);
        }
        let json = Zeroizing::new(serde_json::to_vec(e).map_err(|_| EndpointError::Invalid)?);
        self.secrets
            .set(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT, &json)
            .map_err(|err| EndpointError::Store(err.to_string()))
    }

    /// Reads the record; `Ok(None)` when none exists.
    ///
    /// # Errors
    /// `Invalid` for oversized, malformed or out-of-range content, `Store` when the store fails.
    pub fn load(&self) -> Result<Option<Endpoint>, EndpointError> {
        let raw = self
            .secrets
            .get(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT)
            .map_err(|err| EndpointError::Store(err.to_string()))?;
        let Some(raw) = raw else { return Ok(None) };
        let raw = Zeroizing::new(raw);
        if raw.len() > MAX_RECORD_BYTES {
            return Err(EndpointError::Invalid);
        }
        // The serde error text can quote values, so it is dropped.
        let e: Endpoint = serde_json::from_slice(&raw).map_err(|_| EndpointError::Invalid)?;
        if e.is_valid() {
            Ok(Some(e))
        } else {
            Err(EndpointError::Invalid)
        }
    }

    /// Removes the record; a missing record is not an error.
    ///
    /// # Errors
    /// `Store` when the store fails.
    pub fn clear(&self) -> Result<(), EndpointError> {
        self.secrets
            .delete(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT)
            .map_err(|err| EndpointError::Store(err.to_string()))
    }
}

/// The two handshake frames, sent before any request.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Handshake {
    /// Host to app: the challenge.
    #[serde(rename = "hello")]
    Hello {
        /// Fresh random nonce, base64.
        nonce: String,
    },
    /// App to host: the answer.
    #[serde(rename = "helloAck")]
    HelloAck {
        /// Lowercase hex HMAC-SHA256 proof.
        proof: String,
    },
}

impl std::fmt::Debug for Handshake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hello { .. } => f.write_str("Hello"),
            Self::HelloAck { .. } => f.write_str("HelloAck"),
        }
    }
}

/// `HMAC-SHA256(key = token, msg = "mypass-bridge-hello-v1" || nonce)` as lowercase hex.
#[must_use]
pub fn hello_proof(token: &str, nonce: &str) -> String {
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(token.as_bytes()) else {
        // HMAC takes any key length, so this is unreachable; an empty proof never verifies.
        return String::new();
    };
    mac.update(PROOF_DOMAIN);
    mac.update(nonce.as_bytes());
    let tag = mac.finalize().into_bytes();
    let mut out = String::with_capacity(tag.len() * 2);
    for b in tag {
        out.push(hex_digit(b >> 4));
        out.push(hex_digit(b & 0x0f));
    }
    out
}

fn hex_digit(n: u8) -> char {
    char::from_digit(u32::from(n), 16).unwrap_or('0')
}

/// Constant-time check of a peer's proof for this token and nonce.
#[must_use]
pub fn verify_proof(token: &str, nonce: &str, proof: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let expected = hello_proof(token, nonce);
    !expected.is_empty() && tokens_equal(&expected, proof)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use mypass_platform::testing::InMemorySecretStore;

    fn store() -> (Arc<InMemorySecretStore>, EndpointStore) {
        let mem = InMemorySecretStore::new();
        (mem.clone(), EndpointStore::new(mem))
    }

    fn ep(port: u16, token: &str) -> Endpoint {
        Endpoint {
            port,
            token: token.into(),
            pid: 7,
            started_at: 99,
        }
    }

    #[test]
    fn store_starts_empty() {
        assert_eq!(store().1.load().unwrap(), None);
    }

    #[test]
    fn save_then_load_round_trips() {
        let (_, s) = store();
        let e = ep(5000, &generate_token());
        s.save(&e).unwrap();
        assert_eq!(s.load().unwrap(), Some(e));
    }

    #[test]
    fn clear_removes() {
        let (_, s) = store();
        s.save(&ep(1, "t")).unwrap();
        s.clear().unwrap();
        assert_eq!(s.load().unwrap(), None);
        s.clear().unwrap();
    }

    #[test]
    fn overwrite_replaces_previous() {
        let (_, s) = store();
        s.save(&ep(1, "a")).unwrap();
        s.save(&ep(2, "b")).unwrap();
        assert_eq!(s.load().unwrap(), Some(ep(2, "b")));
    }

    #[test]
    fn token_is_44_char_base64_of_32_bytes_and_distinct_each_call() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 44);
        assert_eq!(B64.decode(&a).unwrap().len(), 32);
        assert_ne!(a, b);
        assert_eq!(B64.decode(new_nonce()).unwrap().len(), 16);
    }

    #[test]
    fn old_two_field_record_still_decodes_with_zero_pid() {
        let (mem, s) = store();
        mem.set(
            ENDPOINT_SERVICE,
            ENDPOINT_ACCOUNT,
            br#"{"port":4242,"token":"abc"}"#,
        )
        .unwrap();
        let e = s.load().unwrap().unwrap();
        assert_eq!((e.port, e.pid, e.started_at), (4242, 0, 0));
    }

    #[test]
    fn json_shape_is_camel_case() {
        let v = serde_json::to_value(ep(1, "t")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"port":1,"token":"t","pid":7,"startedAt":99})
        );
    }

    #[test]
    fn endpoint_debug_redacts_token() {
        let t = generate_token();
        let dbg = format!("{:?}", ep(1, &t));
        assert!(!dbg.contains(&t));
        assert!(dbg.contains("redacted"));
        let hs = format!("{:?}", Handshake::HelloAck { proof: "p".into() });
        assert_eq!(hs, "HelloAck");
    }

    #[test]
    fn hostile_records_are_rejected_without_echoing_content() {
        let (mem, s) = store();
        let cases: Vec<Vec<u8>> = vec![
            b"not json SECRETWORD".to_vec(),
            br#"{"port":0,"token":"SECRETWORD"}"#.to_vec(),
            br#"{"port":70000,"token":"SECRETWORD"}"#.to_vec(),
            br#"{"port":1,"token":""}"#.to_vec(),
            br#"{"port":1,"token":123}"#.to_vec(),
            br#"[1,2]"#.to_vec(),
            format!(r#"{{"port":1,"token":"{}"}}"#, "a".repeat(5000)).into_bytes(),
            format!(r#"{{"port":1,"token":"{}"}}"#, "a".repeat(300)).into_bytes(),
            vec![0xff, 0xfe, 0x00],
            Vec::new(),
        ];
        for raw in cases {
            mem.set(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT, &raw).unwrap();
            let err = s.load().unwrap_err();
            assert_eq!(err, EndpointError::Invalid);
            assert!(!err.to_string().contains("SECRETWORD"));
        }
    }

    #[test]
    fn save_rejects_unreadable_records() {
        let (_, s) = store();
        assert_eq!(s.save(&ep(0, "t")), Err(EndpointError::Invalid));
        assert_eq!(s.save(&ep(1, "")), Err(EndpointError::Invalid));
    }

    #[test]
    fn tokens_equal_cases() {
        assert!(tokens_equal("abcdef", "abcdef"));
        assert!(!tokens_equal("abcdef", "abcdeg"));
        assert!(!tokens_equal("abcdef", "abcde"));
        assert!(!tokens_equal("", ""));
        assert!(!tokens_equal("a", ""));
    }

    #[test]
    fn hello_proof_is_deterministic_hex_64_chars() {
        let p = hello_proof("tok", "nonce");
        assert_eq!(p, hello_proof("tok", "nonce"));
        assert_eq!(p.len(), 64);
        assert!(p.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(b"tok").unwrap();
        mac.update(b"mypass-bridge-hello-v1");
        mac.update(b"nonce");
        let want: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(p, want);
    }

    #[test]
    fn verify_proof_rejects_wrong_token_nonce_or_proof() {
        let p = hello_proof("tok", "n1");
        assert!(verify_proof("tok", "n1", &p));
        assert!(!verify_proof("tok2", "n1", &p));
        assert!(!verify_proof("tok", "n2", &p));
        assert!(!verify_proof("tok", "n1", &p.to_uppercase()));
        assert!(!verify_proof("tok", "n1", ""));
        assert!(!verify_proof("", "n1", &hello_proof("", "n1")));
        // A squatter that echoes the token or a guess does not pass.
        assert!(!verify_proof("tok", "n1", "tok"));
    }

    #[test]
    fn handshake_json_shapes() {
        let h = serde_json::to_string(&Handshake::Hello { nonce: "N".into() }).unwrap();
        assert_eq!(h, r#"{"type":"hello","nonce":"N"}"#);
        let a = serde_json::to_string(&Handshake::HelloAck { proof: "P".into() }).unwrap();
        assert_eq!(a, r#"{"type":"helloAck","proof":"P"}"#);
        let back: Handshake = serde_json::from_str(&a).unwrap();
        assert_eq!(back, Handshake::HelloAck { proof: "P".into() });
    }
}
