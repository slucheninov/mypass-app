//! The authenticator side of a WebAuthn ceremony: authenticator data, the attestation object and
//! the ES256 signatures for `create` and `get`, as a software authenticator. Ports
//! `PasskeyCeremony.performCreate` / `performGet` of the Swift app.
//!
//! Every signature is ECDSA P-256 with SHA-256 over `authenticatorData || SHA-256(clientDataJSON)`,
//! DER-encoded, with a deterministic RFC 6979 nonce (no RNG at signing time). The relying party id
//! that goes into `rpIdHash` is only ever the canonical rpId authorised for the validated origin
//! ([`ValidatedCreate::rp_id`] / [`ValidatedGet::rp_id`]), and `clientDataJSON.origin` is only ever
//! that validated origin. Nothing here takes a bare origin or rpId string:
//!
//! ```
//! use mypass_core::passkey::{parse_create, perform_create};
//! let json = r#"{"challenge":"AAECAwQFBgcICQoLDA0ODw","rp":{"id":"example.com"},
//!     "user":{"id":"AQ","name":"a"},
//!     "extensions":{"remoteDesktopClientOverride":
//!         {"origin":"https://example.com","sameOriginWithAncestors":true}}}"#;
//! let request = parse_create("https://example.com", json).unwrap();
//! let outcome = perform_create(&request).unwrap();
//! assert_eq!(outcome.credential_id.len(), 32);
//! ```
//!
//! ```compile_fail,E0308
//! use mypass_core::passkey::perform_create;
//! let outcome = perform_create("https://example.com");
//! ```
//!
//! The signing key is neither `Clone` nor `Debug`, so it cannot be copied around or printed:
//!
//! ```
//! use mypass_core::passkey::PasskeySigningKey;
//! fn take(key: PasskeySigningKey) -> PasskeySigningKey { key }
//! fn show(key: &PasskeySigningKey) -> String { format!("{}", key.public_key_spki_der().is_ok()) }
//! ```
//!
//! ```compile_fail,E0599
//! use mypass_core::passkey::PasskeySigningKey;
//! fn take(key: PasskeySigningKey) -> PasskeySigningKey { key.clone() }
//! ```
//!
//! ```compile_fail,E0277
//! use mypass_core::passkey::PasskeySigningKey;
//! fn show(key: &PasskeySigningKey) -> String { format!("{key:?}") }
//! ```
//!
//! **User presence and verification.** Both UP and UV are always set. MyPass is a software
//! authenticator: "user verified" means the vault is unlocked and the user clicked to confirm in
//! the extension popup (the owner's decision for Stage 4, the same as the Swift app). That is also
//! the answer to `userVerification: "required"`. There is no per-ceremony re-authentication.
//!
//! **Sign counter.** Always 0. No counter is persisted, so copies of a vault synced between
//! devices never trip a relying party's clone detection (WebAuthn allows a zero counter).
//!
//! **Client extensions.** `clientExtensionResults` is always `{}`. In particular `credProps` is
//! not answered: it is optional, and a relying party that asked for it treats a missing result as
//! "unknown" (every MyPass passkey is discoverable anyway).

use super::{b64url, cbor, PasskeyError, ValidatedCreate, ValidatedGet, AAGUID};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{DerSignature, SigningKey};
use p256::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use p256::SecretKey;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::Zeroizing;

/// Authenticator data flag: user present.
pub const FLAG_UP: u8 = 0x01;
/// Authenticator data flag: user verified.
pub const FLAG_UV: u8 = 0x04;
/// Authenticator data flag: backup eligible.
pub const FLAG_BE: u8 = 0x08;
/// Authenticator data flag: backup state (backed up).
pub const FLAG_BS: u8 = 0x10;
/// Authenticator data flag: attested credential data included (create only).
pub const FLAG_AT: u8 = 0x40;

