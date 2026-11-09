//! Ordered XML encoding. All text is XML-1.0 cleaned before escaping or protection.
use super::pool::AttachmentPool;
use crate::{format::inner_stream::InnerStream, *};
use base64::{prelude::BASE64_STANDARD as B64, Engine};
use zeroize::Zeroizing;
fn clean(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.chars().filter(|c| matches!(*c, '\u{9}' | '\u{a}' | '\u{d}' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')).collect())
}
fn text(out: &mut String, tag: &str, s: &str) {
    out.push('<');
    out.push_str(tag);
    out.push('>');
    for c in clean(s).chars() {
        out.push_str(match c {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&apos;",
            _ => {
                out.push(c);
                continue;
            }
        });
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}
fn time(out: &mut String, tag: &str, t: Timestamp) {
    text(out, tag, &B64.encode(t.to_kdbx_seconds().to_le_bytes()));
}
fn custom(out: &mut String, items: &[CustomDataItem]) {
    if items.is_empty() {
        return;
    }
    out.push_str("<CustomData>");
    for c in items {
        out.push_str("<Item>");
        text(out, "Key", &c.key);
        text(out, "Value", &c.value);
        if let Some(t) = c.last_modified {
            time(out, "LastModificationTime", t);
        }
        out.push_str("</Item>");
    }
    out.push_str("</CustomData>");
}
fn extra(out: &mut String, items: &[RawXml]) {
    for x in items {
        out.push_str(&x.0);
    }
}
fn times(
    out: &mut String,
    ts: [Timestamp; 4],
    expires: bool,
    expiry: Option<Timestamp>,
    usage: Option<u32>,
) {
    out.push_str("<Times>");
    for (tag, t) in [
        "CreationTime",
        "LastModificationTime",
        "LastAccessTime",
        "LocationChanged",
    ]
    .into_iter()
    .zip(ts)
    {
        time(out, tag, t);
    }
    text(out, "Expires", if expires { "True" } else { "False" });
    if let Some(t) = expiry {
        time(out, "ExpiryTime", t);
    }
    if let Some(n) = usage {
        text(out, "UsageCount", &n.to_string());
    }
    out.push_str("</Times>");
}
fn entry(out: &mut String, e: &Entry, pool: &AttachmentPool, stream: &mut InnerStream) {
    out.push_str("<Entry>");
    text(out, "UUID", &B64.encode(e.uuid.as_bytes()));
    text(out, "IconID", &e.icon_id.to_string());
    times(
        out,
        [e.created, e.modified, e.accessed, e.location_changed],
        e.expires,
        e.expiry_time,
        Some(e.usage_count),
    );
    for f in &e.fields {
        out.push_str("<String>");
        text(out, "Key", &f.key);
        if f.protected {
            let mut bytes = Zeroizing::new(clean(&f.value).as_bytes().to_vec());
            stream.apply(&mut bytes);
            out.push_str("<Value Protected=\"True\">");
            out.push_str(&B64.encode(&*bytes));
            out.push_str("</Value>");
        } else {
            text(out, "Value", &f.value);
        }
        out.push_str("</String>");
    }
    for a in &e.attachments {
        out.push_str("<Binary>");
        text(out, "Key", &a.name);
        out.push_str(&format!("<Value Ref=\"{}\"/></Binary>", pool.index_of(a)));
    }
    for a in &e.string_arrays {
        out.push_str("<StringArray>");
        text(out, "Key", &a.key);
        for v in &a.values {
            text(out, "Value", v);
        }
        out.push_str("</StringArray>");
    }
    custom(out, &e.custom_data);
    extra(out, &e.extra);
    if !e.history.is_empty() {
        out.push_str("<History>");
        for e in &e.history {
            entry(out, e, pool, stream);
        }
        out.push_str("</History>");
    }
    out.push_str("</Entry>");
}
fn group(out: &mut String, g: &Group, pool: &AttachmentPool, stream: &mut InnerStream) {
    out.push_str("<Group>");
    text(out, "UUID", &B64.encode(g.uuid.as_bytes()));
    text(out, "Name", &g.name);
    text(out, "IconID", &g.icon_id.to_string());
    times(
        out,
        [g.created, g.modified, g.accessed, g.location_changed],
        g.expires,
        g.expiry_time,
        None,
    );
    custom(out, &g.custom_data);
    extra(out, &g.extra);
    for e in &g.entries {
        entry(out, e, pool, stream);
    }
    for g in &g.groups {
        group(out, g, pool, stream);
    }
    out.push_str("</Group>");
}
pub(crate) fn encode(db: &Database, pool: &AttachmentPool, stream: &mut InnerStream) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><KeePassFile><Meta>");
    let m = &db.meta;
    for (tag, v) in [
        ("Generator", &m.generator),
        ("DatabaseName", &m.database_name),
        ("DatabaseDescription", &m.database_description),
        ("DefaultUserName", &m.default_username),
    ] {
        text(&mut out, tag, v);
    }
    text(
        &mut out,
        "HistoryMaxItems",
        &m.history_max_items.to_string(),
    );
    text(&mut out, "HistoryMaxSize", &m.history_max_size.to_string());
    text(
        &mut out,
        "RecycleBinEnabled",
        if m.recycle_bin_enabled {
            "True"
        } else {
            "False"
        },
    );
    text(
        &mut out,
        "RecycleBinUUID",
        &B64.encode(m.recycle_bin_uuid.unwrap_or(uuid::Uuid::nil()).as_bytes()),
    );
    custom(&mut out, &m.custom_data);
    extra(&mut out, &m.extra);
    out.push_str("</Meta><Root>");
    group(&mut out, &db.root, pool, stream);
    out.push_str("<DeletedObjects>");
    for d in &db.deleted_objects {
        out.push_str("<DeletedObject>");
        text(&mut out, "UUID", &B64.encode(d.uuid.as_bytes()));
        time(&mut out, "DeletionTime", d.deleted_at);
        out.push_str("</DeletedObject>");
    }
    out.push_str("</DeletedObjects></Root></KeePassFile>");
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn make_xml(attachments: bool) -> String {
        let mut db = Database::new("root");
        db.root.uuid = uuid::Uuid::from_bytes([0; 16]);
        let mut e = Entry::new(Timestamp(0));
        e.set_field("Password", "sentinel-password", true);
        if attachments {
            e.attachments.push(Attachment {
                name: "file".into(),
                data: vec![1],
                protected: false,
            });
        }
        e.history.push(Entry::new(Timestamp(0)));
        db.root.entries.push(e);
        let pool = AttachmentPool::collecting(&db.root);
        let mut stream = InnerStream::new(
            crate::format::inner_header::InnerStreamKind::ChaCha20,
            &[0; 64],
        )
        .unwrap();
        encode(&db, &pool, &mut stream)
    }
    #[test]
    fn protected_field_marked_protected_and_not_plaintext_in_xml() {
        let xml = make_xml(false);
        assert!(xml.contains("Protected=\"True\""));
        assert!(!xml.contains("sentinel-password"));
    }
    #[test]
    fn uuid_is_base64_of_raw_16_bytes() {
        assert!(make_xml(false).contains("<UUID>AAAAAAAAAAAAAAAAAAAAAA==</UUID>"));
    }
    #[test]
    fn times_encoded_as_base64_int64_not_iso() {
        assert!(make_xml(false).contains("<CreationTime>APeRdw4AAAA=</CreationTime>"));
    }
    #[test]
    fn binary_emitted_after_strings_before_history() {
        let xml = make_xml(true);
        assert!(xml.find("</String>").unwrap() < xml.find("<Binary>").unwrap());
        assert!(xml.find("<Binary>").unwrap() < xml.find("<History>").unwrap());
    }
    #[test]
    fn no_attachments_emits_no_binary_element() {
        assert!(!make_xml(false).contains("<Binary>"));
    }
    #[test]
    fn empty_custom_data_emits_no_element() {
        assert!(!make_xml(false).contains("<CustomData>"));
    }
    #[test]
    // Full save/open coverage: tests/roundtrip.rs::history_and_string_arrays_survive_round_trip.
    fn special_characters_are_escaped() {
        let mut out = String::new();
        text(&mut out, "Value", "<>&\"'😀");
        assert_eq!(out, "<Value>&lt;&gt;&amp;&quot;&apos;😀</Value>");
    }
}

#[cfg(test)]
mod cleaning_tests {
    use super::*;
    #[test]
    fn xml_10_controls_are_stripped_and_valid_whitespace_survives() {
        let input: String = (0u8..32).map(char::from).collect();
        assert_eq!(&*clean(&input), "\t\n\r");
        assert_eq!(&*clean("a\u{fffe}\u{ffff}😀z"), "a😀z");
    }
}
