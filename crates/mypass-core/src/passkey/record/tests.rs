use super::*;
use crate::passkey::{parse_create, parse_get, perform_create, perform_get, PasskeySigningKey};
use mypass_kdbx::{Argon2Variant, Database, KdfParams};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{DerSignature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use secrecy::SecretString;
use sha2::{Digest, Sha256};

const A25_PKCS8: &str = include_str!("../../../tests/fixtures/passkey/rfc6979_a25_pkcs8.pem");
const A25_SEC1: &str = include_str!("../../../tests/fixtures/passkey/rfc6979_a25_sec1.pem");
const KEEPASSXC: &str = include_str!("../../../tests/fixtures/passkey/keepassxc_style_entry.json");

fn entry(fields: &[(&str, &str)]) -> Entry {
    let mut e = Entry::new(Timestamp(0));
    for (k, v) in fields {
        e.set_field(k, v, false);
    }
    e
}

fn sample() -> PasskeyRecord {
    PasskeyRecord {
        rp_id: "example.com".into(),
        credential_id: vec![1, 2, 3, 4],
        private_key_pem: Zeroizing::new(A25_PKCS8.to_owned()),
        user_handle: Some(vec![9, 9, 9]),
        username: Some("alice".into()),
        created: Some(Timestamp(1_700_000_000)),
        flags: BackupFlags::default(),
    }
}

fn protected(e: &Entry, key: &str) -> Option<bool> {
    e.fields.iter().find(|f| f.key == key).map(|f| f.protected)
}

fn kpex_keys(e: &Entry) -> Vec<&str> {
    let mut keys: Vec<&str> = e
        .fields
        .iter()
        .map(|f| f.key.as_str())
        .filter(|k| k.starts_with(PASSKEY_PREFIX))
        .collect();
    keys.sort_unstable();
    keys
}

fn fixture_entry(variant: &str) -> (Entry, serde_json::Value) {
    let doc: serde_json::Value = serde_json::from_str(KEEPASSXC).unwrap();
    let mut e = Entry::new(Timestamp(0));
    for f in doc["entries"][variant]["fields"].as_array().unwrap() {
        e.set_field(
            f["key"].as_str().unwrap(),
            f["value"].as_str().unwrap(),
            f["protected"].as_bool().unwrap(),
        );
    }
    (e, doc)
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn get_request(allow: &[u8]) -> crate::passkey::ValidatedGet {
    let json = format!(
        r#"{{"challenge":"AAECAwQFBgcICQoLDA0ODw","rpId":"example.com",
        "allowCredentials":[{{"type":"public-key","id":"{}"}}],
        "extensions":{{"remoteDesktopClientOverride":
            {{"origin":"https://example.com","sameOriginWithAncestors":true}}}}}}"#,
        b64url::encode(allow)
    );
    parse_get("https://example.com", &json).unwrap()
}

#[test]
fn fields_round_trip() {
    let mut e = entry(&[("Title", "Example")]);
    let r = sample();
    r.write(&mut e);
    assert_eq!(PasskeyRecord::read(&e), Some(r.clone()));
    for flags in [
        BackupFlags {
            eligible: true,
            state: false,
        },
        BackupFlags {
            eligible: false,
            state: false,
        },
    ] {
        let r = PasskeyRecord { flags, ..r.clone() };
        r.write(&mut e);
        assert_eq!(PasskeyRecord::read(&e).unwrap().flags, flags);
    }
}

#[test]
fn absent_passkey_reads_none_for_every_accessor() {
    let e = entry(&[("Title", "Example"), ("Password", "hunter2")]);
    assert_eq!(PasskeyRecord::read(&e), None);
    assert!(!PasskeyRecord::has_passkey(&e));
    assert!(!PasskeyRecord::has_private_key(&e));
    assert_eq!(PasskeyRecord::relying_party(&e), None);
    assert_eq!(PasskeyRecord::credential_id(&e), None);
}

