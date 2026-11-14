// Test-only support code: synthetic passkey vectors for the independent oracle
// (`tests/crosscompat/passkey_oracle.py`), never an owner's vault.
//
// Usage: write_passkey_vectors <out_dir>   (the directory must exist and be empty of our files)
//
// It writes `vectors.json` (requests, origin, our responses, the stored fields) and `vault.kdbx`
// (one entry per case, written through `PasskeyRecord::write`, fast KDF). The oracle verifies this
// output with python-fido2, pykeepass and `cryptography`; it never produces our known-answer
// values (those are pinned in `tests/passkey_vectors.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::Write;
use std::path::Path;

use mypass_core::passkey::{
    b64url, parse_create, parse_get, perform_create, perform_create_with, perform_get, BackupFlags,
    PasskeyRecord, PasskeySigningKey,
};
use mypass_kdbx::{Database, Entry, KdfParams, Timestamp};
use p256::pkcs8::DecodePrivateKey;
use p256::SecretKey;
use secrecy::SecretString;
use serde_json::{json, Value};

const ORIGIN: &str = "https://example.com";
const PASSWORD: &str = "oracle-fixture-password";
const PKCS8: &str = include_str!("../tests/fixtures/passkey/rfc6979_a25_pkcs8.pem");
const CREATE_REQUEST: &str = include_str!("../tests/fixtures/passkey/chrome_create_request.json");
const GET_REQUEST: &str = include_str!("../tests/fixtures/passkey/chrome_get_request.json");
const CREATED: Timestamp = Timestamp(1_700_000_000);

struct Case {
    name: &'static str,
    create_request: String,
    create_response: Value,
    credential_id: Vec<u8>,
    pem: String,
    user_handle: Vec<u8>,
    username: String,
    assertions: Vec<(String, Value)>,
}

fn with(request: &str, edit: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(request).unwrap();
    edit(&mut value);
    value.to_string()
}

