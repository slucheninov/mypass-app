// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_kdbx::*;
use secrecy::SecretString;
#[test]
fn cyrillic_master_password_and_fields_round_trip() {
    for cipher in [Cipher::Aes256Cbc, Cipher::ChaCha20] {
        let mut db = Database::new("\u{433}\u{440}\u{443}\u{43f}\u{430}");
        db.settings.kdf = KdfParams::AesKdf {
            seed: vec![0; 32],
            rounds: 1,
        };
        db.settings.cipher = cipher;
        let mut e = Entry::new(Timestamp(0));
        e.set_field("Title", "\u{411}\u{430}\u{43d}\u{43a}", false);
        e.set_field("Notes", "\u{411}\n\u{430}", false);
        e.set_field("\u{43f}\u{43e}\u{43b}\u{435}", "value", true);
        e.attachments.push(Attachment {
            name: "\u{444}\u{430}\u{439}\u{43b}.txt".into(),
            data: vec![1, 2],
            protected: true,
        });
        db.root.entries.push(e);
        let p = SecretString::from("\u{43f}\u{430}\u{440}\u{43e}\u{43b}\u{44c}");
        assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
    }
}
#[test]
fn nfd_password_does_not_open_nfc_vault() {
    let mut db = Database::new("nfc");
    db.settings.kdf = KdfParams::AesKdf {
        seed: vec![0; 32],
        rounds: 1,
    };
    let data = db.save(&SecretString::from("\u{e9}")).unwrap();
    assert_eq!(
        Database::open(&data, &SecretString::from("e\u{301}")),
        Err(KdbxError::WrongPassword)
    );
}
