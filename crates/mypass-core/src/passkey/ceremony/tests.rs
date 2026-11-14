use super::*;
use crate::passkey::{parse_create, parse_get};
use ciborium::value::Value as Cbor;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::VerifyingKey;
use p256::pkcs8::DecodePublicKey;
use std::collections::HashSet;

const ORIGIN: &str = "https://login.example.com";
const RP_ID: &str = "example.com";
const CHALLENGE: &[u8] = b"0123456789abcdef-challenge";

fn create_json(attestation: Option<&str>) -> String {
    let attestation = attestation
        .map(|a| format!(r#""attestation":"{a}","#))
        .unwrap_or_default();
    format!(
        r#"{{{attestation}"challenge":"{}","rp":{{"id":"Example.COM","name":"Example"}},
        "user":{{"id":"{}","name":"alice"}},
        "pubKeyCredParams":[{{"type":"public-key","alg":-7}}],
        "extensions":{{"remoteDesktopClientOverride":{{"origin":"{ORIGIN}","sameOriginWithAncestors":true}}}}}}"#,
        b64url::encode(CHALLENGE),
        b64url::encode(b"user-handle"),
    )
}

fn create_request(attestation: Option<&str>) -> ValidatedCreate {
    parse_create(ORIGIN, &create_json(attestation)).unwrap()
}

fn get_request(allow: &[&[u8]]) -> ValidatedGet {
    let allow: Vec<String> = allow
        .iter()
        .map(|id| format!(r#"{{"type":"public-key","id":"{}"}}"#, b64url::encode(id)))
        .collect();
    let json = format!(
        r#"{{"challenge":"{}","rpId":"EXAMPLE.com","allowCredentials":[{}],
        "extensions":{{"remoteDesktopClientOverride":{{"origin":"{ORIGIN}","sameOriginWithAncestors":true}}}}}}"#,
        b64url::encode(CHALLENGE),
        allow.join(",")
    );
    parse_get(ORIGIN, &json).unwrap()
}

fn response(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap()
}

fn field_bytes(v: &serde_json::Value, path: &[&str]) -> Vec<u8> {
    let mut cur = v;
    for p in path {
        cur = &cur[*p];
    }
    b64url::decode(cur.as_str().unwrap()).unwrap()
}

fn cbor_map(bytes: &[u8]) -> Vec<(Cbor, Cbor)> {
    match ciborium::de::from_reader::<Cbor, _>(bytes).unwrap() {
        Cbor::Map(m) => m,
        other => panic!("not a map: {other:?}"),
    }
}

fn lookup<'a>(map: &'a [(Cbor, Cbor)], key: &str) -> &'a Cbor {
    &map.iter()
        .find(|(k, _)| k == &Cbor::Text(key.into()))
        .unwrap()
        .1
}

/// The independent verification path: SPKI from the JSON response, the client data hash
/// recomputed here, the DER signature parsed and checked by `p256`.
fn verify(spki: &[u8], auth_data: &[u8], client_data: &[u8], der: &[u8]) {
    let public = p256::PublicKey::from_public_key_der(spki).unwrap();
    let vk = VerifyingKey::from(&public);
    let mut msg = auth_data.to_vec();
    msg.extend_from_slice(&Sha256::digest(client_data));
    let sig = p256::ecdsa::DerSignature::from_bytes(der).unwrap();
    vk.verify(&msg, &sig).unwrap();
    // A different message must not verify.
    msg.push(0);
    assert!(vk.verify(&msg, &sig).is_err());
}

/// COSE x and y, read with ciborium from the attested credential data of `auth_data`.
fn cose_entries(auth_data: &[u8]) -> Vec<(Cbor, Cbor)> {
    let id_len = u16::from_be_bytes([auth_data[53], auth_data[54]]) as usize;
    cbor_map(&auth_data[55 + id_len..])
}

fn spki_from_cose(auth_data: &[u8]) -> Vec<u8> {
    let cose = cose_entries(auth_data);
    let x = cose[3].1.as_bytes().unwrap().clone();
    let y = cose[4].1.as_bytes().unwrap().clone();
    let mut point = vec![4u8];
    point.extend(x);
    point.extend(y);
    p256::PublicKey::from_sec1_bytes(&point)
        .unwrap()
        .to_public_key_der()
        .unwrap()
        .as_bytes()
        .to_vec()
}

