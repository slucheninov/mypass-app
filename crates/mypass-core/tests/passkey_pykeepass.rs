//! Reverse direction of the passkey oracle: a vault written by pykeepass with KeePassXC-style
//! passkey entries (`tests/crosscompat/passkey_oracle.py`) must read back through
//! `PasskeyRecord` and sign an assertion that verifies against the independent public key.
//!
//! Env-gated like `mypass-kdbx`'s `pykeepass_generated`: without `MYPASS_PYKEEPASS_PASSKEY_DIR`
//! it prints a skip and passes; CI always sets it.

// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use mypass_core::passkey::{
    b64url, has_passkey, parse_get, perform_get, BackupFlags, PasskeyRecord, PasskeySigningKey,
};
use mypass_kdbx::Database;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{DerSignature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use secrecy::SecretString;
use serde_json::Value;
use sha2::{Digest, Sha256};

const GET_REQUEST: &str = include_str!("fixtures/passkey/chrome_get_request.json");

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn pykeepass_written_keepassxc_style_passkeys_read_and_sign() {
    let Some(dir) = std::env::var_os("MYPASS_PYKEEPASS_PASSKEY_DIR") else {
        eprintln!("skipped: set MYPASS_PYKEEPASS_PASSKEY_DIR (see crosscompat/passkey_oracle.py)");
        return;
    };
    let dir = std::path::Path::new(&dir);
    let expected: Value =
        serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();
    let password = SecretString::from(expected["password"].as_str().unwrap());
    let db = Database::open(&std::fs::read(dir.join("vault.kdbx")).unwrap(), &password).unwrap();
    let entries = expected["entries"].as_array().unwrap();
    assert!(
        entries.len() >= 3,
        "expected the fixture and the generated entries"
    );

    for want in entries {
        let title = want["title"].as_str().unwrap();
        let entry = db
            .root
            .entries
            .iter()
            .find(|e| e.field("Title") == Some(title))
            .unwrap_or_else(|| panic!("entry {title} missing"));
        assert!(has_passkey(entry), "{title}: not a passkey");
        let record = PasskeyRecord::read(entry).unwrap();
        assert_eq!(record.rp_id, want["rp_id"].as_str().unwrap(), "{title}");
        assert_eq!(
            record.credential_id,
            unhex(want["credential_id"].as_str().unwrap()),
            "{title}"
        );
        assert_eq!(
            record.user_handle.as_deref(),
            Some(unhex(want["user_handle"].as_str().unwrap()).as_slice()),
            "{title}"
        );
        assert_eq!(record.created, None, "{title}: KeePassXC writes no CREATED");
        assert_eq!(
            record.flags,
            BackupFlags::default(),
            "{title}: absent flags read as 1/1"
        );

        // The stored key, written by another implementation, signs an assertion that verifies
        // against the independently derived public key.
        let origin = format!("https://{}", record.rp_id);
        // A discoverable get (no `allowCredentials`) for this entry's relying party.
        let mut request: Value = serde_json::from_str(GET_REQUEST).unwrap();
        request["rpId"] = Value::String(record.rp_id.clone());
        request["allowCredentials"] = Value::Array(vec![]);
        request["extensions"]["remoteDesktopClientOverride"]["origin"] =
            Value::String(origin.clone());
        let validated = parse_get(&origin, &request.to_string()).unwrap();
        let key = PasskeySigningKey::from_pem(&record.private_key_pem).unwrap();
        let spki = b64url::decode(want["public_key_spki"].as_str().unwrap()).unwrap();
        assert_eq!(
            key.public_key_spki_der().unwrap(),
            spki,
            "{title}: public key"
        );
        let outcome = perform_get(
            &validated,
            &key,
            &record.credential_id,
            record.user_handle.as_deref(),
            record.flags,
        )
        .unwrap();
        let response: Value = serde_json::from_str(&outcome.response_json).unwrap();
        let client_data =
            b64url::decode(response["response"]["clientDataJSON"].as_str().unwrap()).unwrap();
        let mut signed = outcome.auth_data.clone();
        signed.extend_from_slice(&Sha256::digest(&client_data));
        let verifying = VerifyingKey::from_public_key_der(&spki).unwrap();
        let signature = DerSignature::from_bytes(&outcome.signature).unwrap();
        verifying.verify(&signed, &signature).unwrap();
    }
}