fn assertions(
    key_pem: &str,
    credential_id: &[u8],
    user_handle: &[u8],
    flags: BackupFlags,
) -> Vec<(String, Value)> {
    let key = PasskeySigningKey::from_pem(key_pem).unwrap();
    // The first assertion names the credential, the second is a discoverable get (no list) with a
    // different challenge.
    let named = GET_REQUEST.to_owned();
    let named_json = with(&named, |v| {
        v["allowCredentials"] =
            json!([{ "type": "public-key", "id": b64url::encode(credential_id) }]);
    });
    let discoverable = with(GET_REQUEST, |v| {
        v["allowCredentials"] = json!([]);
        v["challenge"] = json!(b64url::encode(&[0x5au8; 32]));
    });
    [named_json, discoverable]
        .into_iter()
        .map(|request| {
            let validated = parse_get(ORIGIN, &request).unwrap();
            let outcome =
                perform_get(&validated, &key, credential_id, Some(user_handle), flags).unwrap();
            (
                request,
                serde_json::from_str(&outcome.response_json).unwrap(),
            )
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args_os()
        .nth(1)
        .ok_or("usage: write_passkey_vectors <out_dir>")?;
    let dir = Path::new(&dir);

    let user_handle = b64url::decode("oKGio6SlpqeoqaqrrK2urw").ok_or("user handle")?;
    let mut cases = Vec::new();

    // 1. Fixed key and credential id (RFC 6979 A.2.5), attestation "none".
    let fixed_id: [u8; 32] = core::array::from_fn(|i| i as u8);
    let create = parse_create(ORIGIN, CREATE_REQUEST)?;
    let secret = SecretKey::from_pkcs8_pem(PKCS8)?;
    let fixed = perform_create_with(&create, secret, fixed_id)?;
    cases.push(Case {
        name: "fixed-none",
        create_request: CREATE_REQUEST.to_owned(),
        create_response: serde_json::from_str(&fixed.response_json)?,
        credential_id: fixed.credential_id.clone(),
        pem: fixed.private_key_pem.to_string(),
        user_handle: user_handle.clone(),
        username: "alice@example.com".into(),
        assertions: assertions(
            &fixed.private_key_pem,
            &fixed.credential_id,
            &user_handle,
            BackupFlags::default(),
        ),
    });

    // 2. Fixed key, "direct" attestation: packed self-attestation, with BE=1 BS=0 on assertions.
    let direct_request = with(CREATE_REQUEST, |v| v["attestation"] = json!("direct"));
    let create = parse_create(ORIGIN, &direct_request)?;
    let secret = SecretKey::from_pkcs8_pem(PKCS8)?;
    let packed = perform_create_with(&create, secret, fixed_id)?;
    cases.push(Case {
        name: "fixed-packed",
        create_request: direct_request,
        create_response: serde_json::from_str(&packed.response_json)?,
        credential_id: packed.credential_id.clone(),
        pem: packed.private_key_pem.to_string(),
        user_handle: user_handle.clone(),
        username: "alice@example.com".into(),
        assertions: assertions(
            &packed.private_key_pem,
            &packed.credential_id,
            &user_handle,
            BackupFlags {
                eligible: true,
                state: false,
            },
        ),
    });

    // 3. One random run: fresh key and credential id from the OS RNG.
    let create = parse_create(ORIGIN, CREATE_REQUEST)?;
    let random = perform_create(&create)?;
    cases.push(Case {
        name: "random-none",
        create_request: CREATE_REQUEST.to_owned(),
        create_response: serde_json::from_str(&random.response_json)?,
        credential_id: random.credential_id.clone(),
        pem: random.private_key_pem.to_string(),
        user_handle: user_handle.clone(),
        username: "alice@example.com".into(),
        assertions: assertions(
            &random.private_key_pem,
            &random.credential_id,
            &user_handle,
            BackupFlags::default(),
        ),
    });

    // The vault: one entry per case, through the production writer, with a fast KDF.
    let mut db = Database::new("Passkey oracle fixture");
    if let KdfParams::Argon2 {
        memory_bytes,
        iterations,
        parallelism,
        ..
    } = &mut db.settings.kdf
    {
        *memory_bytes = 8 * 1024 * 1024;
        *iterations = 1;
        *parallelism = 1;
    }
    for case in &cases {
        let mut entry = Entry::new(CREATED);
        entry.set_field("Title", case.name, false);
        entry.set_field("UserName", &case.username, false);
        entry.set_field("URL", ORIGIN, false);
        PasskeyRecord {
            rp_id: "example.com".into(),
            credential_id: case.credential_id.clone(),
            private_key_pem: case.pem.clone().into(),
            user_handle: Some(case.user_handle.clone()),
            username: Some(case.username.clone()),
            created: Some(CREATED),
            flags: BackupFlags::default(),
        }
        .write(&mut entry);
        db.root.entries.push(entry);
    }
    let bytes = db.save(&SecretString::from(PASSWORD))?;
    let vault = dir.join("vault.kdbx");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&vault)?
        .write_all(&bytes)?;

    let vectors = json!({
        "origin": ORIGIN,
        "rp_id": "example.com",
        "vault": "vault.kdbx",
        "vault_password": PASSWORD,
        "created": "2023-11-14T22:13:20Z",
        "cases": cases.iter().map(|c| json!({
            "name": c.name,
            "create_request": c.create_request,
            "create_response": c.create_response,
            "credential_id": b64url::encode(&c.credential_id),
            "private_key_pem": c.pem,
            "user_handle": b64url::encode(&c.user_handle),
            "username": c.username,
            "assertions": c.assertions.iter().map(|(req, resp)| json!({
                "request": req,
                "response": resp,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("vectors.json"))?
        .write_all(serde_json::to_string_pretty(&vectors)?.as_bytes())?;
    Ok(())
}
