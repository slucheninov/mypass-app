//! Known-answer vectors for the passkey ceremonies (preflight ruling R14).
//!
//! The key is the RFC 6979 A.2.5 P-256 key (`tests/fixtures/passkey/rfc6979_a25_pkcs8.pem`), the
//! credential id is `00 01 .. 1f`, and the requests are the captured Chrome fixtures. Signatures
//! are RFC 6979 deterministic, so every byte is fixed. The expected values were produced once by
//! this code and cross-checked independently before they were pasted: Python `cryptography` 50.0.2
//! (OpenSSL) verified both signatures against the A.2.5 public key and reproduced them byte for
//! byte with `deterministic_signing=True`, and confirmed the SPKI `publicKey` and the rpIdHash.
//! The tests below also verify every signature with `p256` and decode the attestation object with
//! `ciborium`, a different code path than our encoder. They guard against accidental format drift.

// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use ciborium::value::Value as Cbor;
use mypass_core::passkey::{
    b64url, client_data_json, parse_create, parse_get, perform_create_with, perform_get,
    BackupFlags, PasskeySigningKey, AAGUID,
};
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{DerSignature, Signature, SigningKey, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use sha2::{Digest, Sha256};

const PKCS8: &str = include_str!("fixtures/passkey/rfc6979_a25_pkcs8.pem");
const CREATE_REQUEST: &str = include_str!("fixtures/passkey/chrome_create_request.json");
const GET_REQUEST: &str = include_str!("fixtures/passkey/chrome_get_request.json");
const ORIGIN: &str = "https://example.com";

/// RFC 6979 A.2.5: the public key of the fixture key.
const A25_UX: &str = "60fed4ba255a9d31c961eb74c6356d68c049b8923b61fa6ce669622e60f29fb6";
const A25_UY: &str = "7903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294d4462299";

const CREATE_AUTH_DATA: &str = concat!(
    "a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce19475d00000000948034",
    "71f8b448f998fffc4ac1a92af80020000102030405060708090a0b0c0d0e0f101112131415161718",
    "191a1b1c1d1e1fa501020326200121582060fed4ba255a9d31c961eb74c6356d68c049b8923b61fa",
    "6ce669622e60f29fb62258207903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294",
    "d4462299",
);
const CREATE_SIGNATURE: &str = concat!(
    "304502202b51c14f2046ed38966929844025cfddf7e268d007d2549a4360483c333866d1022100e1",
    "9010608d6d586e28c977e860dddc27bbe27c16653e5201e397d3c967038928",
);
const CREATE_CLIENT_DATA: &str = concat!(
    r#"{"type":"webauthn.create","challenge":"EBESExQVFhcYGRobHB0eHyAhIiMkJSYnKCkqKywtLi8","#,
    r#""origin":"https://example.com","crossOrigin":false}"#
);
const GET_AUTH_DATA: &str =
    "a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce19471d00000000";
const GET_SIGNATURE: &str = concat!(
    "30440220598dc651e0cb497f2174a70851b9ade6e1df56548d14733232f54c46a87101f9022015b4",
    "80699b60e289d0eacea3ab90b14a2e830f933005a23b89fbb3fbcd615bdc",
);
const GET_CLIENT_DATA: &str = concat!(
    r#"{"type":"webauthn.get","challenge":"QEFCQ0RFRkdISUpLTE1OT1BRUlNUVVZXWFlaW1xdXl8","#,
    r#""origin":"https://example.com","crossOrigin":false}"#
);

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn credential_id() -> [u8; 32] {
    core::array::from_fn(|i| i as u8)
}

fn verifying_key() -> VerifyingKey {
    let mut point = vec![4u8];
    point.extend(unhex(A25_UX));
    point.extend(unhex(A25_UY));
    VerifyingKey::from_sec1_bytes(&point).unwrap()
}

fn verify(auth_data: &[u8], client_data: &[u8], der: &[u8]) {
    let mut msg = auth_data.to_vec();
    msg.extend_from_slice(&Sha256::digest(client_data));
    let sig = DerSignature::from_bytes(der).unwrap();
    verifying_key().verify(&msg, &sig).unwrap();
}

fn json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap()
}

fn b64(v: &serde_json::Value) -> Vec<u8> {
    b64url::decode(v.as_str().unwrap()).unwrap()
}