#[test]
fn create_signature_verifies_with_independent_verifier() {
    for attestation in [None, Some("direct")] {
        let req = create_request(attestation);
        let out = perform_create(&req).unwrap();
        let resp = response(&out.response_json);
        let att = field_bytes(&resp, &["response", "attestationObject"]);
        let map = cbor_map(&att);
        let auth_data = lookup(&map, "authData").as_bytes().unwrap().clone();
        assert_eq!(auth_data, out.auth_data);
        let client_data = field_bytes(&resp, &["response", "clientDataJSON"]);
        let spki = field_bytes(&resp, &["response", "publicKey"]);
        verify(&spki, &auth_data, &client_data, &out.signature);
        // The COSE key in authData is the same key as the SPKI.
        assert_eq!(spki_from_cose(&auth_data), spki);
        if attestation.is_some() {
            let stmt = match lookup(&map, "attStmt") {
                Cbor::Map(m) => m.clone(),
                other => panic!("{other:?}"),
            };
            let sig = lookup(&stmt, "sig").as_bytes().unwrap().clone();
            assert_eq!(sig, out.signature);
            verify(&spki, &auth_data, &client_data, &sig);
        }
    }
}

#[test]
fn get_signature_verifies_over_authdata_and_client_data_hash() {
    let created = perform_create(&create_request(None)).unwrap();
    let key = PasskeySigningKey::from_pem(&created.private_key_pem).unwrap();
    let spki = field_bytes(
        &response(&created.response_json),
        &["response", "publicKey"],
    );
    let req = get_request(&[&created.credential_id]);
    let out = perform_get(
        &req,
        &key,
        &created.credential_id,
        Some(b"user-handle"),
        BackupFlags::default(),
    )
    .unwrap();
    let resp = response(&out.response_json);
    let auth_data = field_bytes(&resp, &["response", "authenticatorData"]);
    let client_data = field_bytes(&resp, &["response", "clientDataJSON"]);
    let sig = field_bytes(&resp, &["response", "signature"]);
    assert_eq!(auth_data, out.auth_data);
    assert_eq!(sig, out.signature);
    verify(&spki, &auth_data, &client_data, &sig);
    assert_eq!(key.public_key_spki_der().unwrap(), spki);
    let cd = response(std::str::from_utf8(&client_data).unwrap());
    assert_eq!(cd["type"], "webauthn.get");
    assert_eq!(cd["origin"], ORIGIN);
    assert_eq!(cd["challenge"], b64url::encode(CHALLENGE));
}

#[test]
fn create_auth_data_layout_flags_counter_aaguid_credid_cose() {
    let out = perform_create(&create_request(None)).unwrap();
    let a = &out.auth_data;
    assert_eq!(a[32], 0x5D);
    assert_eq!(a[33..37], [0u8; 4]);
    assert_eq!(a[37..53], AAGUID);
    assert_eq!(u16::from_be_bytes([a[53], a[54]]), 32);
    assert_eq!(a[55..87], out.credential_id[..]);
    let cose = cose_entries(a);
    let keys: Vec<Cbor> = cose.iter().map(|(k, _)| k.clone()).collect();
    let int = |i: i64| Cbor::Integer(i.into());
    assert_eq!(keys, vec![int(1), int(3), int(-1), int(-2), int(-3)]);
    assert_eq!(cose[0].1, int(2));
    assert_eq!(cose[1].1, int(-7));
    assert_eq!(cose[2].1, int(1));
    assert_eq!(cose[3].1.as_bytes().unwrap().len(), 32);
    assert_eq!(cose[4].1.as_bytes().unwrap().len(), 32);
    // Raw bytes: map(5), 1:2, 3:-7, -1:1, -2:bstr(32), then -3:bstr(32) closes authData exactly.
    assert_eq!(a[87..94], [0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01]);
    assert_eq!(a[94..96], [0x21, 0x58]);
    assert_eq!(a.len(), 87 + 7 + 2 * (1 + 2 + 32));
}

#[test]
fn get_auth_data_has_no_attested_credential_data() {
    let created = perform_create(&create_request(None)).unwrap();
    let key = PasskeySigningKey::from_pem(&created.private_key_pem).unwrap();
    let out = perform_get(
        &get_request(&[]),
        &key,
        &created.credential_id,
        None,
        BackupFlags::default(),
    )
    .unwrap();
    assert_eq!(out.auth_data.len(), 37);
    assert_eq!(out.auth_data[32], 0x1D);
    assert_eq!(out.auth_data[33..37], [0u8; 4]);
}

