//! Authenticated KDBX open pipeline.
use crate::{
    format::{
        block_stream, cipher, compression, header_auth,
        inner_header::InnerHeader,
        inner_stream::InnerStream,
        kdf::{derive_keys, CompositeKey},
        outer_header::OuterHeader,
        reader::Reader,
        xml::decode,
    },
    *,
};
use secrecy::SecretString;
use std::fmt;
use zeroize::Zeroizing;
#[derive(Clone, Debug, PartialEq)]
pub struct SaveSettings {
    pub minor_version: u16,
    pub cipher: Cipher,
    pub compression: Compression,
    pub kdf: KdfParams,
}
#[derive(Clone)]
pub struct Database {
    pub root: Group,
    pub deleted_objects: Vec<DeletedObject>,
    pub meta: Meta,
    pub settings: SaveSettings,
    pub dropped_attachment_refs: usize,
    pub dropped_protected_unknown: usize,
}
impl PartialEq for Database {
    fn eq(&self, o: &Self) -> bool {
        self.root == o.root && self.deleted_objects == o.deleted_objects && self.meta == o.meta
    }
}
impl Database {
    pub fn new(name: &str) -> Database {
        Self {
            root: Group::new(name, Timestamp::now()),
            deleted_objects: vec![],
            meta: Meta {
                database_name: name.into(),
                ..Meta::default()
            },
            settings: SaveSettings {
                minor_version: 0,
                cipher: Cipher::Aes256Cbc,
                compression: Compression::Gzip,
                kdf: KdfParams::argon2id_default(),
            },
            dropped_attachment_refs: 0,
            dropped_protected_unknown: 0,
        }
    }

    pub fn save(&self, password: &SecretString) -> Result<Vec<u8>> {
        self.save_with_field_limit(password, u32::MAX as usize)
    }

    fn save_with_field_limit(&self, password: &SecretString, limit: usize) -> Result<Vec<u8>> {
        use crate::format::{
            inner_header::InnerStreamKind,
            kdf::fill_random,
            signature::Version,
            xml::{encode, pool::AttachmentPool, validate_document},
        };
        validate_model(&self.root, 3, limit)?;
        crate::format::xml::validate_extras(&self.meta.extra, 2)?;
        validate_times(
            self.deleted_objects
                .iter()
                .map(|d| d.deleted_at)
                .chain(self.meta.custom_data.iter().filter_map(|c| c.last_modified)),
        )?;
        let salt_len = match &self.settings.kdf {
            KdfParams::Argon2 { salt, .. } => salt.len(),
            KdfParams::AesKdf { seed, .. } => seed.len(),
        };
        check_field_len(salt_len, limit)?;
        // Generated dictionaries have fixed names and scalar values; account for the entire
        // containing field as well as its variable salt before allocating/serializing it.
        let overhead = match self.settings.kdf {
            KdfParams::Argon2 { .. } => 107,
            KdfParams::AesKdf { .. } => 61,
        };
        check_field_len(
            salt_len
                .checked_add(overhead)
                .ok_or_else(|| KdbxError::InvalidOperation("header field size".into()))?,
            limit,
        )?;
        for len in [32, self.settings.cipher.iv_len(), 64] {
            check_field_len(len, limit)?;
        }
        let kdf = self.settings.kdf.try_with_fresh_randomness()?;
        let mut master_seed = vec![0; 32];
        fill_random(&mut master_seed)?;
        let mut iv = vec![0; self.settings.cipher.iv_len()];
        fill_random(&mut iv)?;
        let mut inner_key = Zeroizing::new(vec![0; 64]);
        fill_random(&mut inner_key)?;
        let pool = AttachmentPool::collecting(&self.root);
        let mut stream = InnerStream::new(InnerStreamKind::ChaCha20, &inner_key)?;
        let xml = Zeroizing::new(encode::encode(self, &pool, &mut stream));
        validate_document(xml.as_bytes())?;
        if stream.is_failed() {
            return Err(KdbxError::InvalidOperation("inner stream exhausted".into()));
        }
        let transformed = kdf.transform(&CompositeKey::from_password(password))?;
        let keys = derive_keys(&master_seed, &transformed);
        let header = OuterHeader {
            version: Version {
                major: 4,
                minor: self.settings.minor_version,
            },
            cipher: self.settings.cipher,
            compression: self.settings.compression,
            master_seed,
            encryption_iv: iv,
            kdf_parameters: kdf.to_variant(),
            public_custom_data: None,
        };
        let mut out = header.serialize();
        let sha = header_auth::header_sha256(&out);
        let mac = header_auth::header_hmac(&out, &keys.hmac_key_base);
        out.extend(sha);
        out.extend(mac);
        let inner = InnerHeader {
            stream: InnerStreamKind::ChaCha20,
            stream_key: inner_key,
            binaries: pool.binaries,
        };
        let mut plain = Zeroizing::new(inner.serialize());
        plain.extend_from_slice(xml.as_bytes());
        if plain.len() as u64 > crate::format::limits::MAX_DECOMPRESSED_LEN {
            return Err(KdbxError::InvalidOperation("payload size".into()));
        }
        let compressed = Zeroizing::new(match self.settings.compression {
            Compression::None => plain.to_vec(),
            Compression::Gzip => compression::gzip(&plain)?,
        });
        let encrypted = cipher::encrypt(
            self.settings.cipher,
            &keys.master_key,
            &header.encryption_iv,
            &compressed,
        )?;
        out.extend(block_stream::write(&encrypted, &keys.hmac_key_base, 0));
        Ok(out)
    }