/// COSE algorithm identifier of ES256, encoded as the CBOR magnitude of `-7`.
const COSE_ES256_MAGNITUDE: u64 = 6;
/// The ES256 COSE identifier as it appears in the JSON response.
const COSE_ES256: i64 = -7;
/// Length of every credential id MyPass creates.
pub const CREDENTIAL_ID_LEN: usize = 32;
/// How many fresh 32-byte draws key generation tries before giving up. A uniformly random draw
/// is outside the P-256 scalar range with probability about 2^-32, so one try practically always
/// succeeds; the bound only keeps a broken RNG from looping forever.
const KEY_DRAW_ATTEMPTS: usize = 8;

/// The backup flags of a credential (`BE` and `BS` in the authenticator data).
///
/// MyPass passkeys live in a vault file the user syncs, so they are created backup eligible and
/// backed up (`1/1`). For an assertion the flags come from the stored `KPEX_PASSKEY_FLAG_BE/BS`
/// (default `1/1` when absent), so a credential another tool created with `BE=0` does not claim
/// otherwise to its relying party. `BS` without `BE` is not a valid state (WebAuthn), so
/// [`BackupFlags::bits`] drops `BS` whenever `BE` is clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupFlags {
    /// Backup eligible (`BE`).
    pub eligible: bool,
    /// Backup state (`BS`); only reported when `eligible` is set.
    pub state: bool,
}

impl Default for BackupFlags {
    fn default() -> Self {
        Self {
            eligible: true,
            state: true,
        }
    }
}

impl BackupFlags {
    /// The `BE` / `BS` bits of the authenticator data flags byte; `BS` requires `BE`.
    #[must_use]
    pub fn bits(self) -> u8 {
        match (self.eligible, self.state) {
            (true, true) => FLAG_BE | FLAG_BS,
            (true, false) => FLAG_BE,
            (false, _) => 0,
        }
    }
}

/// The result of a create ceremony: the response for the browser and the material to store.
pub struct CreateOutcome {
    /// The `PublicKeyCredential` JSON handed back to Chrome.
    pub response_json: String,
    /// The new credential id (32 random bytes).
    pub credential_id: Vec<u8>,
    /// The private key as PKCS#8 PEM with LF line endings, for `KPEX_PASSKEY_PRIVATE_KEY_PEM`.
    pub private_key_pem: Zeroizing<String>,
    /// The authenticator data that was signed (also inside the attestation object).
    pub auth_data: Vec<u8>,
    /// The DER signature over `auth_data || SHA-256(clientDataJSON)`.
    pub signature: Vec<u8>,
}

impl fmt::Debug for CreateOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateOutcome")
            .field("response_json_len", &self.response_json.len())
            .field("credential_id_len", &self.credential_id.len())
            .field("private_key_pem", &"<redacted>")
            .field("auth_data_len", &self.auth_data.len())
            .field("signature_len", &self.signature.len())
            .finish()
    }
}

/// The result of a get ceremony. It holds no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetOutcome {
    /// The `PublicKeyCredential` JSON handed back to Chrome.
    pub response_json: String,
    /// The authenticator data that was signed.
    pub auth_data: Vec<u8>,
    /// The DER signature over `auth_data || SHA-256(clientDataJSON)`.
    pub signature: Vec<u8>,
}

/// A P-256 signing key loaded from a stored PEM. The key material is zeroized on drop (by the
/// wrapped `SigningKey`); the type is deliberately neither `Clone` nor `Debug`.
pub struct PasskeySigningKey(SigningKey);

impl PasskeySigningKey {
    /// Load a P-256 private key from PEM: PKCS#8 (`BEGIN PRIVATE KEY`, what MyPass and KeePassXC
    /// write) or SEC1 (`BEGIN EC PRIVATE KEY`). Surrounding whitespace and CRLF line endings are
    /// tolerated.
    ///
    /// # Errors
    /// `Crypto` for anything that is not a P-256 private key PEM (garbage, another curve, a
    /// public key). The error carries no part of the input.
    pub fn from_pem(pem: &str) -> Result<Self, PasskeyError> {
        let normalised = Zeroizing::new(pem.trim().replace("\r\n", "\n"));
        let secret = SecretKey::from_pem(&normalised).map_err(|_| PasskeyError::Crypto)?;
        Ok(Self(SigningKey::from(&secret)))
    }