#[test]
fn rp_id_hash_is_sha256_of_rp_id() {
    // The page asked for "Example.COM" / "EXAMPLE.com": the hash is over the canonical rpId.
    let expected = Sha256::digest(RP_ID.as_bytes());
    let created = perform_create(&create_request(None)).unwrap();
    assert_eq!(created.auth_data[..32], expected[..]);
    assert_ne!(created.auth_data[..32], Sha256::digest(b"Example.COM")[..]);
    let key = PasskeySigningKey::from_pem(&created.private_key_pem).unwrap();
    let got = perform_get(
        &get_request(&[]),
        &key,
        &created.credential_id,
        None,
        BackupFlags::default(),
    )
    .unwrap();
    assert_eq!(got.auth_data[..32], expected[..]);
}

#[test]
fn attestation_none_uses_none_format_with_empty_statement() {
    for attestation in [None, Some("none"), Some("bogus"), Some("DIRECT")] {
        let out = perform_create(&create_request(attestation)).unwrap();
        let att = field_bytes(
            &response(&out.response_json),
            &["response", "attestationObject"],
        );
        let map = cbor_map(&att);
        assert_eq!(map.len(), 3, "{attestation:?}");
        assert_eq!(lookup(&map, "fmt"), &Cbor::Text("none".into()));
        assert_eq!(lookup(&map, "attStmt"), &Cbor::Map(vec![]));
    }
}

#[test]
fn attestation_other_than_none_uses_packed_self_attestation_with_alg_minus_7() {
    for attestation in ["direct", "indirect", "enterprise"] {
        let out = perform_create(&create_request(Some(attestation))).unwrap();
        let att = field_bytes(
            &response(&out.response_json),
            &["response", "attestationObject"],
        );
        let map = cbor_map(&att);
        assert_eq!(lookup(&map, "fmt"), &Cbor::Text("packed".into()));
        let Cbor::Map(stmt) = lookup(&map, "attStmt") else {
            panic!("attStmt not a map")
        };
        let keys: Vec<&Cbor> = stmt.iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            vec![&Cbor::Text("alg".into()), &Cbor::Text("sig".into())]
        );
        assert_eq!(lookup(stmt, "alg"), &Cbor::Integer((-7).into()));
        assert_eq!(
            lookup(stmt, "sig").as_bytes().unwrap(),
            &out.signature,
            "{attestation}"
        );
    }
}

#[test]
fn attestation_object_key_order_is_fmt_attstmt_authdata() {
    for attestation in [None, Some("direct")] {
        let out = perform_create(&create_request(attestation)).unwrap();
        let att = field_bytes(
            &response(&out.response_json),
            &["response", "attestationObject"],
        );
        assert_eq!(att[0], 0xa3);
        let find = |needle: &[u8]| att.windows(needle.len()).position(|w| w == needle).unwrap();
        let fmt = find(b"\x63fmt");
        let stmt = find(b"\x67attStmt");
        let auth = find(b"\x68authData");
        assert_eq!(fmt, 1);
        assert!(fmt < stmt && stmt < auth);
        // authData is the last item: its byte string runs to the end.
        assert!(att.ends_with(&out.auth_data));
    }
}

#[test]
fn create_answers_cred_props_rk_true_only_when_requested() {
    let asked =
        create_json(None).replace(r#""extensions":{"#, r#""extensions":{"credProps":true,"#);
    let request = parse_create(ORIGIN, &asked).unwrap();
    assert!(request.request().cred_props);
    let resp = response(&perform_create(&request).unwrap().response_json);
    assert_eq!(
        resp["clientExtensionResults"],
        serde_json::json!({"credProps": {"rk": true}})
    );
    let resp = response(&perform_create(&create_request(None)).unwrap().response_json);
    assert_eq!(resp["clientExtensionResults"], serde_json::json!({}));
}

#[test]
fn create_response_has_chromium_required_fields() {
    let out = perform_create(&create_request(None)).unwrap();
    let resp = response(&out.response_json);
    let id = b64url::encode(&out.credential_id);
    assert_eq!(resp["id"], id.as_str());
    assert_eq!(resp["rawId"], id.as_str());
    assert_eq!(resp["type"], "public-key");
    assert_eq!(resp["clientExtensionResults"], serde_json::json!({}));
    let r = &resp["response"];
    for key in [
        "clientDataJSON",
        "attestationObject",
        "authenticatorData",
        "publicKey",
    ] {
        assert!(r[key].is_string(), "{key}");
        let s = r[key].as_str().unwrap();
        assert!(!s.contains('=') && !s.contains('+') && !s.contains('/'));
    }
    assert_eq!(r["publicKeyAlgorithm"], -7);
    assert_eq!(r["transports"], serde_json::json!(["internal"]));
    assert_eq!(
        field_bytes(&resp, &["response", "authenticatorData"]),
        out.auth_data
    );
    let mut keys: Vec<&String> = resp.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["clientExtensionResults", "id", "rawId", "response", "type"]
    );
    // The response never carries key material.
    assert!(!out.response_json.contains("PRIVATE"));
    let der = p256::SecretKey::from_pem(&out.private_key_pem)
        .unwrap()
        .to_bytes();
    assert!(!out.response_json.contains(&b64url::encode(&der)));
}

