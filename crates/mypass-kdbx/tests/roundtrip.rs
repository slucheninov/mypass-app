// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_kdbx::*;
use secrecy::SecretString;
fn make_db() -> Database {
    let mut db = Database::new("root");
    db.root.created = Timestamp(0);
    db.root.modified = Timestamp(0);
    db.root.accessed = Timestamp(0);
    db.root.location_changed = Timestamp(0);
    db.settings.kdf = KdfParams::AesKdf {
        seed: vec![0; 32],
        rounds: 1,
    };
    db
}
#[test]
fn history_and_string_arrays_survive_round_trip() {
    let mut db = make_db();
    let mut e = Entry::new(Timestamp(1793750604));
    e.set_field("Title", "<>&\"'😀", false);
    e.set_field("Password", "sentinel-secret", true);
    e.string_arrays.push(StringArray {
        key: "array".into(),
        values: vec!["a".into(), "b".into()],
    });
    e.history.push(e.clone());
    db.root.entries.push(e);
    let p = SecretString::from("password");
    assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
}
#[test]
fn xml_invalid_control_chars_are_stripped_not_corrupting() {
    let mut db = make_db();
    let mut e = Entry::new(Timestamp(0));
    e.set_field("Title", "a\u{1}b", false);
    db.root.entries.push(e);
    let p = SecretString::from("password");
    let reopened = Database::open(&db.save(&p).unwrap(), &p).unwrap();
    assert_eq!(reopened.root.entries[0].title(), "ab");
}
#[test]
fn binary_all_byte_values_round_trip() {
    let mut db = make_db();
    let mut e = Entry::new(Timestamp(0));
    e.attachments.push(Attachment {
        name: "bytes".into(),
        data: (0..=255).collect(),
        protected: true,
    });
    db.root.entries.push(e);
    let p = SecretString::from("password");
    assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
}
#[test]
fn vault_without_attachments_round_trips() {
    let db = make_db();
    let p = SecretString::from("p");
    assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
}
#[test]
#[ignore]
fn fifty_megabyte_attachment_round_trips() {
    let mut db = make_db();
    let p = SecretString::from("p");
    let mut e = Entry::new(Timestamp(0));
    e.attachments.push(Attachment {
        name: "large".into(),
        data: vec![42; 50 * 1024 * 1024],
        protected: false,
    });
    db.root.entries.push(e);
    assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
}
#[test]
fn typed_metadata_custom_data_and_nested_history_round_trip() {
    let mut db = make_db();
    let p = SecretString::from("p");
    db.meta.generator = "MyPass test".into();
    db.meta.database_description = "description".into();
    db.meta.default_username = "default".into();
    db.meta.recycle_bin_uuid = Some(db.root.uuid);
    db.meta.recycle_bin_enabled = false;
    let item = CustomDataItem {
        key: "<key>".into(),
        value: "<&>😀".into(),
        last_modified: Some(Timestamp(1793750604)),
    };
    db.meta.custom_data.push(item.clone());
    db.root.custom_data.push(item.clone());
    db.root.expires = true;
    db.root.expiry_time = Some(Timestamp(3));
    db.deleted_objects.push(DeletedObject {
        uuid: db.root.uuid,
        deleted_at: Timestamp(4),
    });
    let mut deep = Entry::new(Timestamp(0));
    deep.custom_data.push(item);
    deep.expires = true;
    deep.expiry_time = Some(Timestamp(5));
    deep.usage_count = 8;
    deep.attachments.push(Attachment {
        name: "deep".into(),
        data: vec![9, 8, 7],
        protected: true,
    });
    let mut middle = Entry::new(Timestamp(1));
    middle.history.push(deep);
    let mut top = Entry::new(Timestamp(2));
    top.history.push(middle);
    db.root.entries.push(top);
    assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
}