#[test]
fn golden_fields_match_keepassxc_conventions() {
    let mut e = Entry::new(Timestamp(0));
    sample().write(&mut e);
    let mut expected = KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(kpex_keys(&e), expected);
    assert_eq!(e.fields.len(), 8);
    assert_eq!(e.field(PASSKEY_RELYING_PARTY), Some("example.com"));
    assert_eq!(e.field(PASSKEY_CREDENTIAL_ID), Some("AQIDBA"));
    assert_eq!(e.field(PASSKEY_USER_HANDLE), Some("CQkJ"));
    assert_eq!(e.field(PASSKEY_USERNAME), Some("alice"));
    assert!(e
        .field(PASSKEY_PRIVATE_KEY_PEM)
        .unwrap()
        .starts_with("-----BEGIN PRIVATE KEY-----\n"));
    assert_eq!(e.field(PASSKEY_FLAG_BE), Some("1"));
    assert_eq!(e.field(PASSKEY_FLAG_BS), Some("1"));
    assert_eq!(e.field(PASSKEY_CREATED), Some("2023-11-14T22:13:20Z"));
    // A freshly created passkey stored through the codec: unpadded base64url, PKCS#8, 1/1.
    let created = perform_create(
        &parse_create(
            "https://example.com",
            r#"{"challenge":"AAECAwQFBgcICQoLDA0ODw","rp":{"id":"example.com"},
            "user":{"id":"_-8","name":"a"},
            "extensions":{"remoteDesktopClientOverride":
                {"origin":"https://example.com","sameOriginWithAncestors":true}}}"#,
        )
        .unwrap(),
    )
    .unwrap();
    let mut e = Entry::new(Timestamp(0));
    PasskeyRecord {
        credential_id: created.credential_id.clone(),
        private_key_pem: created.private_key_pem.clone(),
        user_handle: Some(vec![0xff, 0xef]),
        ..sample()
    }
    .write(&mut e);
    let id = e.field(PASSKEY_CREDENTIAL_ID).unwrap();
    assert_eq!(id.len(), 43);
    assert!(!id.contains(['=', '+', '/']));
    assert_eq!(e.field(PASSKEY_USER_HANDLE), Some("_-8"));
}

#[test]
fn secret_fields_are_protected_and_plain_fields_are_not() {
    let mut e = Entry::new(Timestamp(0));
    sample().write(&mut e);
    for key in [
        PASSKEY_CREDENTIAL_ID,
        PASSKEY_PRIVATE_KEY_PEM,
        PASSKEY_USER_HANDLE,
    ] {
        assert_eq!(protected(&e, key), Some(true), "{key}");
    }
    for key in [
        PASSKEY_RELYING_PARTY,
        PASSKEY_USERNAME,
        PASSKEY_FLAG_BE,
        PASSKEY_FLAG_BS,
        PASSKEY_CREATED,
    ] {
        assert_eq!(protected(&e, key), Some(false), "{key}");
    }
    // Writing over plain copies of the secret fields protects them.
    let mut e = entry(&[
        (PASSKEY_PRIVATE_KEY_PEM, "x"),
        (PASSKEY_CREDENTIAL_ID, "AQ"),
    ]);
    sample().write(&mut e);
    assert_eq!(protected(&e, PASSKEY_PRIVATE_KEY_PEM), Some(true));
    assert_eq!(protected(&e, PASSKEY_CREDENTIAL_ID), Some(true));
}

#[test]
fn setting_none_removes_only_that_field() {
    let mut e = Entry::new(Timestamp(0));
    sample().write(&mut e);
    for (strip, key) in [
        (0, PASSKEY_USER_HANDLE),
        (1, PASSKEY_USERNAME),
        (2, PASSKEY_CREATED),
    ] {
        let mut r = sample();
        match strip {
            0 => r.user_handle = None,
            1 => r.username = None,
            _ => r.created = None,
        }
        let mut e2 = e.clone();
        r.write(&mut e2);
        assert_eq!(e2.field(key), None, "{key}");
        assert_eq!(e2.fields.len(), 7, "{key}");
        assert_eq!(PasskeyRecord::read(&e2), Some(r));
    }
}

#[test]
fn backup_flags_default_to_one_one() {
    let base = [
        (PASSKEY_CREDENTIAL_ID, "AQ"),
        (PASSKEY_PRIVATE_KEY_PEM, "pem"),
    ];
    let read = |extra: &[(&str, &str)]| {
        let mut fields = base.to_vec();
        fields.extend_from_slice(extra);
        PasskeyRecord::read(&entry(&fields)).unwrap().flags
    };
    let one_one = BackupFlags::default();
    assert_eq!(read(&[]), one_one);
    assert_eq!(
        read(&[(PASSKEY_FLAG_BE, "yes"), (PASSKEY_FLAG_BS, "")]),
        one_one
    );
    assert_eq!(
        read(&[(PASSKEY_FLAG_BE, "1"), (PASSKEY_FLAG_BS, "true")]),
        one_one
    );
    assert_eq!(
        read(&[(PASSKEY_FLAG_BE, "1"), (PASSKEY_FLAG_BS, "0")]),
        BackupFlags {
            eligible: true,
            state: false
        }
    );
    assert_eq!(
        read(&[(PASSKEY_FLAG_BE, "False"), (PASSKEY_FLAG_BS, "1")]),
        BackupFlags {
            eligible: false,
            state: true
        }
    );
    // BS without BE never reaches the authenticator data.
    assert_eq!(
        BackupFlags {
            eligible: false,
            state: true
        }
        .bits(),
        0
    );
}