    pub fn open(data: &[u8], password: &SecretString) -> Result<Self> {
        let (header, offset) = OuterHeader::parse(data)?;
        let raw = data
            .get(..offset)
            .ok_or_else(|| KdbxError::Corrupt("header offset".into()))?;
        let mut reader = Reader::new(
            data.get(offset..)
                .ok_or_else(|| KdbxError::Corrupt("header offset".into()))?,
        );
        let sha = reader.bytes(32)?;
        let mac = reader.bytes(32)?;
        // Key-independent integrity and KDF limits precede any expensive derivation (R10).
        header_auth::verify_sha256(raw, sha)?;
        let kdf = KdfParams::from_variant(&header.kdf_parameters)?;
        let transformed = kdf.transform(&CompositeKey::from_password(password))?;
        let keys = derive_keys(&header.master_seed, &transformed);
        header_auth::verify_hmac(raw, &keys.hmac_key_base, mac)?;
        let blocks = reader.bytes(reader.remaining())?;
        // Authenticate every block before touching the outer cipher (no padding oracle).
        let encrypted = Zeroizing::new(block_stream::read(blocks, &keys.hmac_key_base)?);
        let decrypted = Zeroizing::new(cipher::decrypt(
            header.cipher,
            &keys.master_key,
            &header.encryption_iv,
            &encrypted,
        )?);
        let plain = Zeroizing::new(match header.compression {
            Compression::None => decrypted.to_vec(),
            Compression::Gzip => compression::gunzip(&decrypted)?,
        });
        let (inner, offset) = InnerHeader::parse(&plain)?;
        let mut stream = InnerStream::new(inner.stream, &inner.stream_key)?;
        let decoded = decode::decode(
            plain
                .get(offset..)
                .ok_or_else(|| KdbxError::Corrupt("inner offset".into()))?,
            &inner.binaries,
            &mut |v| stream.apply(v),
        )?;
        if stream.is_failed() {
            return Err(KdbxError::Corrupt("inner stream exhausted".into()));
        }
        Ok(Self {
            root: decoded.root,
            meta: decoded.meta,
            deleted_objects: decoded.deleted_objects,
            settings: SaveSettings {
                minor_version: header.version.minor,
                cipher: header.cipher,
                compression: header.compression,
                kdf,
            },
            dropped_attachment_refs: decoded.dropped_attachment_refs,
            dropped_protected_unknown: decoded.dropped_protected_unknown,
        })
    }
}
fn check_field_len(len: usize, limit: usize) -> Result<()> {
    if len > limit {
        return Err(KdbxError::InvalidOperation("header field size".into()));
    }
    Ok(())
}
fn validate_model(g: &Group, depth: usize, limit: usize) -> Result<()> {
    if depth > crate::format::limits::MAX_XML_DEPTH {
        return Err(KdbxError::InvalidOperation("XML depth".into()));
    }
    crate::format::xml::validate_extras(&g.extra, depth)?;
    validate_times(
        [g.created, g.modified, g.accessed, g.location_changed]
            .into_iter()
            .chain(g.expiry_time)
            .chain(g.custom_data.iter().filter_map(|c| c.last_modified)),
    )?;
    for e in &g.entries {
        validate_entry(e, depth + 1, limit)?;
    }
    for g in &g.groups {
        validate_model(g, depth + 1, limit)?;
    }
    Ok(())
}
fn validate_times(ts: impl Iterator<Item = Timestamp>) -> Result<()> {
    for t in ts {
        t.0.checked_add(Timestamp::EPOCH_OFFSET_SECS)
            .ok_or_else(|| KdbxError::InvalidOperation("timestamp overflow".into()))?;
    }
    Ok(())
}
fn validate_entry(e: &Entry, depth: usize, limit: usize) -> Result<()> {
    if depth > crate::format::limits::MAX_XML_DEPTH {
        return Err(KdbxError::InvalidOperation("XML depth".into()));
    }
    crate::format::xml::validate_extras(&e.extra, depth)?;
    validate_times(
        [e.created, e.modified, e.accessed, e.location_changed]
            .into_iter()
            .chain(e.expiry_time)
            .chain(e.custom_data.iter().filter_map(|c| c.last_modified)),
    )?;
    for a in &e.attachments {
        check_field_len(
            a.data
                .len()
                .checked_add(1)
                .ok_or_else(|| KdbxError::InvalidOperation("header field size".into()))?,
            limit,
        )?;
    }
    for e in &e.history {
        validate_entry(e, depth + 2, limit)?;
    }
    Ok(())
}
impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn counts(g: &Group) -> (usize, usize) {
            g.groups.iter().fold((g.entries.len(), 1), |(e, n), g| {
                let (a, b) = counts(g);
                (e + a, n + b)
            })
        }
        let (e, g) = counts(&self.root);
        // KDF Debug currently contains salts: format the parameter set explicitly without them.
        let kdf = match &self.settings.kdf {
            KdfParams::AesKdf { rounds, .. } => format!("AES-KDF rounds={rounds}"),
            KdfParams::Argon2 {
                variant, memory_bytes, iterations, parallelism, version, ..
            } => format!(
                "{variant:?} memory={memory_bytes} iterations={iterations} parallelism={parallelism} version={version}"
            ),
        };
        f.debug_struct("Database")
            .field("entries", &e)
            .field("groups", &g)
            .field("deleted_objects", &self.deleted_objects.len())
            .field("kdf", &kdf)
            .field("cipher", &self.settings.cipher)
            .field("compression", &self.settings.compression)
            .finish()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::signature::Version;
    fn make_vector(rounds: u64, bad_sha: bool) -> Vec<u8> {
        let kdf = KdfParams::AesKdf {
            seed: vec![4; 32],
            rounds,
        };
        let header = OuterHeader {
            version: Version { major: 4, minor: 1 },
            cipher: Cipher::Aes256Cbc,
            compression: Compression::None,
            master_seed: vec![5; 32],
            encryption_iv: vec![6; 16],
            kdf_parameters: kdf.to_variant(),
            public_custom_data: None,
        };
        let mut raw = header.serialize();
        let mut sha = header_auth::header_sha256(&raw);
        if bad_sha {
            sha[0] ^= 1;
        }
        let mac = if rounds == 1 {
            let transformed = kdf
                .transform(&CompositeKey::from_password(&SecretString::from(
                    "password",
                )))
                .unwrap();
            header_auth::header_hmac(
                &raw,
                &derive_keys(&header.master_seed, &transformed).hmac_key_base,
            )
        } else {
            [0; 32]
        };
        raw.extend(sha);
        raw.extend(mac);
        // Bad MAC and one-byte ciphertext: decrypting this first would report outer-cipher failure.
        raw.extend([0; 32]);
        raw.extend(1_u32.to_le_bytes());
        raw.push(0);
        raw
    }
    #[test]
    fn block_authentication_precedes_decryption_and_wrong_password_stops_at_header() {
        let wire = make_vector(1, false);
        let err = Database::open(&wire, &SecretString::from("password")).unwrap_err();
        assert!(matches!(err,KdbxError::Corrupt(ref s) if s.contains("HMAC")));
        assert_eq!(
            Database::open(&wire, &SecretString::from("nope")),
            Err(KdbxError::WrongPassword)
        );
    }
    #[test]
    fn header_sha_and_kdf_limits_precede_transform() {
        let rounds = 1_000_000_001;
        let result = Database::open(&make_vector(rounds, true), &SecretString::from("x"));
        assert!(matches!(result, Err(KdbxError::Corrupt(ref s)) if s == "header checksum"));
        assert!(matches!(
            Database::open(&make_vector(rounds, false), &SecretString::from("x")),
            Err(KdbxError::InvalidKdfParameters(_))
        ));
    }
    #[test]
    fn trailing_data_after_authenticated_terminator_is_corrupt() {
        let data = include_bytes!("../tests/fixtures/aeskdf-aes256-none.kdbx");
        let mut wire = data.to_vec();
        wire.push(0);
        let result = Database::open(&wire, &SecretString::from("TestMasterPass123"));
        assert!(matches!(result, Err(KdbxError::Corrupt(ref s)) if s == "trailing block data"));
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;
    use crate::format::{inner_header::InnerHeader, signature::Version};
    fn make_db() -> Database {
        let mut db = Database::new("root");
        db.settings.kdf = KdfParams::AesKdf {
            seed: vec![0; 32],
            rounds: 1,
        };
        db
    }
    fn parse_headers(data: &[u8], p: &SecretString) -> (OuterHeader, InnerHeader) {
        let (h, n) = OuterHeader::parse(data).unwrap();
        let kdf = KdfParams::from_variant(&h.kdf_parameters).unwrap();
        let t = kdf.transform(&CompositeKey::from_password(p)).unwrap();
        let keys = derive_keys(&h.master_seed, &t);
        let encrypted = block_stream::read(&data[n + 64..], &keys.hmac_key_base).unwrap();
        let plain =
            cipher::decrypt(h.cipher, &keys.master_key, &h.encryption_iv, &encrypted).unwrap();
        let plain = match h.compression {
            Compression::None => plain,
            Compression::Gzip => compression::gunzip(&plain).unwrap(),
        };
        (h, InnerHeader::parse(&plain).unwrap().0)
    }
    #[test]
    fn every_save_uses_fresh_randomness() {
        let db = make_db();
        let p = SecretString::from("p");
        let (a, ai) = parse_headers(&db.save(&p).unwrap(), &p);
        let (b, bi) = parse_headers(&db.save(&p).unwrap(), &p);
        assert_ne!(a.master_seed, b.master_seed);
        assert_ne!(a.encryption_iv, b.encryption_iv);
        assert_ne!(a.kdf_parameters, b.kdf_parameters);
        assert_ne!(ai.stream_key, bi.stream_key);
    }
    #[test]
    fn new_database_writes_documented_kdf_defaults() {
        let db = Database::new("new");
        let p = SecretString::from("p");
        let data = db.save(&p).unwrap();
        // Independent wire scan: do not use the production header/dictionary parsers here.
        assert_eq!(&data[8..12], &[0, 0, 4, 0]);
        let mut fields = std::collections::HashMap::new();
        let mut offset = 12;
        loop {
            let id = data[offset];
            let len = u32::from_le_bytes(data[offset + 1..offset + 5].try_into().unwrap()) as usize;
            offset += 5;
            let value = &data[offset..offset + len];
            offset += len;
            fields.insert(id, value);
            if id == 0 {
                break;
            }
        }
        assert_eq!(
            fields[&2],
            &[
                0x31, 0xc1, 0xf2, 0xe6, 0xbf, 0x71, 0x43, 0x50, 0xbe, 0x58, 0x05, 0x21, 0x6a, 0xfc,
                0x5a, 0xff
            ]
        );
        assert_eq!(fields[&3], &1u32.to_le_bytes());
        let dict = fields[&11];
        assert_eq!(&dict[..2], &[0, 1]);
        let mut offset = 2;
        let mut entries = std::collections::HashMap::new();
        while dict[offset] != 0 {
            let tag = dict[offset];
            offset += 1;
            let key_len = u32::from_le_bytes(dict[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            let key = &dict[offset..offset + key_len];
            offset += key_len;
            let len = u32::from_le_bytes(dict[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            entries.insert(key, (tag, &dict[offset..offset + len]));
            offset += len;
        }
        assert_eq!(
            entries[b"$UUID".as_slice()],
            (
                0x42,
                [
                    0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6,
                    0xf0, 0xa1, 0xe6
                ]
                .as_slice()
            )
        );
        for (key, expected) in [(b"M", 64 * 1024 * 1024u64), (b"I", 3u64)] {
            assert_eq!(
                entries[key.as_slice()],
                (0x05, expected.to_le_bytes().as_slice())
            );
        }
        for (key, expected) in [(b"P", 2u32), (b"V", 0x13u32)] {
            assert_eq!(
                entries[key.as_slice()],
                (0x04, expected.to_le_bytes().as_slice())
            );
        }
        let (salt_tag, salt) = entries[b"S".as_slice()];
        assert_eq!(salt_tag, 0x42);
        assert_eq!(salt.len(), 32);
        let (h, _) = OuterHeader::parse(&data).unwrap();
        assert_eq!(h.version, Version { major: 4, minor: 0 });
        assert_eq!(h.cipher, Cipher::Aes256Cbc);
        assert_eq!(h.compression, Compression::Gzip);
        match KdfParams::from_variant(&h.kdf_parameters).unwrap() {
            KdfParams::Argon2 {
                variant,
                salt,
                memory_bytes,
                iterations,
                parallelism,
                version,
            } => {
                assert_eq!(variant, Argon2Variant::Id);
                assert_eq!(salt.len(), 32);
                assert_eq!(memory_bytes, 64 * 1024 * 1024);
                assert_eq!(iterations, 3);
                assert_eq!(parallelism, 2);
                assert_eq!(version, 0x13);
            }
            _ => panic!("not Argon2"),
        }
    }
    #[test]
    fn header_field_size_is_checked_before_serialization() {
        let mut db = make_db();
        let p = SecretString::from("p");
        let mut e = Entry::new(Timestamp(0));
        e.attachments.push(Attachment {
            name: "large".into(),
            data: vec![7; 128],
            protected: false,
        });
        db.root.entries.push(e);
        assert!(db.save_with_field_limit(&p, 129).is_ok());
        assert!(
            matches!(db.save_with_field_limit(&p,128),Err(KdbxError::InvalidOperation(ref s)) if s=="header field size")
        );
    }
    #[test]
    fn kdf_dictionary_container_size_is_checked() {
        let db = make_db();
        let p = SecretString::from("p");
        assert!(db.save_with_field_limit(&p, 93).is_ok());
        assert!(
            matches!(db.save_with_field_limit(&p,92),Err(KdbxError::InvalidOperation(ref s)) if s=="header field size")
        );
    }
    #[test]
    fn unrepresentable_time_fails_to_save() {
        let mut db = make_db();
        db.root.created = Timestamp(i64::MAX);
        assert!(
            matches!(db.save(&SecretString::from("p")),Err(KdbxError::InvalidOperation(ref s)) if s=="timestamp overflow")
        );
    }
    #[test]
    fn exact_xml_depth_round_trips_one_more_fails_to_save() {
        let p = SecretString::from("p");
        // KeePassFile/Root + 93 Groups + Entry/History/Entry/String/Value = 100.
        fn make_chain(count: usize) -> Group {
            let mut g = Group::new("leaf", Timestamp(0));
            let mut e = Entry::new(Timestamp(0));
            e.set_field("Title", "leaf", false);
            let mut outer = Entry::new(Timestamp(0));
            outer.history.push(e);
            g.entries.push(outer);
            for _ in 1..count {
                let mut parent = Group::new("parent", Timestamp(0));
                parent.groups.push(g);
                g = parent;
            }
            g
        }
        let mut db = make_db();
        db.root = make_chain(93);
        let wire = db.save(&p).unwrap();
        assert_eq!(Database::open(&wire, &p).unwrap(), db);
        db.root = make_chain(94);
        assert!(matches!(db.save(&p),Err(KdbxError::InvalidOperation(ref s)) if s=="XML depth"));
    }
    #[test]
    fn unsafe_raw_extras_are_rejected_before_encoding() {
        for fragment in [
            "<Foo Protected=\"True\">AAAAAAAAAAAAAAAAAAAAAA==</Foo>",
            "<Foo><Bar Protected='Tr&#117;e'/></Foo>",
            "</Meta><Meta>",
            "<A>",
            "</A>",
            "<A></B>",
            "text only",
            "<!DOCTYPE A><A/>",
            "<A/><B>",
        ] {
            for location in 0..3 {
                let mut db = make_db();
                let mut e = Entry::new(Timestamp(0));
                e.set_field("Password", "protected secret", true);
                db.root.entries.push(e);
                let extra = RawXml(fragment.into());
                match location {
                    0 => db.meta.extra.push(extra),
                    1 => db.root.extra.push(extra),
                    _ => db.root.entries[0].extra.push(extra),
                }
                assert_eq!(
                    db.save(&SecretString::from("p")),
                    Err(KdbxError::InvalidOperation("unsafe raw XML extra".into())),
                    "location {location}, fragment {fragment}"
                );
            }
        }
    }
    #[test]
    fn benign_raw_extras_round_trip_byte_equal_in_all_locations() {
        let fragment = RawXml("<Extension a='a&amp;b'><![CDATA[<>&]]><!--ok--><Nested Protected=\"False\" /></Extension>".into());
        let mut db = make_db();
        db.meta.extra.push(fragment.clone());
        db.root.extra.push(fragment.clone());
        let mut e = Entry::new(Timestamp(0));
        e.extra.push(fragment);
        e.set_field("Password", "protected secret", true);
        db.root.entries.push(e);
        let p = SecretString::from("p");
        assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
    }
    #[test]
    fn raw_extra_depth_includes_its_containing_element() {
        let mut db = make_db();
        db.root
            .extra
            .push(RawXml(format!("{}{}", "<X>".repeat(97), "</X>".repeat(97))));
        let p = SecretString::from("p");
        assert!(db.save(&p).is_ok());
        db.root.extra[0] = RawXml(format!("{}{}", "<X>".repeat(98), "</X>".repeat(98)));
        assert_eq!(
            db.save(&p),
            Err(KdbxError::InvalidOperation("unsafe raw XML extra".into()))
        );
    }
    #[test]
    fn unknown_keepassxc_elements_survive_round_trip() {
        let mut db = make_db();
        let mut e = Entry::new(Timestamp(0));
        db.meta.extra.push(RawXml(
            "<Extension attr = 'a&amp;b'><Nested /></Extension>".into(),
        ));
        e.extra.push(RawXml(
            "<AutoType><Enabled>True</Enabled></AutoType>".into(),
        ));
        db.root.entries.push(e);
        let xml = b"<KeePassFile><Meta><Extension attr = 'a&amp;b'><Nested /></Extension></Meta><Root><Group><UUID>AAAAAAAAAAAAAAAAAAAAAA==</UUID><Name>root</Name><Entry><UUID>AQAAAAAAAAAAAAAAAAAAAA==</UUID><AutoType><Enabled>True</Enabled></AutoType></Entry></Group></Root></KeePassFile>";
        let decoded = decode::decode(xml, &[], &mut |_| {}).unwrap();
        db.meta = decoded.meta;
        db.root = decoded.root;
        assert_eq!(
            db.meta.extra,
            vec![RawXml(
                "<Extension attr = 'a&amp;b'><Nested /></Extension>".into()
            )]
        );
        assert_eq!(
            db.root.entries[0].extra,
            vec![RawXml(
                "<AutoType><Enabled>True</Enabled></AutoType>".into()
            )]
        );
        let p = SecretString::from("p");
        assert_eq!(Database::open(&db.save(&p).unwrap(), &p).unwrap(), db);
    }
}
