//! Logical vault content. Debug implementations deliberately omit all values.
use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);
impl Timestamp {
    pub const EPOCH_OFFSET_SECS: i64 = 62_135_596_800;
    pub fn now() -> Self {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => Self(i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
            Err(e) => Self(-i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX)),
        }
    }
    pub fn from_kdbx_seconds(s: i64) -> Option<Self> {
        s.checked_sub(Self::EPOCH_OFFSET_SECS).map(Self)
    }
    /// Saturates for caller-created timestamps outside the representable KDBX range.
    pub fn to_kdbx_seconds(self) -> i64 {
        self.0.saturating_add(Self::EPOCH_OFFSET_SECS)
    }
}
#[derive(Clone, PartialEq)]
pub struct Field {
    pub key: String,
    pub value: String,
    pub protected: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct StringArray {
    pub key: String,
    pub values: Vec<String>,
}
#[derive(Clone, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub data: Vec<u8>,
    pub protected: bool,
}
#[derive(Clone, PartialEq)]
pub struct CustomDataItem {
    pub key: String,
    pub value: String,
    pub last_modified: Option<Timestamp>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RawXml(pub String);
#[derive(Clone, PartialEq)]
pub struct Entry {
    pub uuid: Uuid,
    pub icon_id: u32,
    pub fields: Vec<Field>,
    pub string_arrays: Vec<StringArray>,
    pub attachments: Vec<Attachment>,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub accessed: Timestamp,
    pub location_changed: Timestamp,
    pub expires: bool,
    pub expiry_time: Option<Timestamp>,
    pub usage_count: u32,
    pub custom_data: Vec<CustomDataItem>,
    pub history: Vec<Entry>,
    pub extra: Vec<RawXml>,
}
impl Entry {
    pub fn new(now: Timestamp) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            icon_id: 0,
            fields: vec![],
            string_arrays: vec![],
            attachments: vec![],
            created: now,
            modified: now,
            accessed: now,
            location_changed: now,
            expires: false,
            expiry_time: None,
            usage_count: 0,
            custom_data: vec![],
            history: vec![],
            extra: vec![],
        }
    }
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.as_str())
    }
    pub fn set_field(&mut self, key: &str, value: &str, protected: bool) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            f.value = value.into();
            f.protected = protected;
        } else {
            self.fields.push(Field {
                key: key.into(),
                value: value.into(),
                protected,
            });
        }
    }
    pub fn remove_field(&mut self, key: &str) {
        self.fields.retain(|f| f.key != key);
    }
    pub fn title(&self) -> &str {
        self.field("Title").unwrap_or("")
    }
    pub fn content_eq(&self, o: &Entry) -> bool {
        self.uuid == o.uuid
            && self.icon_id == o.icon_id
            && self.fields == o.fields
            && self.string_arrays == o.string_arrays
            && self.attachments == o.attachments
            && self.expires == o.expires
            && self.expiry_time == o.expiry_time
            && self.custom_data == o.custom_data
            && self.extra == o.extra
    }
}
#[derive(Clone, PartialEq)]
pub struct Group {
    pub uuid: Uuid,
    pub name: String,
    pub icon_id: u32,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub accessed: Timestamp,
    pub location_changed: Timestamp,
    pub expires: bool,
    pub expiry_time: Option<Timestamp>,
    pub entries: Vec<Entry>,
    pub groups: Vec<Group>,
    pub custom_data: Vec<CustomDataItem>,
    pub extra: Vec<RawXml>,
}
impl Group {
    pub fn new(name: &str, now: Timestamp) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name: name.into(),
            icon_id: 0,
            created: now,
            modified: now,
            accessed: now,
            location_changed: now,
            expires: false,
            expiry_time: None,
            entries: vec![],
            groups: vec![],
            custom_data: vec![],
            extra: vec![],
        }
    }
    pub fn custom(&self, key: &str) -> Option<&str> {
        self.custom_data
            .iter()
            .find(|c| c.key == key)
            .map(|c| c.value.as_str())
    }
    pub fn set_custom(&mut self, key: &str, value: &str) {
        if let Some(c) = self.custom_data.iter_mut().find(|c| c.key == key) {
            c.value = value.into();
        } else {
            self.custom_data.push(CustomDataItem {
                key: key.into(),
                value: value.into(),
                last_modified: None,
            });
        }
    }
    pub fn remove_custom(&mut self, key: &str) {
        self.custom_data.retain(|c| c.key != key);
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct DeletedObject {
    pub uuid: Uuid,
    pub deleted_at: Timestamp,
}
#[derive(Clone, PartialEq)]
pub struct Meta {
    pub generator: String,
    pub database_name: String,
    pub database_description: String,
    pub default_username: String,
    pub history_max_items: i32,
    pub history_max_size: i64,
    pub recycle_bin_enabled: bool,
    pub recycle_bin_uuid: Option<Uuid>,
    pub custom_data: Vec<CustomDataItem>,
    pub extra: Vec<RawXml>,
}
impl Default for Meta {
    fn default() -> Self {
        Self {
            generator: String::new(),
            database_name: String::new(),
            database_description: String::new(),
            default_username: String::new(),
            history_max_items: 10,
            history_max_size: 6_291_456,
            recycle_bin_enabled: true,
            recycle_bin_uuid: None,
            custom_data: vec![],
            extra: vec![],
        }
    }
}
impl fmt::Debug for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Field")
            .field("key", &self.key)
            .field("protected", &self.protected)
            .field("value_len", &self.value.len())
            .finish()
    }
}
impl fmt::Debug for Attachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachment")
            .field("name", &self.name)
            .field("protected", &self.protected)
            .field("len", &self.data.len())
            .finish()
    }
}
impl fmt::Debug for CustomDataItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomDataItem")
            .field("key", &self.key)
            .field("value_len", &self.value.len())
            .finish()
    }
}
impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("uuid", &self.uuid)
            .field(
                "field_keys",
                &self.fields.iter().map(|v| &v.key).collect::<Vec<_>>(),
            )
            .field(
                "attachments",
                &self
                    .attachments
                    .iter()
                    .map(|a| (&a.name, a.data.len()))
                    .collect::<Vec<_>>(),
            )
            .field("history_len", &self.history.len())
            .finish()
    }
}
impl fmt::Debug for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Group")
            .field("uuid", &self.uuid)
            .field("name_len", &self.name.len())
            .field("entries", &self.entries.len())
            .field("groups", &self.groups.len())
            .finish()
    }
}
impl fmt::Debug for Meta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Meta")
            .field("database_name_len", &self.database_name.len())
            .field("custom_data_len", &self.custom_data.len())
            .field("extra_len", &self.extra.len())
            .finish()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cipher, Compression, Database, KdfParams, SaveSettings};
    fn make_db() -> Database {
        let mut e = Entry::new(Timestamp(0));
        e.set_field("Password", "hunter2-secret", true);
        e.set_field("custom", "plain-secret", false);
        e.set_field("Notes", "notes-secret", false);
        e.attachments.push(Attachment {
            name: "file".into(),
            data: vec![0x53, 0x45, 0x43],
            protected: true,
        });
        e.custom_data.push(CustomDataItem {
            key: "key".into(),
            value: "custom-secret".into(),
            last_modified: None,
        });
        let mut root = Group::new("group-secret", Timestamp(0));
        root.entries.push(e);
        let meta = Meta {
            database_name: "meta-secret".into(),
            ..Meta::default()
        };
        Database {
            root,
            meta,
            deleted_objects: vec![],
            settings: SaveSettings {
                minor_version: 1,
                cipher: Cipher::Aes256Cbc,
                compression: Compression::Gzip,
                kdf: KdfParams::AesKdf {
                    seed: vec![0; 32],
                    rounds: 1,
                },
            },
            dropped_attachment_refs: 0,
            dropped_protected_unknown: 0,
        }
    }
    #[test]
    fn debug_output_contains_no_secret() {
        let db = make_db();
        let e = &db.root.entries[0];
        let objects: Vec<&dyn fmt::Debug> = vec![
            &e.fields[0],
            &e.fields[1],
            &e.fields[2],
            &e.attachments[0],
            e,
            &db,
            &db.meta,
            &db.root,
            &e.custom_data[0],
        ];
        for obj in objects {
            for rendered in [format!("{obj:?}"), format!("{obj:#?}")] {
                for secret in [
                    "hunter2-secret",
                    "plain-secret",
                    "notes-secret",
                    "custom-secret",
                    "meta-secret",
                    "group-secret",
                    "83, 69, 67",
                    "83,\n",
                    "69,\n",
                ] {
                    assert!(!rendered.contains(secret), "{rendered}");
                }
            }
        }
        assert!(format!("{:?}", e.fields[0]).contains("Password"));
        assert!(format!("{db:?}").contains("entries: 1"));
    }
    #[test]
    fn database_and_save_settings_are_clone() {
        let db = make_db();
        assert_eq!(db.clone(), db);
        assert_eq!(db.settings.clone(), db.settings);
        let mut other = db.clone();
        other.dropped_attachment_refs = 123;
        other.settings.minor_version = 99;
        assert_eq!(other, db);
    }
    #[test]
    fn timestamp_epoch_offset() {
        assert_eq!(
            Timestamp::from_kdbx_seconds(62_135_596_800),
            Some(Timestamp(0))
        );
        assert_eq!(Timestamp(0).to_kdbx_seconds(), 62_135_596_800);
        assert_eq!(Timestamp::from_kdbx_seconds(i64::MIN), None);
    }
    #[test]
    fn entry_content_eq_ignores_times_and_history() {
        let a = Entry::new(Timestamp(0));
        let mut b = a.clone();
        b.created = Timestamp(1);
        b.modified = Timestamp(2);
        b.accessed = Timestamp(3);
        b.location_changed = Timestamp(4);
        b.usage_count = 2;
        b.history.push(a.clone());
        assert!(a.content_eq(&b));
        b.set_field("Title", "changed", false);
        assert!(!a.content_eq(&b));
    }
    #[test]
    fn group_custom_set_remove() {
        let mut g = Group::new("", Timestamp(0));
        g.set_custom("a", "one");
        g.set_custom("a", "two");
        assert_eq!(g.custom_data.len(), 1);
        assert_eq!(g.custom("a"), Some("two"));
        g.remove_custom("a");
        assert_eq!(g.custom("a"), None);
    }
    #[test]
    fn entry_set_field_replaces_and_keeps_order() {
        let mut e = Entry::new(Timestamp(0));
        e.set_field("a", "one", false);
        e.set_field("b", "two", false);
        e.set_field("a", "three", true);
        assert_eq!(
            e.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(e.field("a"), Some("three"));
        assert!(e.fields[0].protected);
        e.remove_field("a");
        assert_eq!(e.field("a"), None);
    }
}