#[test]
fn clear_removes_every_kpex_passkey_field_including_unknown_ones() {
    let mut e = Entry::new(Timestamp(0));
    sample().write(&mut e);
    e.set_field("KPEX_PASSKEY_GENERATED_USER_ID", "x", false);
    PasskeyRecord::clear(&mut e);
    assert!(kpex_keys(&e).is_empty());
    assert_eq!(PasskeyRecord::read(&e), None);
}

#[test]
fn clear_leaves_other_fields_untouched() {
    let mut e = entry(&[
        ("Title", "Example"),
        ("UserName", "alice"),
        ("KPEX_OTHER", "k"),
    ]);
    e.set_field("Password", "hunter2", true);
    sample().write(&mut e);
    let before: Vec<_> = e
        .fields
        .iter()
        .filter(|f| !f.key.starts_with(PASSKEY_PREFIX))
        .cloned()
        .collect();
    PasskeyRecord::clear(&mut e);
    assert_eq!(e.fields, before);
    assert_eq!(e.field("Password"), Some("hunter2"));
    assert_eq!(e.field("KPEX_OTHER"), Some("k"));
}

#[test]
fn created_round_trips_to_the_second() {
    for t in [0, 1_700_000_000, 1_700_000_001, 253_402_300_799] {
        let text = format_created(Timestamp(t)).unwrap();
        assert_eq!(parse_created(&text), Some(Timestamp(t)), "{text}");
    }
    assert_eq!(
        format_created(Timestamp(1_700_000_000)).as_deref(),
        Some("2023-11-14T22:13:20Z")
    );
    for (text, t) in [
        ("2023-11-14T22:13:20Z", 1_700_000_000),
        ("2023-11-14T22:13:20.5Z", 1_700_000_000),
        ("2023-11-14T22:13:20.123456Z", 1_700_000_000),
        ("2023-11-14T22:13:20+00:00", 1_700_000_000),
        ("2023-11-14T22:13:20.999+00:00", 1_700_000_000),
    ] {
        assert_eq!(parse_created(text), Some(Timestamp(t)), "{text}");
    }
    // Out of the writable range: nothing is written.
    assert_eq!(format_created(Timestamp(i64::MAX)), None);
    let mut e = Entry::new(Timestamp(0));
    PasskeyRecord {
        created: Some(Timestamp(i64::MAX)),
        ..sample()
    }
    .write(&mut e);
    assert_eq!(e.field(PASSKEY_CREATED), None);
}

#[test]
fn created_none_when_absent_or_not_a_date() {
    let base = [
        (PASSKEY_CREDENTIAL_ID, "AQ"),
        (PASSKEY_PRIVATE_KEY_PEM, "pem"),
    ];
    assert_eq!(PasskeyRecord::read(&entry(&base)).unwrap().created, None);
    for bad in [
        "",
        "yesterday",
        "2023-02-31T00:00:00Z",
        "2023-11-14T22:13:20",
        "2023-11-14T22:13:20.Z",
        "2023-11-14T22:13:20.1aZ",
        "2023-11-14T22:13:20+01:00",
        "2023-11-14T22:13:20-00:00",
        "2023-11-14 22:13:20Z",
        "2023-11-14T22:13:20ZZ",
        "\u{663}023-11-14T22:13:20Z",
    ] {
        assert_eq!(parse_created(bad), None, "{bad:?}");
        let mut fields = base.to_vec();
        fields.push((PASSKEY_CREATED, bad));
        assert_eq!(PasskeyRecord::read(&entry(&fields)).unwrap().created, None);
    }
}

