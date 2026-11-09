//! Bounded UTF-8 event parsing. Consume protected values before interpreting the model.
use super::decode::corrupt;
use crate::{
    format::limits::{MAX_DECOMPRESSED_LEN, MAX_XML_DEPTH},
    Result,
};
use base64::{prelude::BASE64_STANDARD, Engine};
use quick_xml::{events::Event, Reader, Writer};
use zeroize::Zeroizing;
#[derive(Default)]
pub(super) struct Node<'a> {
    pub name: String,
    pub text: Zeroizing<String>,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node<'a>>,
    pub raw: &'a str,
    pub protected_subtree: bool,
    pub invalid_protected_utf8: bool,
}
impl Node<'_> {
    pub fn child(&self, name: &str) -> Option<&Node<'_>> {
        self.children.iter().find(|n| n.name == name)
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}
pub(super) fn parse<'a>(data: &'a [u8], decrypt: &mut impl FnMut(&mut [u8])) -> Result<Node<'a>> {
    if data.len() as u64 > MAX_DECOMPRESSED_LEN {
        return Err(corrupt("XML size"));
    }
    let text = std::str::from_utf8(data).map_err(|_| corrupt("XML UTF-8"))?;
    // NUL is forbidden in XML and identifies BOM-less UTF-16 that is otherwise valid UTF-8.
    if text.contains('\0') {
        return Err(corrupt("XML UTF-8"));
    }
    let mut reader = Reader::from_str(text.strip_prefix('\u{feff}').unwrap_or(text));
    let mut stack: Vec<(Node, u64)> = vec![];
    let mut root = None;
    loop {
        let start = reader.buffer_position();
        let event = reader.read_event().map_err(|_| corrupt("malformed XML"))?;
        match event {
            Event::DocType(_) => return Err(corrupt("XML DOCTYPE")),
            Event::Decl(d) => {
                if root.is_some() || !stack.is_empty() {
                    return Err(corrupt("XML declaration"));
                }
                if let Some(enc) = d.encoding() {
                    let enc = enc.map_err(|_| corrupt("XML encoding"))?;
                    if !enc.eq_ignore_ascii_case("utf-8") {
                        return Err(corrupt("XML UTF-8 declaration"));
                    }
                }
            }
            Event::Start(e) | Event::Empty(e) => {
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(corrupt("XML depth"));
                }
                let empty = text
                    .strip_prefix('\u{feff}')
                    .unwrap_or(text)
                    .get(start as usize..reader.buffer_position() as usize)
                    .is_some_and(|s| s.ends_with("/>"));
                let mut node = Node {
                    name: e.name().as_ref().to_owned(),
                    ..Node::default()
                };
                for a in e.attributes() {
                    let a = a.map_err(|_| corrupt("XML attribute"))?;
                    node.attrs.push((
                        a.key.as_ref().to_owned(),
                        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|_| corrupt("XML attribute"))?
                            .into_owned(),
                    ));
                }
                stack.push((node, start));
                if !empty {
                    continue;
                }
                finish(&mut stack, &mut root, &reader, text, decrypt)?;
            }
            Event::End(_) => finish(&mut stack, &mut root, &reader, text, decrypt)?,
            Event::Text(t) => {
                let s = t.xml10_content();
                if let Some((n, _)) = stack.last_mut() {
                    n.text.push_str(&s);
                } else if !s.trim().is_empty() {
                    return Err(corrupt("text outside root"));
                }
            }
            Event::GeneralRef(r) => {
                let name = r.xml10_content();
                let entity = format!("&{name};");
                let s = quick_xml::escape::unescape(&entity).map_err(|_| corrupt("XML entity"))?;
                stack
                    .last_mut()
                    .ok_or_else(|| corrupt("XML entity outside root"))?
                    .0
                    .text
                    .push_str(&s);
            }
            Event::CData(t) => {
                stack
                    .last_mut()
                    .ok_or_else(|| corrupt("XML CDATA"))?
                    .0
                    .text
                    .push_str(&t.xml10_content());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(corrupt("unclosed XML"));
    }
    root.ok_or_else(|| corrupt("missing XML root"))
}
fn finish<'a>(
    stack: &mut Vec<(Node<'a>, u64)>,
    root: &mut Option<Node<'a>>,
    reader: &Reader<&[u8]>,
    text: &'a str,
    decrypt: &mut impl FnMut(&mut [u8]),
) -> Result<()> {
    let (mut node, start) = stack.pop().ok_or_else(|| corrupt("unexpected XML end"))?;
    let source = text.strip_prefix('\u{feff}').unwrap_or(text);
    let raw = source
        .get(start as usize..reader.buffer_position() as usize)
        .ok_or_else(|| corrupt("XML position"))?;
    node.raw = raw;
    if is_protected(node.attr("Protected")) {
        if !node.children.is_empty() {
            return Err(corrupt("nested protected XML"));
        }
        let mut bytes = Zeroizing::new(
            BASE64_STANDARD
                .decode(node.text.trim())
                .map_err(|_| corrupt("protected base64"))?,
        );
        decrypt(&mut bytes);
        // Unknown protected elements can contain arbitrary bytes. Consume them even if
        // they are not strings; only modelled text values require UTF-8.
        match std::str::from_utf8(&bytes) {
            Ok(s) => node.text = Zeroizing::new(s.to_owned()),
            Err(_) => {
                node.text.clear();
                node.invalid_protected_utf8 = true;
            }
        }
        node.protected_subtree = true;
    }
    if let Some((parent, _)) = stack.last_mut() {
        parent.protected_subtree |= node.protected_subtree;
        parent.children.push(node);
    } else {
        if root.is_some() {
            return Err(corrupt("multiple XML roots"));
        }
        *root = Some(node);
    }
    Ok(())
}
/// Re-serialize only unknown unprotected subtrees. Protected content is never retained as raw XML.
pub(super) fn raw_xml(node: &Node) -> Result<crate::RawXml> {
    let mut reader = Reader::from_str(node.raw);
    let mut writer = Writer::new(Vec::new());
    loop {
        let e = reader.read_event().map_err(|_| corrupt("raw XML"))?;
        if matches!(e, Event::Eof) {
            break;
        }
        writer.write_event(e).map_err(|_| corrupt("raw XML"))?;
    }
    Ok(crate::RawXml(
        String::from_utf8(writer.into_inner()).map_err(|_| corrupt("raw XML UTF-8"))?,
    ))
}