#[test]
fn get_response_omits_user_handle_when_none_and_includes_it_otherwise() {
    let created = perform_create(&create_request(None)).unwrap();
    let key = PasskeySigningKey::from_pem(&created.private_key_pem).unwrap();
    let req = get_request(&[]);
    let none = perform_get(
        &req,
        &key,
        &created.credential_id,
        None,
        BackupFlags::default(),
    )
    .unwrap();
    let resp = response(&none.response_json);
    assert!(resp["response"].get("userHandle").is_none());
    assert_eq!(resp["id"], b64url::encode(&created.credential_id).as_str());
    assert_eq!(resp["rawId"], resp["id"]);
    assert_eq!(resp["type"], "public-key");
    assert_eq!(resp["clientExtensionResults"], serde_json::json!({}));
    let some = perform_get(
        &req,
        &key,
        &created.credential_id,
        Some(&[0xfb, 0xff]),
        BackupFlags::default(),
    )
    .unwrap();
    let resp = response(&some.response_json);
    assert_eq!(resp["response"]["userHandle"], "-_8");
    let mut keys: Vec<&String> = resp["response"].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "authenticatorData",
            "clientDataJSON",
            "signature",
            "userHandle"
        ]
    );
}

#[test]
fn get_refuses_a_credential_outside_allow_credentials_and_an_empty_id() {
    let created = perform_create(&create_request(None)).unwrap();
    let key = PasskeySigningKey::from_pem(&created.private_key_pem).unwrap();
    let other = [9u8; 32];
    let req = get_request(&[&other]);
    assert_eq!(
        perform_get(
            &req,
            &key,
            &created.credential_id,
            None,
            BackupFlags::default()
        ),
        Err(PasskeyError::CredentialNotAllowed)
    );
    assert!(perform_get(&req, &key, &other, None, BackupFlags::default()).is_ok());
    assert_eq!(
        perform_get(&get_request(&[]), &key, &[], None, BackupFlags::default()),
        Err(PasskeyError::InvalidRequest)
    );
}

#[test]
fn client_data_json_is_canonical_and_unescaped() {
    assert_eq!(
        client_data_json("webauthn.create", &[1, 2, 3], "https://example.com"),
        br#"{"type":"webauthn.create","challenge":"AQID","origin":"https://example.com","crossOrigin":false}"#
    );
    assert_eq!(
        client_data_json("webauthn.get", &[0xfb, 0xff], "https://a.example:8443"),
        br#"{"type":"webauthn.get","challenge":"-_8","origin":"https://a.example:8443","crossOrigin":false}"#
    );
    // Characters that JSON must escape are escaped, and the result is valid JSON.
    let odd = client_data_json("t", &[], "https://x\"\\\u{1}");
    let parsed = response(std::str::from_utf8(&odd).unwrap());
    assert_eq!(parsed["origin"], "https://x\"\\\u{1}");
}

#[test]
fn client_data_json_in_responses_uses_the_validated_origin() {
    let out = perform_create(&create_request(None)).unwrap();
    let cd = field_bytes(
        &response(&out.response_json),
        &["response", "clientDataJSON"],
    );
    assert_eq!(
        cd,
        client_data_json("webauthn.create", CHALLENGE, "https://login.example.com")
    );
}