#[test]
fn has_passkey_requires_both_nonempty_and_decodable() {
    let cases: Vec<(Vec<(&str, &str)>, bool)> = vec![
        (
            vec![
                (PASSKEY_CREDENTIAL_ID, "abc"),
                (PASSKEY_PRIVATE_KEY_PEM, "pem"),
            ],
            true,
        ),
        (
            vec![
                (PASSKEY_CREDENTIAL_ID, "AQ=="),
                (PASSKEY_PRIVATE_KEY_PEM, "pem"),
            ],
            true,
        ),
        (
            vec![
                (PASSKEY_CREDENTIAL_ID, "+/8"),
                (PASSKEY_PRIVATE_KEY_PEM, "pem"),
            ],
            true,
        ),
        (vec![(PASSKEY_CREDENTIAL_ID, "abc")], false),
        (vec![(PASSKEY_PRIVATE_KEY_PEM, "pem")], false),
        (
            vec![(PASSKEY_CREDENTIAL_ID, ""), (PASSKEY_PRIVATE_KEY_PEM, "")],
            false,
        ),
        (
            vec![
                (PASSKEY_CREDENTIAL_ID, "!!"),
                (PASSKEY_PRIVATE_KEY_PEM, "pem"),
            ],
            false,
        ),
        (
            vec![
                (PASSKEY_CREDENTIAL_ID, ""),
                (PASSKEY_PRIVATE_KEY_PEM, "pem"),
            ],
            false,
        ),
        (
            vec![
                (PASSKEY_CREDENTIAL_ID, "=="),
                (PASSKEY_PRIVATE_KEY_PEM, "pem"),
            ],
            false,
        ),
        (
            vec![(PASSKEY_CREDENTIAL_ID, "AQ"), (PASSKEY_PRIVATE_KEY_PEM, "")],
            false,
        ),
        (vec![], false),
    ];
    for (fields, expected) in cases {
        let e = entry(&fields);
        assert_eq!(has_passkey(&e), expected, "{fields:?}");
        assert_eq!(PasskeyRecord::has_passkey(&e), expected, "{fields:?}");
        assert_eq!(PasskeyRecord::read(&e).is_some(), expected, "{fields:?}");
    }
    // A PEM alone is enough for the replace guard, though not for a passkey.
    let pem_only = entry(&[
        (PASSKEY_CREDENTIAL_ID, "!!"),
        (PASSKEY_PRIVATE_KEY_PEM, "pem"),
    ]);
    assert!(PasskeyRecord::has_private_key(&pem_only));
    assert!(!has_passkey(&pem_only));
}

#[test]
fn keepassxc_style_entry_reads_and_signs() {
    for variant in ["unpadded", "padded"] {
        let (e, doc) = fixture_entry(variant);
        let cid = unhex(doc["credential_id_hex"].as_str().unwrap());
        assert!(has_passkey(&e), "{variant}");
        let r = PasskeyRecord::read(&e).unwrap();
        assert_eq!(r.rp_id, "example.com");
        assert_eq!(r.credential_id, cid);
        assert_eq!(
            r.user_handle.as_deref(),
            Some(unhex(doc["user_handle_hex"].as_str().unwrap()).as_slice())
        );
        assert_eq!(r.username.as_deref(), Some("alice@example.com"));
        assert_eq!(r.created, None);
        assert_eq!(r.flags, BackupFlags::default());

        let key = PasskeySigningKey::from_pem(&r.private_key_pem).unwrap();
        let out = perform_get(
            &get_request(&cid),
            &key,
            &r.credential_id,
            r.user_handle.as_deref(),
            r.flags,
        )
        .unwrap();
        let spki = b64url::decode(doc["public_key_spki_der_b64url"].as_str().unwrap()).unwrap();
        let vk = VerifyingKey::from_public_key_der(&spki).unwrap();
        let resp: serde_json::Value = serde_json::from_str(&out.response_json).unwrap();
        let client_data =
            b64url::decode(resp["response"]["clientDataJSON"].as_str().unwrap()).unwrap();
        let mut msg = out.auth_data.clone();
        msg.extend_from_slice(&Sha256::digest(&client_data));
        vk.verify(&msg, &DerSignature::from_bytes(&out.signature).unwrap())
            .unwrap();
        assert_eq!(out.auth_data[32], 0x1D);
        assert_eq!(
            resp["response"]["userHandle"],
            b64url::encode(r.user_handle.as_deref().unwrap()).as_str()
        );
    }
}