/// The wire predicate is deliberately case-sensitive, after attribute normalization.
pub(super) fn is_protected(value: Option<&str>) -> bool {
    value == Some("True")
}

/// Extras cannot escape their containing element or consume protected-value keystream.
pub(crate) fn validate_extras(extras: &[crate::RawXml], parent_depth: usize) -> Result<()> {
    let invalid = || crate::KdbxError::InvalidOperation("unsafe raw XML extra".into());
    for extra in extras {
        let mut reader = Reader::from_str(&extra.0);
        let mut depth = 0usize;
        let mut elements = 0usize;
        if extra.0.contains('\0') {
            return Err(invalid());
        }
        loop {
            match reader.read_event().map_err(|_| invalid())? {
                Event::Start(e) | Event::Empty(e) => {
                    if parent_depth + depth >= MAX_XML_DEPTH {
                        return Err(invalid());
                    }
                    for attr in e.attributes() {
                        let attr = attr.map_err(|_| invalid())?;
                        let value = attr
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|_| invalid())?;
                        if attr.key.as_ref() == "Protected" && is_protected(Some(&value)) {
                            return Err(invalid());
                        }
                    }
                    elements += 1;
                    // quick-xml exposes the consumed source position for empty tags.
                    if !extra
                        .0
                        .get(..reader.buffer_position() as usize)
                        .is_some_and(|s| s.ends_with("/>"))
                    {
                        depth += 1;
                    }
                }
                Event::End(_) => depth = depth.checked_sub(1).ok_or_else(invalid)?,
                Event::DocType(_) | Event::Decl(_) => return Err(invalid()),
                Event::Text(t) if depth == 0 && !t.xml10_content().trim().is_empty() => {
                    return Err(invalid())
                }
                Event::CData(_) | Event::GeneralRef(_) if depth == 0 => return Err(invalid()),
                Event::Eof => break,
                _ => {}
            }
        }
        if depth != 0 || elements == 0 {
            return Err(invalid());
        }
    }
    Ok(())
}