#[test]
fn signer_is_rfc6979_deterministic_a25_sample_vector() {
    // RFC 6979 A.2.5, message "sample", SHA-256: the signer the ceremonies use is deterministic.
    let key = SigningKey::from(&p256::SecretKey::from_pem(PKCS8).unwrap());
    let sig: Signature = key.sign(b"sample");
    assert_eq!(
        hex(&sig.to_bytes()),
        concat!(
            "efd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716",
            "f7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8"
        )
    );
    assert_eq!(key.verifying_key(), &verifying_key());
}

#[test]
fn known_answer_vector_is_byte_stable() {
    let req = parse_create(ORIGIN, CREATE_REQUEST).unwrap();
    let key = p256::SecretKey::from_pem(PKCS8).unwrap();
    let out = perform_create_with(&req, key, credential_id()).unwrap();
    assert_eq!(hex(&out.auth_data), CREATE_AUTH_DATA);
    assert_eq!(hex(&out.signature), CREATE_SIGNATURE);
    assert_eq!(
        client_data_json("webauthn.create", &req.request().challenge, ORIGIN),
        CREATE_CLIENT_DATA.as_bytes()
    );
    verify(
        &out.auth_data,
        CREATE_CLIENT_DATA.as_bytes(),
        &out.signature,
    );
    // The PEM written for the vault is the same key.
    assert_eq!(&*out.private_key_pem, PKCS8);

    let resp = json(&out.response_json);
    assert_eq!(
        b64(&resp["response"]["clientDataJSON"]),
        CREATE_CLIENT_DATA.as_bytes()
    );
    assert_eq!(b64(&resp["response"]["authenticatorData"]), out.auth_data);
    let spki = b64(&resp["response"]["publicKey"]);
    assert_eq!(
        VerifyingKey::from_public_key_der(&spki).unwrap(),
        verifying_key()
    );
    // The attestation object, decoded by ciborium.
    let att: Cbor =
        ciborium::de::from_reader(b64(&resp["response"]["attestationObject"]).as_slice()).unwrap();
    assert_eq!(
        att,
        Cbor::Map(vec![
            (Cbor::Text("fmt".into()), Cbor::Text("none".into())),
            (Cbor::Text("attStmt".into()), Cbor::Map(vec![])),
            (
                Cbor::Text("authData".into()),
                Cbor::Bytes(out.auth_data.clone())
            ),
        ])
    );
    // authData fields, and the COSE key decoded by ciborium.
    let a = &out.auth_data;
    assert_eq!(a[..32], Sha256::digest(b"example.com")[..]);
    assert_eq!(a[32], 0x5d);
    assert_eq!(a[37..53], AAGUID);
    assert_eq!(a[53..55], [0, 32]);
    assert_eq!(a[55..87], credential_id());
    let cose: Cbor = ciborium::de::from_reader(&a[87..]).unwrap();
    let int = |i: i64| Cbor::Integer(i.into());
    assert_eq!(
        cose,
        Cbor::Map(vec![
            (int(1), int(2)),
            (int(3), int(-7)),
            (int(-1), int(1)),
            (int(-2), Cbor::Bytes(unhex(A25_UX))),
            (int(-3), Cbor::Bytes(unhex(A25_UY))),
        ])
    );
}

#[test]
fn known_answer_assertion_is_byte_stable() {
    let id = credential_id();
    let get = GET_REQUEST.replace(
        "wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t8",
        &b64url::encode(&id),
    );
    let req = parse_get(ORIGIN, &get).unwrap();
    let key = PasskeySigningKey::from_pem(PKCS8).unwrap();
    let out = perform_get(&req, &key, &id, Some(&[0xa0, 0xa1]), BackupFlags::default()).unwrap();
    assert_eq!(hex(&out.auth_data), GET_AUTH_DATA);
    assert_eq!(hex(&out.signature), GET_SIGNATURE);
    verify(&out.auth_data, GET_CLIENT_DATA.as_bytes(), &out.signature);
    let resp = json(&out.response_json);
    assert_eq!(
        b64(&resp["response"]["clientDataJSON"]),
        GET_CLIENT_DATA.as_bytes()
    );
    assert_eq!(resp["response"]["userHandle"], "oKE");
    assert_eq!(resp["id"], b64url::encode(&id).as_str());
}
