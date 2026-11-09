use super::{
    raw::{self, Node},
    time,
};
use crate::{format::inner_header::InnerBinary, *};
use base64::{prelude::BASE64_STANDARD, Engine};
use uuid::Uuid;
pub(super) fn corrupt(s: &str) -> KdbxError {
    KdbxError::Corrupt(s.into())
}
pub(crate) struct Decoded {
    pub root: Group,
    pub meta: Meta,
    pub deleted_objects: Vec<DeletedObject>,
    pub dropped_attachment_refs: usize,
    pub dropped_protected_unknown: usize,
}
#[cfg(test)]
mod tests;
pub(crate) fn decode(
    data: &[u8],
    pool: &[InnerBinary],
    decrypt: &mut impl FnMut(&mut [u8]),
) -> Result<Decoded> {
    let doc = raw::parse(data, decrypt)?;
    if doc.name != "KeePassFile" {
        return Err(corrupt("missing KeePassFile"));
    }
    let root = doc
        .child("Root")
        .and_then(|n| n.child("Group"))
        .ok_or_else(|| corrupt("missing Root Group"))?;
    let mut ctx = Context {
        pool,
        refs: 0,
        protected: 0,
    };
    let root = ctx.group(root)?;
    let meta = if let Some(n) = doc.child("Meta") {
        ctx.meta(n)?
    } else {
        Meta::default()
    };
    let mut deleted_objects = vec![];
    if let Some(deleted) = doc.child("Root").and_then(|n| n.child("DeletedObjects")) {
        for n in &deleted.children {
            if n.name == "DeletedObject" {
                deleted_objects.push(DeletedObject {
                    uuid: uuid(value(n, "UUID")?)?,
                    deleted_at: time::parse(value(n, "DeletionTime")?)?,
                });
            }
        }
    }
    Ok(Decoded {
        root,
        meta,
        deleted_objects,
        dropped_attachment_refs: ctx.refs,
        dropped_protected_unknown: ctx.protected,
    })
}
fn text<'a>(n: &'a Node<'_>) -> Result<&'a str> {
    if n.invalid_protected_utf8 {
        return Err(corrupt("protected UTF-8"));
    }
    Ok(&n.text)
}
fn value<'a>(n: &'a Node<'_>, name: &str) -> Result<&'a str> {
    n.child(name).map(text).transpose().map(|v| v.unwrap_or(""))
}
fn uuid(s: &str) -> Result<Uuid> {
    let bytes = BASE64_STANDARD
        .decode(s)
        .map_err(|_| corrupt("UUID base64"))?;
    Uuid::from_slice(&bytes).map_err(|_| corrupt("UUID length"))
}
fn custom(n: &Node) -> Result<Vec<CustomDataItem>> {
    n.children
        .iter()
        .filter(|c| c.name == "Item")
        .map(|c| {
            Ok(CustomDataItem {
                key: value(c, "Key")?.into(),
                value: value(c, "Value")?.into(),
                last_modified: c
                    .child("LastModificationTime")
                    .map(|n| time::parse(text(n)?))
                    .transpose()?,
            })
        })
        .collect()
}
#[derive(Default)]
struct Times {
    created: Option<Timestamp>,
    modified: Option<Timestamp>,
    accessed: Option<Timestamp>,
    location: Option<Timestamp>,
    expires: bool,
    expiry: Option<Timestamp>,
    usage: u32,
}
fn times(n: &Node) -> Result<Times> {
    let mut t = Times::default();
    for c in &n.children {
        match c.name.as_str() {
            "CreationTime" => t.created = Some(time::parse(text(c)?)?),
            "LastModificationTime" => t.modified = Some(time::parse(text(c)?)?),
            "LastAccessTime" => t.accessed = Some(time::parse(text(c)?)?),
            "LocationChanged" => t.location = Some(time::parse(text(c)?)?),
            "Expires" => t.expires = text(c)?.eq_ignore_ascii_case("true"),
            "ExpiryTime" => t.expiry = Some(time::parse(text(c)?)?),
            "UsageCount" => t.usage = text(c)?.parse().map_err(|_| corrupt("usage count"))?,
            _ => { /* Unknown Times children intentionally dropped. */ }
        }
    }
    Ok(t)
}
struct Context<'a> {
    pool: &'a [InnerBinary],
    refs: usize,
    protected: usize,
}
impl Context<'_> {
    fn extra(&mut self, n: &Node, out: &mut Vec<RawXml>) -> Result<()> {
        if n.protected_subtree {
            self.protected += 1;
        } else {
            out.push(raw::raw_xml(n)?);
        }
        Ok(())
    }
    fn group(&mut self, n: &Node) -> Result<Group> {
        // Missing times/UUID default deterministically; decoding never invents wall-clock values.
        let mut g = Group::new("", Timestamp(0));
        g.uuid = Uuid::nil();
        for c in &n.children {
            match c.name.as_str() {
                "UUID" => g.uuid = uuid(text(c)?)?,
                "Name" => g.name = text(c)?.to_owned(),
                "IconID" => g.icon_id = text(c)?.parse().map_err(|_| corrupt("icon ID"))?,
                "Times" => {
                    let t = times(c)?;
                    g.created = t.created.unwrap_or(Timestamp(0));
                    g.modified = t.modified.unwrap_or(Timestamp(0));
                    g.accessed = t.accessed.unwrap_or(Timestamp(0));
                    g.location_changed = t.location.unwrap_or(Timestamp(0));
                    g.expires = t.expires;
                    g.expiry_time = t.expiry;
                }
                "Entry" => g.entries.push(self.entry(c)?),
                "Group" => g.groups.push(self.group(c)?),
                "CustomData" => g.custom_data = custom(c)?,
                "Binary" => { /* Binary elements are attachments, never raw XML. */ }
                _ => self.extra(c, &mut g.extra)?,
            }
        }
        Ok(g)
    }
    fn entry(&mut self, n: &Node) -> Result<Entry> {
        let mut e = Entry::new(Timestamp(0));
        e.uuid = Uuid::nil();
        for c in &n.children {
            match c.name.as_str() {
                "UUID" => e.uuid = uuid(text(c)?)?,
                "IconID" => e.icon_id = text(c)?.parse().map_err(|_| corrupt("icon ID"))?,
                "String" => {
                    let v = c.child("Value");
                    if v.is_some_and(|v| v.invalid_protected_utf8) {
                        return Err(corrupt("protected UTF-8"));
                    }
                    e.fields.push(Field {
                        key: value(c, "Key")?.into(),
                        value: value(c, "Value")?.into(),
                        protected: v.is_some_and(|v| v.attr("Protected") == Some("True")),
                    });
                }
                "StringArray" => e.string_arrays.push(StringArray {
                    key: value(c, "Key")?.into(),
                    values: c
                        .children
                        .iter()
                        .filter(|v| v.name == "Value")
                        .map(|v| text(v).map(str::to_owned))
                        .collect::<Result<Vec<_>>>()?,
                }),
                "Binary" => {
                    if let Some(binary) = c
                        .child("Value")
                        .and_then(|v| v.attr("Ref"))
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|i| self.pool.get(i))
                    {
                        e.attachments.push(Attachment {
                            name: value(c, "Key")?.into(),
                            data: binary.data.clone(),
                            protected: binary.protected,
                        });
                    } else {
                        self.refs += 1;
                    }
                }
                "Times" => {
                    let t = times(c)?;
                    e.created = t.created.unwrap_or(Timestamp(0));
                    e.modified = t.modified.unwrap_or(Timestamp(0));
                    e.accessed = t.accessed.unwrap_or(Timestamp(0));
                    e.location_changed = t.location.unwrap_or(Timestamp(0));
                    e.expires = t.expires;
                    e.expiry_time = t.expiry;
                    e.usage_count = t.usage;
                }
                "CustomData" => e.custom_data = custom(c)?,
                "History" => {
                    for h in &c.children {
                        if h.name == "Entry" {
                            e.history.push(self.entry(h)?);
                        }
                    }
                }
                _ => self.extra(c, &mut e.extra)?,
            }
        }
        Ok(e)
    }
    fn meta(&mut self, n: &Node) -> Result<Meta> {
        let mut m = Meta::default();
        for c in &n.children {
            match c.name.as_str() {
                "Generator" => m.generator = text(c)?.to_owned(),
                "DatabaseName" => m.database_name = text(c)?.to_owned(),
                "DatabaseDescription" => m.database_description = text(c)?.to_owned(),
                "DefaultUserName" => m.default_username = text(c)?.to_owned(),
                "HistoryMaxItems" => {
                    m.history_max_items =
                        text(c)?.parse().map_err(|_| corrupt("history max items"))?
                }
                "HistoryMaxSize" => {
                    m.history_max_size =
                        text(c)?.parse().map_err(|_| corrupt("history max size"))?
                }
                "RecycleBinEnabled" => {
                    m.recycle_bin_enabled = text(c)?.eq_ignore_ascii_case("true")
                }
                "RecycleBinUUID" => {
                    m.recycle_bin_uuid = uuid(text(c)?).ok().filter(|u| !u.is_nil())
                }
                "CustomData" => m.custom_data = custom(c)?,
                "Binary" => {}
                _ => self.extra(c, &mut m.extra)?,
            }
        }
        Ok(m)
    }
}
