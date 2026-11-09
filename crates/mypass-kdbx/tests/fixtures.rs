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