#[test]
fn credential_ids_are_32_random_bytes_and_unique_over_1000_creates() {
    let req = create_request(None);
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for _ in 0..1000 {
        let out = perform_create(&req).unwrap();
        assert_eq!(out.credential_id.len(), 32);
        assert!(ids.insert(out.credential_id.clone()));
        assert!(keys.insert(out.private_key_pem.to_string()));
    }
}

#[test]
fn private_key_pem_is_pkcs8_pem_with_lf_and_round_trips_through_from_pem() {
    let out = perform_create(&create_request(None)).unwrap();
    let pem: &str = &out.private_key_pem;
    assert!(
        pem.starts_with("-----BEGIN PRIVATE KEY-----\n"),
        "{}",
        pem.len()
    );
    assert!(pem.ends_with("-----END PRIVATE KEY-----\n"));
    assert!(!pem.contains('\r'));
    let key = PasskeySigningKey::from_pem(pem).unwrap();
    let spki = field_bytes(&response(&out.response_json), &["response", "publicKey"]);
    assert_eq!(key.public_key_spki_der().unwrap(), spki);
    // CRLF and surrounding whitespace are tolerated.
    let crlf = format!("  \n{}\n", pem.replace('\n', "\r\n"));
    let key = PasskeySigningKey::from_pem(&crlf).unwrap();
    assert_eq!(key.public_key_spki_der().unwrap(), spki);
}

/// RFC 6979 A.2.5 P-256 key as SEC1 (`openssl ec -in pkcs8.pem`), and a P-384 key.
const A25_SEC1_PEM: &str = include_str!("../../../tests/fixtures/passkey/rfc6979_a25_sec1.pem");
const A25_PKCS8_PEM: &str = include_str!("../../../tests/fixtures/passkey/rfc6979_a25_pkcs8.pem");
const P384_PEM: &str = include_str!("../../../tests/fixtures/passkey/p384_pkcs8.pem");

#[test]
fn from_pem_accepts_sec1_and_rejects_garbage_and_p384() {
    let sec1 = PasskeySigningKey::from_pem(A25_SEC1_PEM).unwrap();
    let pkcs8 = PasskeySigningKey::from_pem(A25_PKCS8_PEM).unwrap();
    assert_eq!(
        sec1.public_key_spki_der().unwrap(),
        pkcs8.public_key_spki_der().unwrap()
    );
    for bad in [
        "",
        "garbage",
        "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
        P384_PEM,
        "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEYP7UuiVanTHJYet0xjVtaMBJuJI7Yfps5mliLmDyn7Z5A/4QCLi8maQa6elWKLxk8vGyDC1+n1F3o8KU1EYimQ==\n-----END PUBLIC KEY-----\n",
    ] {
        assert!(PasskeySigningKey::from_pem(bad).is_err());
    }
    // The error says nothing about the input.
    let err = PasskeySigningKey::from_pem(P384_PEM).err().unwrap();
    assert_eq!(err, PasskeyError::Crypto);
}

#[test]
fn debug_output_redacts_private_key() {
    let out = perform_create(&create_request(None)).unwrap();
    let text = format!("{out:?}");
    assert!(!text.contains("BEGIN"));
    assert!(!text.contains(out.private_key_pem.lines().nth(1).unwrap()));
    assert!(text.contains("<redacted>"));
}

#[test]
fn flags_follow_backup_flags_and_bs_requires_be() {
    let created = perform_create(&create_request(None)).unwrap();
    let key = PasskeySigningKey::from_pem(&created.private_key_pem).unwrap();
    let spki = field_bytes(
        &response(&created.response_json),
        &["response", "publicKey"],
    );
    let req = get_request(&[]);
    for (eligible, state, expected) in [
        (true, true, 0x1D),
        (true, false, 0x0D),
        (false, true, 0x05),
        (false, false, 0x05),
    ] {
        let out = perform_get(
            &req,
            &key,
            &created.credential_id,
            None,
            BackupFlags { eligible, state },
        )
        .unwrap();
        assert_eq!(out.auth_data[32], expected, "{eligible} {state}");
        let cd = field_bytes(
            &response(&out.response_json),
            &["response", "clientDataJSON"],
        );
        verify(&spki, &out.auth_data, &cd, &out.signature);
    }
    assert_eq!(
        BackupFlags::default(),
        BackupFlags {
            eligible: true,
            state: true
        }
    );
    assert_eq!(
        created.auth_data[32] & (FLAG_BE | FLAG_BS),
        FLAG_BE | FLAG_BS
    );
}
