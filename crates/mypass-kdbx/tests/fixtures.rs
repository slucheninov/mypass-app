// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_kdbx::{Argon2Variant, Cipher, Compression, Database, KdbxError, KdfParams};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
const XC: [&[u8]; 4] = [
    include_bytes!("fixtures/aeskdf-aes256-none.kdbx"),
    include_bytes!("fixtures/argon2d-aes256-gzip.kdbx"),
    include_bytes!("fixtures/argon2id-aes256-gzip.kdbx"),
    include_bytes!("fixtures/chacha20-argon2id-gzip.kdbx"),
];
const PY: &[u8] = include_bytes!("fixtures/pykeepass-attachments.kdbx");
#[test]
fn four_keepassxc_fixtures_open_and_recover_data() {
    for (data, cipher, compression, argon_variant) in [
        (XC[0], Cipher::Aes256Cbc, Compression::None, None),
        (
            XC[1],
            Cipher::Aes256Cbc,
            Compression::Gzip,
            Some(Argon2Variant::D),
        ),
        (
            XC[2],
            Cipher::Aes256Cbc,
            Compression::Gzip,
            Some(Argon2Variant::Id),
        ),
        (
            XC[3],
            Cipher::ChaCha20,
            Compression::Gzip,
            Some(Argon2Variant::Id),
        ),
    ] {
        let db = Database::open(data, &SecretString::from("TestMasterPass123")).unwrap();
        assert_eq!(db.settings.cipher, cipher);
        assert_eq!(db.settings.compression, compression);
        match argon_variant {
            None => assert!(matches!(db.settings.kdf, KdfParams::AesKdf { .. })),
            Some(expected) => assert!(matches!(
                db.settings.kdf, KdfParams::Argon2 { variant, .. } if variant == expected
            )),
        }
        assert_eq!(db.root.entries.len(), 1);
        let e = &db.root.entries[0];
        for (k, v) in [
            ("Title", "Test Entry"),
            ("UserName", "testuser"),
            ("Password", "hunter2"),
        ] {
            assert_eq!(e.field(k), Some(v));
        }
        assert!(
            e.fields
                .iter()
                .find(|f| f.key == "Password")
                .unwrap()
                .protected
        );
    }
}
#[test]
fn pykeepass_attachments_fixture_opens() {
    let db = Database::open(PY, &SecretString::from("fixture-password")).unwrap();
    // Read directly from this synthetic fixture's outer header: AES UUID, compression=1,
    // and KDF UUID ef636ddf8c29444b91f7a9a403e30a0c (Argon2d).
    assert_eq!(db.settings.cipher, Cipher::Aes256Cbc);
    assert_eq!(db.settings.compression, Compression::Gzip);
    assert!(matches!(
        db.settings.kdf,
        KdfParams::Argon2 {
            variant: Argon2Variant::D,
            ..
        }
    ));
    let docs = db
        .root
        .entries
        .iter()
        .find(|e| e.title() == "Docs")
        .unwrap();
    assert_eq!(docs.attachments.len(), 2);
    assert_eq!(docs.attachments[0].name, "note.txt");
    assert_eq!(docs.attachments[0].data, b"abc");
    let blob = &docs.attachments[1];
    assert_eq!(blob.name, "blob.bin");
    assert_eq!(blob.data.len(), 70 * 1024);
    assert_eq!(
        Sha256::digest(&blob.data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "6541e8907643d16a8a53fc9613ee0c8d7a8c5540085a14da9225ff0b09ef9deb"
    );
    assert_eq!(docs.history.len(), 1);
    assert_eq!(docs.history[0].attachments[0].name, "old.txt");
    assert_eq!(docs.history[0].attachments[0].data, b"old");
    let copy = db
        .root
        .entries
        .iter()
        .find(|e| e.title() == "Copy")
        .unwrap();
    assert_eq!(copy.attachments.len(), 1);
    assert_eq!(copy.attachments[0].name, "same.txt");
    assert_eq!(copy.attachments[0].data, b"abc");
    for e in [docs, &docs.history[0], copy] {
        assert!(e.attachments.iter().all(|a| a.protected));
    }
}
#[test]
fn wrong_password_on_every_fixture_is_wrong_password() {
    for data in XC.into_iter().chain([PY]) {
        assert_eq!(
            Database::open(data, &SecretString::from("nope")),
            Err(KdbxError::WrongPassword)
        );
    }
}
#[test]
fn fixtures_round_trip_logically_equal() {
    for (data, password) in XC
        .into_iter()
        .map(|d| (d, "TestMasterPass123"))
        .chain([(PY, "fixture-password")])
    {
        let p = SecretString::from(password);
        let db = Database::open(data, &p).unwrap();
        let next = Database::open(&db.save(&p).unwrap(), &p).unwrap();
        assert_eq!(next, db);
    }
}
#[test]
fn round_trip_preserves_kdf_cipher_compression() {
    assert_eq!(XC.len(), 4);
    for (data, password) in XC
        .into_iter()
        .map(|d| (d, "TestMasterPass123"))
        .chain([(PY, "fixture-password")])
    {
        let p = SecretString::from(password);
        let db = Database::open(data, &p).unwrap();
        assert!(!db.root.entries.is_empty());
        let next = Database::open(&db.save(&p).unwrap(), &p).unwrap();
        assert_eq!(next.settings.cipher, db.settings.cipher);
        assert_eq!(next.settings.compression, db.settings.compression);
        assert_eq!(next.settings.minor_version, db.settings.minor_version);
        let mut expected = db.settings.kdf.clone();
        match (&mut expected, &next.settings.kdf) {
            (KdfParams::Argon2 { salt: a, .. }, KdfParams::Argon2 { salt: b, .. })
            | (KdfParams::AesKdf { seed: a, .. }, KdfParams::AesKdf { seed: b, .. }) => {
                assert_eq!(a.len(), b.len());
                assert_ne!(a, b);
                *a = b.clone();
            }
            _ => panic!("KDF changed"),
        }
        assert_eq!(expected, next.settings.kdf);
    }
}

#[test]
fn pykeepass_fixture_round_trip_preserves_attachment_flags_and_history() {
    let p = SecretString::from("fixture-password");
    let db = Database::open(PY, &p).unwrap();
    // Assert the fixture's required content before saving, avoiding vacuous all().
    fn assert_attachment_fixture(vault: &Database) {
        assert_eq!(vault.root.entries.len(), 2);
        let docs = vault
            .root
            .entries
            .iter()
            .find(|e| e.title() == "Docs")
            .unwrap();
        assert_eq!(docs.attachments.len(), 2);
        assert_eq!(docs.attachments[0].name, "note.txt");
        assert_eq!(docs.attachments[0].data, b"abc");
        assert!(docs.attachments[0].protected);
        let blob = &docs.attachments[1];
        assert!(blob.protected);
        assert_eq!(blob.name, "blob.bin");
        assert_eq!(blob.data.len(), 70 * 1024);
        assert_eq!(
            Sha256::digest(&blob.data)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "6541e8907643d16a8a53fc9613ee0c8d7a8c5540085a14da9225ff0b09ef9deb"
        );
        assert_eq!(docs.history.len(), 1);
        assert_eq!(docs.history[0].attachments.len(), 1);
        let old = &docs.history[0].attachments[0];
        assert_eq!(old.name, "old.txt");
        assert_eq!(old.data, b"old");
        assert!(old.protected);
        let copy = vault
            .root
            .entries
            .iter()
            .find(|e| e.title() == "Copy")
            .unwrap();
        assert_eq!(copy.attachments.len(), 1);
        assert_eq!(copy.attachments[0].name, "same.txt");
        assert_eq!(copy.attachments[0].data, b"abc");
        assert!(copy.attachments[0].protected);
    }
    assert_attachment_fixture(&db);
    let reopened = Database::open(&db.save(&p).unwrap(), &p).unwrap();
    assert_attachment_fixture(&reopened);
    assert_eq!(reopened.root.entries, db.root.entries);
}