    /// The public key as SubjectPublicKeyInfo DER (what the create response calls `publicKey`).
    ///
    /// # Errors
    /// `Crypto` if the encoder fails, which it does not for a valid key.
    pub fn public_key_spki_der(&self) -> Result<Vec<u8>, PasskeyError> {
        public_key_spki_der(&p256::PublicKey::from(self.0.verifying_key()))
    }

    fn sign(&self, auth_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
        sign(&self.0, auth_data, client_data_json)
    }
}

/// `clientDataJSON` in the WebAuthn-recommended member order `type, challenge, origin,
/// crossOrigin`, with no escaping of `/`. The Swift app used `JSONSerialization`, which escaped
/// slashes (`https:\/\/x`): valid JSON, but not the canonical form some relying party libraries
/// byte-compare against. Strings are escaped by `serde_json`, which never escapes `/`. It
/// writes control characters as short escapes (`\n`) where WebAuthn's `CCDToString` wants
/// `\u000a`; no value that reaches this function can hold one (the type is fixed, the challenge
/// is base64url, the origin an ASCII serialisation), so the output is canonical for its inputs.
#[must_use]
pub fn client_data_json(kind: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
    let quoted = |s: &str| Value::String(s.to_owned()).to_string();
    format!(
        "{{\"type\":{},\"challenge\":{},\"origin\":{},\"crossOrigin\":false}}",
        quoted(kind),
        quoted(&b64url::encode(challenge)),
        quoted(origin)
    )
    .into_bytes()
}

/// Run a create ceremony with a fresh key and a fresh 32-byte credential id, both from the OS RNG.
///
/// # Errors
/// `Crypto` when the OS RNG fails or a key cannot be encoded.
pub fn perform_create(request: &ValidatedCreate) -> Result<CreateOutcome, PasskeyError> {
    let key = random_secret_key()?;
    let mut credential_id = [0u8; CREDENTIAL_ID_LEN];
    getrandom::fill(&mut credential_id).map_err(|_| PasskeyError::Crypto)?;
    create_with(request, key, credential_id)
}

/// [`perform_create`] with a caller-chosen key and credential id: the deterministic seam for
/// known-answer vectors (signatures are RFC 6979, so the whole outcome is a pure function of the
/// inputs). Behind the `test-util` feature: production code calls [`perform_create`] and can
/// never choose the key.
///
/// # Errors
/// `Crypto` when a key cannot be encoded.
#[cfg(feature = "test-util")]
pub fn perform_create_with(
    request: &ValidatedCreate,
    key: SecretKey,
    credential_id: [u8; CREDENTIAL_ID_LEN],
) -> Result<CreateOutcome, PasskeyError> {
    create_with(request, key, credential_id)
}

