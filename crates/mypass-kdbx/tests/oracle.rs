// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use keepass::DatabaseKey;
use mypass_kdbx::*;
use secrecy::SecretString;
#[test]
fn oracle_opens_vault_written_by_us() {
    for (cipher, compression) in [
        (Cipher::Aes256Cbc, Compression::Gzip),
        (Cipher::ChaCha20, Compression::None),
    ] {
        let mut db = Database::new("root");
        db.settings.cipher = cipher;
        db.settings.compression = compression;
        let mut g = Group::new("child", Timestamp(0));
        let mut e = Entry::new(Timestamp(1793750604));
        e.set_field("Title", "\u{411}\u{430}\u{43d}\u{43a}", false);
        e.set_field("UserName", "user", false);
        e.set_field("Password", "secret", true);
        e.attachments.push(Attachment {
            name: "file".into(),
            data: vec![1, 2, 255],
            protected: true,
        });
        g.entries.push(e);
        db.root.groups.push(g);
        let data = db.save(&SecretString::from("RoundTripPass123")).unwrap();
        let oracle = keepass::Database::open(
            &mut data.as_slice(),
            DatabaseKey::new().with_password("RoundTripPass123"),
        )
        .unwrap();
        let root = oracle.root();
        let group = root.groups().next().unwrap();
        assert_eq!(group.name, "child");
        let entry = group.entries().next().unwrap();
        assert_eq!(entry.get_title(), Some("\u{411}\u{430}\u{43d}\u{43a}"));
        assert_eq!(entry.get_username(), Some("user"));
        assert_eq!(entry.get_password(), Some("secret"));
        assert_eq!(
            entry.attachments().next().unwrap().data.get(),
            &vec![1, 2, 255]
        );
    }
}
#[test]
fn we_open_vault_written_by_oracle() {
    let mut oracle = keepass::Database::new();
    {
        let mut root = oracle.root_mut();
        root.name = "oracle".into();
        let mut e = root.add_entry();
        e.set_unprotected("Title", "independent");
        e.set_unprotected("UserName", "user");
        e.set_protected("Password", "secret");
        e.add_attachment("file", keepass::db::Value::unprotected(vec![7, 8, 9]));
    }
    let mut data = vec![];
    oracle
        .save(
            &mut data,
            DatabaseKey::new().with_password("oracle-password"),
        )
        .unwrap();
    let db = Database::open(&data, &SecretString::from("oracle-password")).unwrap();
    assert_eq!(db.root.name, "oracle");
    let e = &db.root.entries[0];
    assert_eq!(e.title(), "independent");
    assert_eq!(e.field("Password"), Some("secret"));
    assert_eq!(e.attachments[0].data, vec![7, 8, 9]);
}
