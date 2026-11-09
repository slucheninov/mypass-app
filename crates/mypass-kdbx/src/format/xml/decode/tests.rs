use super::*;
use crate::format::{inner_header::InnerStreamKind, inner_stream::InnerStream};
fn make_xml(body: &str) -> String {
    format!("<KeePassFile><Meta/><Root><Group>{body}</Group></Root></KeePassFile>")
}
fn read(s: &str) -> Result<Decoded> {
    decode(s.as_bytes(), &[], &mut |_| {})
}
fn assert_corrupt(s: &str) {
    assert!(matches!(read(s), Err(KdbxError::Corrupt(_))));
}
#[test]
fn malformed_xml_is_corrupt() {
    assert_corrupt("<KeePassFile><Root></KeePassFile>");
}
#[test]
fn missing_root_is_corrupt() {
    assert_corrupt("<KeePassFile><Meta/></KeePassFile>");
}
#[test]
fn doctype_external_entity_rejected() {
    let s = format!(
        "<!DOCTYPE KeePassFile SYSTEM 'file:///does-not-exist'>{}",
        make_xml("")
    );
    assert_corrupt(&s);
    assert!(read(&make_xml("")).is_ok());
}
#[test]
fn billion_laughs_rejected() {
    assert_corrupt(&format!(
        "<!DOCTYPE KeePassFile [<!ENTITY a 'ha'><!ENTITY b '&a;&a;'>]>{}",
        make_xml("")
    ));
}
#[test]
fn utf16_payload_rejected() {
    for bom in [false, true] {
        let mut b = if bom { vec![0xff, 0xfe] } else { vec![] };
        b.extend(make_xml("").encode_utf16().flat_map(u16::to_le_bytes));
        assert!(decode(&b, &[], &mut |_| {}).is_err());
    }
    assert_corrupt(&format!(
        "<?xml version='1.0' encoding='UTF-16'?>{}",
        make_xml("")
    ));
}
#[test]
fn utf7_declared_payload_rejected() {
    assert_corrupt(&format!(
        "<?xml version='1.0' encoding='UTF-7'?>{}",
        make_xml("")
    ));
}
#[test]
fn deeply_nested_groups_rejected_not_stack_overflow() {
    assert_corrupt(&make_xml(&format!(
        "{}{}",
        "<Group>".repeat(10000),
        "</Group>".repeat(10000)
    )));
}
#[test]
fn uuid_is_base64_of_16_raw_bytes() {
    let d = read(&make_xml("<UUID>AAECAwQFBgcICQoLDA0ODw==</UUID>")).unwrap();
    assert_eq!(
        d.root.uuid.as_bytes(),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    assert_corrupt(&make_xml("<UUID>AA==</UUID>"));
}
#[test]
fn times_are_base64_int64_seconds_since_0001() {
    let d = read(&make_xml(
        "<Times><CreationTime>DzwOAAAAAAA=</CreationTime></Times>",
    ))
    .unwrap();
    assert_eq!(
        d.root.created,
        Timestamp(932879 - Timestamp::EPOCH_OFFSET_SECS)
    );
}
#[test]
fn iso8601_times_accepted() {
    let d = read(&make_xml(
        "<Times><CreationTime>1970-01-01T00:00:00Z</CreationTime></Times>",
    ))
    .unwrap();
    assert_eq!(d.root.created, Timestamp(0));
    for s in [
        "2001-02-29T00:00:00Z",
        "2000-13-01T00:00:00Z",
        "2000-01-32T00:00:00Z",
        "2000-01-01T24:00:00Z",
        "2000-01-01T00:60:00Z",
        "2000-01-01T00:00:61Z",
    ] {
        assert_corrupt(&make_xml(&format!(
            "<Times><CreationTime>{s}</CreationTime></Times>"
        )));
    }
}
#[test]
fn date_near_i64_min_is_corrupt_not_crash() {
    assert_corrupt(&make_xml(
        "<Times><CreationTime>AAAAAAAAAIA=</CreationTime></Times>",
    ));
}
fn make_protected(unknown: bool) -> (String, InnerStream) {
    let mut stream = InnerStream::new(InnerStreamKind::ChaCha20, &[7; 64]).unwrap();
    let mut a = b"first".to_vec();
    let mut b = b"second".to_vec();
    stream.apply(&mut a);
    stream.apply(&mut b);
    let first = if unknown {
        format!(
            "<Foo><Bar Protected=\"True\">{}</Bar></Foo>",
            BASE64_STANDARD.encode(a)
        )
    } else {
        format!(
            "<Entry><String><Key>A</Key><Value Protected=\"True\">{}</Value></String></Entry>",
            BASE64_STANDARD.encode(a)
        )
    };
    (make_xml(&format!("{first}<Entry><String><Key>Password</Key><Value Protected=\"True\">{}</Value></String></Entry>",BASE64_STANDARD.encode(b))),InnerStream::new(InnerStreamKind::ChaCha20,&[7;64]).unwrap())
}
#[test]
fn protected_value_decrypted_in_document_order() {
    let (s, mut st) = make_protected(false);
    let d = decode(s.as_bytes(), &[], &mut |v| st.apply(v)).unwrap();
    assert_eq!(d.root.entries[0].field("A"), Some("first"));
    assert_eq!(d.root.entries[1].field("Password"), Some("second"));
}
#[test]
fn unknown_protected_element_dropped_but_consumes_keystream() {
    let (s, mut st) = make_protected(true);
    let d = decode(s.as_bytes(), &[], &mut |v| st.apply(v)).unwrap();
    assert_eq!(d.root.entries[0].field("Password"), Some("second"));
    assert_eq!(d.dropped_protected_unknown, 1);
    assert!(d.root.extra.is_empty());
}
#[test]
fn unknown_meta_and_entry_elements_preserved_verbatim() {
    let s="<KeePassFile><Meta><MemoryProtection><ProtectPassword>True</ProtectPassword></MemoryProtection><CustomIcons/></Meta><Root><Group><Entry><Tags>a;b</Tags><ForegroundColor>#ffffff</ForegroundColor><AutoType><Enabled>True</Enabled></AutoType></Entry></Group></Root></KeePassFile>";
    let d = read(s).unwrap();
    assert_eq!(
        d.meta.extra,
        vec![
            RawXml(
                "<MemoryProtection><ProtectPassword>True</ProtectPassword></MemoryProtection>"
                    .into()
            ),
            RawXml("<CustomIcons/>".into())
        ]
    );
    assert_eq!(
        d.root.entries[0].extra,
        vec![
            RawXml("<Tags>a;b</Tags>".into()),
            RawXml("<ForegroundColor>#ffffff</ForegroundColor>".into()),
            RawXml("<AutoType><Enabled>True</Enabled></AutoType>".into())
        ]
    );
}
#[test]
fn dangling_and_non_numeric_binary_refs_skipped_and_counted() {
    let d=read(&make_xml("<Entry><Binary><Key>a</Key><Value Ref='0'/></Binary><Binary><Value Ref='no'/></Binary></Entry>")).unwrap();
    assert_eq!(d.dropped_attachment_refs, 2);
    assert!(d.root.entries[0].attachments.is_empty());
}
#[test]
fn duplicate_attachment_names_within_entry_preserved() {
    let s=make_xml("<Entry><Binary><Key>x</Key><Value Ref='0'/></Binary><Binary><Key>x</Key><Value Ref='0'/></Binary></Entry>");
    let d = decode(
        s.as_bytes(),
        &[InnerBinary {
            data: b"abc".to_vec(),
            protected: true,
        }],
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(d.root.entries[0].attachments.len(), 2);
    assert_eq!(
        d.root.entries[0].attachments[0],
        d.root.entries[0].attachments[1]
    );
}
#[test]
fn attachment_name_with_xml_specials_and_unicode() {
    let s = make_xml("<Entry><Binary><Key>&lt;&amp;\u{43f}</Key><Value Ref='0'/></Binary></Entry>");
    let d = decode(
        s.as_bytes(),
        &[InnerBinary {
            data: vec![],
            protected: false,
        }],
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(d.root.entries[0].attachments[0].name, "<&\u{43f}");
}
#[test]
fn history_nested_decodes() {
    let d = read(&make_xml(
        "<Entry><History><Entry><History><Entry/></History></Entry></History></Entry>",
    ))
    .unwrap();
    assert_eq!(d.root.entries[0].history[0].history.len(), 1);
}
#[test]
fn recycle_bin_uuid_present_absent_and_malformed() {
    for (s, expected) in [
        ("", None),
        ("<RecycleBinUUID>bad</RecycleBinUUID>", None),
        (
            "<RecycleBinUUID>AAAAAAAAAAAAAAAAAAAAAA==</RecycleBinUUID>",
            None,
        ),
        (
            "<RecycleBinUUID>AAECAwQFBgcICQoLDA0ODw==</RecycleBinUUID>",
            Some(Uuid::from_bytes([
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
            ])),
        ),
    ] {
        let d = read(&format!(
            "<KeePassFile><Meta>{s}</Meta><Root><Group/></Root></KeePassFile>"
        ))
        .unwrap();
        assert_eq!(d.meta.recycle_bin_uuid, expected);
    }
}
#[test]
fn group_custom_data_and_item_last_modified() {
    let d=read(&make_xml("<CustomData><Item><Key>x</Key><Value>y</Value><LastModificationTime>1970-01-01T00:00:00Z</LastModificationTime></Item></CustomData>")).unwrap();
    assert_eq!(d.root.custom("x"), Some("y"));
    assert_eq!(d.root.custom_data[0].last_modified, Some(Timestamp(0)));
}
#[test]
fn string_arrays_and_plain_kp2a_field() {
    let d=read(&make_xml("<Entry><String><Key>KP2A_URL</Key><Value>plain</Value></String><StringArray><Key>KP2A_URL</Key><Value>a</Value><Value>b</Value></StringArray></Entry>")).unwrap();
    assert_eq!(d.root.entries[0].field("KP2A_URL"), Some("plain"));
    assert_eq!(
        d.root.entries[0].string_arrays,
        vec![StringArray {
            key: "KP2A_URL".into(),
            values: vec!["a".into(), "b".into()]
        }]
    );
}
#[test]
fn protected_base64_invalid_is_corrupt() {
    assert_corrupt(&make_xml(
        "<Entry><String><Value Protected='True'>!!!</Value></String></Entry>",
    ));
}
#[test]
fn depth_boundary_is_enforced_before_model_recursion() {
    let nested = |n| make_xml(&format!("{}{}", "<Group>".repeat(n), "</Group>".repeat(n)));
    assert!(read(&nested(97)).is_ok()); // KeePassFile + Root + Group + 97 groups = 100.
    assert!(matches!(read(&nested(98)),Err(KdbxError::Corrupt(ref s)) if s=="XML depth"));
}
#[test]
fn bomless_utf16_rejected_by_encoding_check() {
    let bytes: Vec<u8> = make_xml("")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(
        matches!(decode(&bytes,&[],&mut |_|{}),Err(KdbxError::Corrupt(ref s)) if s=="XML UTF-8")
    );
}
#[test]
fn literal_inner_keystream_ciphertext_recovers_plaintext() {
    // libsodium-derived first keystream bytes 8c e8 bc 61 0a XOR ASCII "first".
    let xml=make_xml("<Entry><String><Key>Password</Key><Value Protected='True'>6oHOEn4=</Value></String></Entry>");
    let mut stream =
        InnerStream::new(InnerStreamKind::ChaCha20, &(0..64).collect::<Vec<u8>>()).unwrap();
    let d = decode(xml.as_bytes(), &[], &mut |v| stream.apply(v)).unwrap();
    assert_eq!(d.root.entries[0].field("Password"), Some("first"));
}
#[test]
fn utf8_bom_and_declared_utf8_accepted() {
    assert!(read(&format!(
        "\u{feff}<?xml version='1.0' encoding='UTF-8'?>{}",
        make_xml("").as_str()
    ))
    .is_ok());
}
#[test]
fn unknown_protected_binary_consumed_and_dropped() {
    let mut enc = InnerStream::new(InnerStreamKind::ChaCha20, &[7; 64]).unwrap();
    let mut bytes = vec![0xff, 0xfe, 0xfd];
    enc.apply(&mut bytes);
    let mut value = b"password".to_vec();
    enc.apply(&mut value);
    let xml=make_xml(&format!("<Unknown Protected='True'>{}</Unknown><Entry><String><Key>Password</Key><Value Protected='True'>{}</Value></String></Entry>",BASE64_STANDARD.encode(&bytes),BASE64_STANDARD.encode(&value)));
    let mut dec = InnerStream::new(InnerStreamKind::ChaCha20, &[7; 64]).unwrap();
    let d = decode(xml.as_bytes(), &[], &mut |v| dec.apply(v)).unwrap();
    assert_eq!(d.root.entries[0].field("Password"), Some("password"));
    assert_eq!(d.dropped_protected_unknown, 1);
    let xml = make_xml(&format!(
        "<Entry><String><Value Protected='True'>{}</Value></String></Entry>",
        BASE64_STANDARD.encode(&bytes)
    ));
    let mut dec = InnerStream::new(InnerStreamKind::ChaCha20, &[7; 64]).unwrap();
    assert!(matches!(
        decode(xml.as_bytes(), &[], &mut |v| dec.apply(v)),
        Err(KdbxError::Corrupt(_))
    ));
}
#[test]
fn protected_document_order_includes_meta_and_history() {
    let mut enc = InnerStream::new(InnerStreamKind::ChaCha20, &[7; 64]).unwrap();
    let values: Vec<String> = ["meta", "entry", "history"]
        .iter()
        .map(|s| {
            let mut b = s.as_bytes().to_vec();
            enc.apply(&mut b);
            BASE64_STANDARD.encode(b)
        })
        .collect();
    let xml=format!("<KeePassFile><Meta><Unknown Protected='True'>{}</Unknown></Meta><Root><Group><Entry><String><Key>Password</Key><Value Protected='True'>{}</Value></String><History><Entry><String><Key>Password</Key><Value Protected='True'>{}</Value></String></Entry></History></Entry></Group></Root></KeePassFile>",values[0],values[1],values[2]);
    let mut dec = InnerStream::new(InnerStreamKind::ChaCha20, &[7; 64]).unwrap();
    let d = decode(xml.as_bytes(), &[], &mut |v| dec.apply(v)).unwrap();
    assert_eq!(d.root.entries[0].field("Password"), Some("entry"));
    assert_eq!(
        d.root.entries[0].history[0].field("Password"),
        Some("history")
    );
    assert_eq!(d.dropped_protected_unknown, 1);
}
#[test]
fn deleted_objects_and_meta_defaults_decode() {
    let d=read("<KeePassFile><Meta/><Root><Group/><DeletedObjects><DeletedObject><UUID>AAECAwQFBgcICQoLDA0ODw==</UUID><DeletionTime>1970-01-01T00:00:00Z</DeletionTime></DeletedObject></DeletedObjects></Root></KeePassFile>").unwrap();
    assert_eq!(d.meta.history_max_items, 10);
    assert_eq!(d.meta.history_max_size, 6_291_456);
    assert_eq!(d.deleted_objects.len(), 1);
    assert_eq!(d.deleted_objects[0].deleted_at, Timestamp(0));
}

#[test]
fn base64_times_containing_t_are_not_iso() {
    for (encoded, raw, expected) in [
        ("TG184g4AAAA=", 63_929_347_404_i64, 1_793_750_604_i64),
        ("TG1T4g4AAAA=", 63_926_660_428_i64, 1_791_063_628_i64),
    ] {
        // Independent little-endian base64 bytes and epoch arithmetic cross-check the literals.
        assert_eq!(BASE64_STANDARD.decode(encoded).unwrap(), raw.to_le_bytes());
        assert_eq!(raw - 62_135_596_800, expected);
        let d = read(&make_xml(&format!(
            "<Times><CreationTime>{encoded}</CreationTime></Times>"
        )))
        .unwrap();
        assert_eq!(d.root.created, Timestamp(expected));
    }
}

#[test]
fn attachment_expansion_budget_includes_history_and_all_entries() {
    let binary = "<Binary><Key>x</Key><Value Ref='0'/></Binary>";
    let pool = [InnerBinary {
        data: b"abc".to_vec(),
        protected: true,
    }];
    for body in [
        format!("<Entry>{binary}{binary}</Entry>"),
        format!("<Entry>{binary}<History><Entry>{binary}</Entry></History></Entry>"),
        format!("<Entry>{binary}</Entry><Group><Entry>{binary}</Entry></Group>"),
    ] {
        let xml = make_xml(&body);
        assert!(decode_with_limit(xml.as_bytes(), &pool, &mut |_| {}, 6).is_ok());
        assert!(matches!(
            decode_with_limit(xml.as_bytes(), &pool, &mut |_| {}, 5),
            Err(KdbxError::Corrupt(ref message)) if message == "attachment expansion exceeds limit"
        ));
    }
    let xml = make_xml(&format!("<Entry>{binary}</Entry>"));
    assert!(decode_with_limit(xml.as_bytes(), &pool, &mut |_| {}, 3).is_ok());
    assert!(matches!(
        decode_with_limit(xml.as_bytes(), &pool, &mut |_| {}, 2),
        Err(KdbxError::Corrupt(_))
    ));
}