fn create_with(
    request: &ValidatedCreate,
    key: SecretKey,
    credential_id: [u8; CREDENTIAL_ID_LEN],
) -> Result<CreateOutcome, PasskeyError> {
    let signing = SigningKey::from(&key);
    let public = key.public_key();
    let point = public.to_sec1_bytes();
    // Uncompressed SEC1 point: 0x04 || X(32) || Y(32).
    let (x, y) = match (point.first(), point.get(1..33), point.get(33..65)) {
        (Some(0x04), Some(x), Some(y)) if point.len() == 65 => (x, y),
        _ => return Err(PasskeyError::Crypto),
    };

    let mut attested = Vec::with_capacity(16 + 2 + CREDENTIAL_ID_LEN + 77);
    attested.extend_from_slice(&AAGUID);
    attested.extend_from_slice(&credential_id_length(&credential_id)?);
    attested.extend_from_slice(&credential_id);
    attested.extend(cose_ec2_public_key(x, y));

    let flags = FLAG_UP | FLAG_UV | BackupFlags::default().bits() | FLAG_AT;
    let auth_data = authenticator_data(request.rp_id(), flags, Some(&attested));
    let client_data = client_data_json(
        "webauthn.create",
        &request.request().challenge,
        request.origin().as_str(),
    );
    let signature = sign(&signing, &auth_data, &client_data);
    let attestation_object =
        attestation_object(&auth_data, &signature, &request.request().attestation);
    let public_key_der = public_key_spki_der(&public)?;

    let credential = b64url::encode(&credential_id);
    let response = json!({
        "id": credential,
        "rawId": credential,
        "type": "public-key",
        "response": {
            "clientDataJSON": b64url::encode(&client_data),
            "attestationObject": b64url::encode(&attestation_object),
            // Chrome validates these WebAuthn JSON fields against the CBOR object; omitting them
            // rejects completion even after the vault saved the key.
            "publicKeyAlgorithm": COSE_ES256,
            "authenticatorData": b64url::encode(&auth_data),
            "publicKey": b64url::encode(&public_key_der),
            "transports": ["internal"],
        },
        "clientExtensionResults": {},
    });
    let private_key_pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|_| PasskeyError::Crypto)?;

    Ok(CreateOutcome {
        response_json: response.to_string(),
        credential_id: credential_id.to_vec(),
        private_key_pem,
        auth_data,
        signature,
    })
}

/// Run a get ceremony: sign an assertion with a stored key for a stored credential.
///
/// When the request named credentials (`allowCredentials`), `credential_id` must be one of them;
/// the caller resolves the entry, this is a last check that nothing signs for a credential the
/// relying party did not ask for. `userHandle` is omitted from the response when `None`.
///
/// # Errors
/// `InvalidRequest` for an empty credential id; `CredentialNotAllowed` when `allowCredentials` is
/// non-empty and does not contain `credential_id`.
pub fn perform_get(
    request: &ValidatedGet,
    key: &PasskeySigningKey,
    credential_id: &[u8],
    user_handle: Option<&[u8]>,
    flags: BackupFlags,
) -> Result<GetOutcome, PasskeyError> {
    if credential_id.is_empty() {
        return Err(PasskeyError::InvalidRequest);
    }
    let allow = &request.request().allow_credential_ids;
    if !allow.is_empty() && !allow.iter().any(|id| id.as_slice() == credential_id) {
        return Err(PasskeyError::CredentialNotAllowed);
    }

    let auth_data = authenticator_data(request.rp_id(), FLAG_UP | FLAG_UV | flags.bits(), None);
    let client_data = client_data_json(
        "webauthn.get",
        &request.request().challenge,
        request.origin().as_str(),
    );
    let signature = key.sign(&auth_data, &client_data);

    let mut inner = Map::new();
    inner.insert(
        "clientDataJSON".into(),
        Value::String(b64url::encode(&client_data)),
    );
    inner.insert(
        "authenticatorData".into(),
        Value::String(b64url::encode(&auth_data)),
    );
    inner.insert(
        "signature".into(),
        Value::String(b64url::encode(&signature)),
    );
    if let Some(handle) = user_handle {
        inner.insert("userHandle".into(), Value::String(b64url::encode(handle)));
    }
    let credential = b64url::encode(credential_id);
    let response = json!({
        "id": credential,
        "rawId": credential,
        "type": "public-key",
        "response": Value::Object(inner),
        "clientExtensionResults": {},
    });
    Ok(GetOutcome {
        response_json: response.to_string(),
        auth_data,
        signature,
    })
}

/// A uniformly random P-256 secret key from the OS RNG. The draw is zeroized; an out-of-range
/// draw (zero or not below the group order) is redrawn.
fn random_secret_key() -> Result<SecretKey, PasskeyError> {
    for _ in 0..KEY_DRAW_ATTEMPTS {
        let mut draw = Zeroizing::new([0u8; 32]);
        getrandom::fill(draw.as_mut_slice()).map_err(|_| PasskeyError::Crypto)?;
        if let Ok(key) = SecretKey::from_slice(draw.as_slice()) {
            return Ok(key);
        }
    }
    Err(PasskeyError::Crypto)
}