#[test]
fn sec1_pem_is_accepted_on_read() {
    let mut e = entry(&[
        (PASSKEY_CREDENTIAL_ID, "AQID"),
        (PASSKEY_RELYING_PARTY, "example.com"),
    ]);
    e.set_field(PASSKEY_PRIVATE_KEY_PEM, A25_SEC1, true);
    let r = PasskeyRecord::read(&e).unwrap();
    assert!(r
        .private_key_pem
        .starts_with("-----BEGIN EC PRIVATE KEY-----"));
    let sec1 = PasskeySigningKey::from_pem(&r.private_key_pem).unwrap();
    let pkcs8 = PasskeySigningKey::from_pem(A25_PKCS8).unwrap();
    assert_eq!(
        sec1.public_key_spki_der().unwrap(),
        pkcs8.public_key_spki_der().unwrap()
    );
}

#[test]
fn write_over_existing_passkey_keeps_unknown_kpex_fields() {
    let mut e = entry(&[("Title", "Example")]);
    sample().write(&mut e);
    e.set_field("KPEX_PASSKEY_GENERATED_USER_ID", "generated", false);
    let replacement = PasskeyRecord {
        credential_id: vec![7; 32],
        user_handle: None,
        username: None,
        created: None,
        ..sample()
    };
    replacement.write(&mut e);
    assert_eq!(e.field("KPEX_PASSKEY_GENERATED_USER_ID"), Some("generated"));
    assert_eq!(e.field(PASSKEY_USER_HANDLE), None);
    assert_eq!(PasskeyRecord::read(&e), Some(replacement));
}

#[test]
fn debug_output_redacts_private_key() {
    let r = PasskeyRecord {
        user_handle: Some(b"secret-handle".to_vec()),
        ..sample()
    };
    let text = format!("{r:?}");
    assert!(!text.contains("BEGIN"));
    assert!(!text.contains(A25_PKCS8.lines().nth(1).unwrap()));
    assert!(!text.contains("secret-handle") && !text.contains("115, 101"));
    assert!(text.contains("example.com"));
}

#[test]
fn write_then_save_then_open_roundtrip_through_database() {
    let mut db = Database::new("passkeys");
    // Argon2id 8 MiB, 1 pass: fast, built inline (core must not depend on mypass-session).
    if let KdfParams::Argon2 {
        variant,
        memory_bytes,
        iterations,
        parallelism,
        ..
    } = &mut db.settings.kdf
    {
        *variant = Argon2Variant::Id;
        *memory_bytes = 8 * 1024 * 1024;
        *iterations = 1;
        *parallelism = 1;
    }
    let root = db.root_id();
    let mut e = entry(&[("Title", "Example")]);
    let r = sample();
    r.write(&mut e);
    let id = e.uuid;
    db.add_entry(root, e, Timestamp(1)).unwrap();

    let password = SecretString::from("synthetic");
    let bytes = db.save(&password).unwrap();
    let reopened = Database::open(&bytes, &password).unwrap();
    let e = reopened.find_entry(id).unwrap();
    assert_eq!(PasskeyRecord::read(e), Some(r));
    for key in [
        PASSKEY_CREDENTIAL_ID,
        PASSKEY_PRIVATE_KEY_PEM,
        PASSKEY_USER_HANDLE,
    ] {
        assert_eq!(protected(e, key), Some(true), "{key}");
    }
    for key in [
        PASSKEY_RELYING_PARTY,
        PASSKEY_USERNAME,
        PASSKEY_FLAG_BE,
        PASSKEY_FLAG_BS,
        PASSKEY_CREATED,
    ] {
        assert_eq!(protected(e, key), Some(false), "{key}");
    }
}

/// Review M4 (Tasks 3+4): an undecodable or empty USER_HANDLE reads as `None` and the entry still
/// counts as a passkey (Swift behaved the same); the get response then carries no `userHandle`.
#[test]
fn undecodable_user_handle_reads_as_none_and_keeps_the_passkey() {
    for bad in ["!!", "", "=="] {
        let e = entry(&[
            (PASSKEY_RELYING_PARTY, "example.com"),
            (PASSKEY_CREDENTIAL_ID, "AQIDBA"),
            (PASSKEY_PRIVATE_KEY_PEM, A25_PKCS8),
            (PASSKEY_USER_HANDLE, bad),
        ]);
        assert!(has_passkey(&e), "{bad:?}");
        let r = PasskeyRecord::read(&e).unwrap();
        assert_eq!(r.user_handle, None, "{bad:?}");
        assert_eq!(r.credential_id, vec![1, 2, 3, 4]);
    }
}