fn public_key_spki_der(public: &p256::PublicKey) -> Result<Vec<u8>, PasskeyError> {
    public
        .to_public_key_der()
        .map(|doc| doc.as_bytes().to_vec())
        .map_err(|_| PasskeyError::Crypto)
}

/// ECDSA P-256 / SHA-256 (RFC 6979 nonce) over `auth_data || SHA-256(client_data_json)`, DER.
fn sign(key: &SigningKey, auth_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(auth_data.len() + 32);
    message.extend_from_slice(auth_data);
    message.extend_from_slice(&Sha256::digest(client_data_json));
    // p256 does not normalise S, so about half of the signatures are high-S. That is
    // deliberate: P-256 WebAuthn verifiers do not require low-S (the Swift app's CryptoKit
    // signatures were the same), and normalising would change the pinned RFC 6979 vector.
    let signature: DerSignature = key.sign(&message);
    signature.as_bytes().to_vec()
}

/// `SHA-256(rpId) || flags || signCount (u32 BE, always 0) || attestedCredentialData?`.
fn authenticator_data(rp_id: &str, flags: u8, attested: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(37 + attested.map_or(0, <[u8]>::len));
    out.extend_from_slice(&Sha256::digest(rp_id.as_bytes()));
    out.push(flags);
    out.extend_from_slice(&0u32.to_be_bytes());
    if let Some(attested) = attested {
        out.extend_from_slice(attested);
    }
    out
}

fn credential_id_length(credential_id: &[u8]) -> Result<[u8; 2], PasskeyError> {
    u16::try_from(credential_id.len())
        .map(u16::to_be_bytes)
        .map_err(|_| PasskeyError::Crypto)
}

/// COSE_Key for an EC2 P-256 public key, map keys in canonical CTAP2 order: unsigned labels
/// ascending (1, 3), then negative labels ascending by magnitude (-1, -2, -3).
fn cose_ec2_public_key(x: &[u8], y: &[u8]) -> Vec<u8> {
    let mut out = cbor::map_header(5);
    out.extend(cbor::uint(1)); // kty
    out.extend(cbor::uint(2)); // EC2
    out.extend(cbor::uint(3)); // alg
    out.extend(cbor::nint(COSE_ES256_MAGNITUDE)); // ES256 (-7)
    out.extend(cbor::nint(0)); // crv (-1)
    out.extend(cbor::uint(1)); // P-256
    out.extend(cbor::nint(1)); // x (-2)
    out.extend(cbor::bytes(x));
    out.extend(cbor::nint(2)); // y (-3)
    out.extend(cbor::bytes(y));
    out
}

/// The attestation object, keys in canonical CTAP2 order (text keys by length, then bytewise):
/// `fmt`, `attStmt`, `authData`.
///
/// `direct`, `indirect` and `enterprise` get `packed` self-attestation (`alg` -7 and the
/// credential key's signature, no `x5c`). Everything else, `none` included, gets `none` with an
/// empty statement: WebAuthn says unknown conveyance values are ignored, so this deliberately
/// departs from the Swift app, which mapped every non-`none` value to packed.
fn attestation_object(auth_data: &[u8], signature: &[u8], attestation: &str) -> Vec<u8> {
    let mut out = cbor::map_header(3);
    out.extend(cbor::text("fmt"));
    if matches!(attestation, "direct" | "indirect" | "enterprise") {
        out.extend(cbor::text("packed"));
        out.extend(cbor::text("attStmt"));
        out.extend(cbor::map_header(2));
        out.extend(cbor::text("alg"));
        out.extend(cbor::nint(COSE_ES256_MAGNITUDE));
        out.extend(cbor::text("sig"));
        out.extend(cbor::bytes(signature));
    } else {
        out.extend(cbor::text("none"));
        out.extend(cbor::text("attStmt"));
        out.extend(cbor::map_header(0));
    }
    out.extend(cbor::text("authData"));
    out.extend(cbor::bytes(auth_data));
    out
}

#[cfg(test)]
mod tests;
