//! Entry listing, search, sidebar and detail queries over an open database.
//!
//! Visibility rule shared by every query: the generator history group (by its fixed UUID or by the
//! `MyPass.GeneratorHistory` marker) is never visible, and the recycle bin subtree is visible only
//! to the `Deleted` category (and to a group scope that targets the bin itself).
//!
//! Nothing here returns a secret: summaries and details carry flags and metadata only. The
//! password is read in memory for the security audit (compared by SHA-256) and dropped.

use crate::dto::{
    timestamp_ms, AddressDto, AttachmentMeta, CardDto, Category, CategoryCount, CustomFieldDto,
    EntryDetailDto, EntryQuery, EntrySummary, EntryTypeDto, GroupNode, PasskeyMeta, Scope,
    SecurityFlag, Sidebar, SortKey, TotpDto,
};
use crate::fields::{
    bin_id, is_history_group, EntryFields, ADDRESS_FIELDS, ADDRESS_LABELS, ADDRESS_PREFIX,
    CARD_CVV, CARD_EXPIRY, CARD_HOLDER, CARD_NUMBER, GROUP_EMOJI, NOTES_BODY, PASSKEY_CREATED,
    PASSKEY_RELYING_PARTY, PASSKEY_USERNAME,
};
use mypass_core::entry_ext::{EntryExt, EntryType};
use mypass_core::matcher::host_of;
use mypass_kdbx::{Database, Entry, Group, Timestamp};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The sidebar order of the smart categories.
const CATEGORY_ORDER: [Category; 10] = [
    Category::All,
    Category::Favorites,
    Category::Codes,
    Category::Passkeys,
    Category::Addresses,
    Category::Cards,
    Category::Notes,
    Category::Files,
    Category::Security,
    Category::Deleted,
];

/// A passwords shorter than this is weak.
const MIN_PASSWORD_LEN: usize = 8;
/// A password of a single character class is weak below this length.
const SINGLE_CLASS_MIN_LEN: usize = 12;
/// How many trailing card digits the detail shows.
const CARD_LAST_DIGITS: usize = 4;

struct Item<'a> {
    entry: &'a Entry,
    group_id: Uuid,
    group_name: &'a str,
    in_bin: bool,
}

struct Walk<'a> {
    items: Vec<Item<'a>>,
    /// Ids of the recycle bin group and every group below it.
    bin_groups: HashSet<Uuid>,
}

fn walk(db: &Database) -> Walk<'_> {
    let mut out = Walk {
        items: Vec::new(),
        bin_groups: HashSet::new(),
    };
    walk_group(bin_id(db), &db.root, false, &mut out);
    out
}

fn walk_group<'a>(bin: Option<Uuid>, group: &'a Group, parent_in_bin: bool, out: &mut Walk<'a>) {
    let in_bin = parent_in_bin || bin == Some(group.uuid);
    if in_bin {
        out.bin_groups.insert(group.uuid);
    }
    for entry in &group.entries {
        out.items.push(Item {
            entry,
            group_id: group.uuid,
            group_name: &group.name,
            in_bin,
        });
    }
    for child in group.groups.iter().filter(|g| !is_history_group(g)) {
        walk_group(bin, child, in_bin, out);
    }
}

/// Passwords that occur on at least two visible entries, as SHA-256 digests.
struct Audit {
    reused: HashSet<[u8; 32]>,
}

fn digest(password: &str) -> [u8; 32] {
    Sha256::digest(password.as_bytes()).into()
}

impl Audit {
    fn new(items: &[Item<'_>]) -> Self {
        let mut counts: HashMap<[u8; 32], u32> = HashMap::new();
        for item in items.iter().filter(|i| !i.in_bin) {
            if let Some(pw) = item.entry.field("Password").filter(|p| !p.is_empty()) {
                *counts.entry(digest(pw)).or_default() += 1;
            }
        }
        Self {
            reused: counts
                .into_iter()
                .filter(|(_, n)| *n >= 2)
                .map(|(d, _)| d)
                .collect(),
        }
    }

    fn flags(&self, entry: &Entry) -> Vec<SecurityFlag> {
        let mut flags = Vec::new();
        if entry.entry_type() != EntryType::Login {
            return flags;
        }
        let password = entry.field("Password").unwrap_or("");
        if password.is_empty() {
            flags.push(SecurityFlag::EmptyPassword);
        } else {
            if self.reused.contains(&digest(password)) {
                flags.push(SecurityFlag::Reused);
            }
            if is_weak(password) {
                flags.push(SecurityFlag::Weak);
            }
        }
        if entry
            .urls()
            .iter()
            .any(|u| u.trim_start().to_ascii_lowercase().starts_with("http://"))
        {
            flags.push(SecurityFlag::InsecureUrl);
        }
        flags
    }
}

fn is_weak(password: &str) -> bool {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return true;
    }
    let classes = [
        password.chars().any(|c| c.is_lowercase()),
        password.chars().any(|c| c.is_uppercase()),
        password.chars().any(|c| c.is_ascii_digit()),
        password.chars().any(|c| !c.is_alphanumeric()),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    classes <= 1 && len < SINGLE_CLASS_MIN_LEN
}

fn in_category(category: Category, item: &Item<'_>, audit: &Audit) -> bool {
    let e = item.entry;
    if category == Category::Deleted {
        return item.in_bin;
    }
    if item.in_bin {
        return false;
    }
    match category {
        Category::All => true,
        Category::Favorites => e.is_favorite(),
        Category::Codes => e.otp_source().is_some(),
        Category::Passkeys => e.has_passkey(),
        Category::Addresses => e.entry_type() == EntryType::Address,
        Category::Cards => e.entry_type() == EntryType::Card,
        Category::Notes => e.entry_type() == EntryType::Notes,
        Category::Files => e.entry_type() == EntryType::Files,
        Category::Security => !audit.flags(e).is_empty(),
        Category::Deleted => false,
    }
}

fn group_subtree(group: &Group, out: &mut HashSet<Uuid>) {
    out.insert(group.uuid);
    for child in group.groups.iter().filter(|g| !is_history_group(g)) {
        group_subtree(child, out);
    }
}

fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_lowercase().contains(needle_lower)
}

/// Which fields a search looks at. Never searched in any scope: passwords, TOTP secrets, card
/// number, CVV, protected custom values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    /// The in-app search: everything the user can see in the detail pane. Title, username, every
    /// URL, the KeePass notes, `MyPass.NotesBody`, the group name, the 14 address fields of
    /// ADDRESS entries only, and attachment names.
    App,
    /// The stage 3 browser bridge (`searchEntries`). Only what picking a login to autofill needs:
    /// title, username and every URL (the `URL` field, the `KP2A_URL` array and any `KP2A_URL*`
    /// field). Notes, the group name, address fields and attachment names are never matched, so a
    /// page-adjacent query cannot probe free text or file names the extension has no use for.
    Autofill,
}

/// The one search predicate: a case-insensitive substring, over the fields `scope` allows.
pub fn entry_matches(entry: &Entry, group_name: &str, query: &str, scope: SearchScope) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    let hit = |s: &str| contains_ci(s, &q);
    let app = scope == SearchScope::App;
    if hit(entry.title())
        || hit(entry.field("UserName").unwrap_or(""))
        || (app
            && (hit(entry.field("Notes").unwrap_or(""))
                || hit(entry.field(NOTES_BODY).unwrap_or(""))
                || hit(group_name)))
    {
        return true;
    }
    if entry.urls().iter().any(|u| hit(u))
        || entry
            .fields
            .iter()
            .filter(|f| {
                f.key
                    .starts_with(mypass_core::entry_ext::ADDITIONAL_URLS_KEY)
            })
            .any(|f| hit(&f.value))
        || entry
            .string_arrays
            .iter()
            .filter(|a| {
                a.key
                    .starts_with(mypass_core::entry_ext::ADDITIONAL_URLS_KEY)
            })
            .any(|a| a.values.iter().any(|v| hit(v)))
    {
        return true;
    }
    if !app {
        return false;
    }
    if entry.entry_type() == EntryType::Address
        && entry.fields.iter().any(|f| {
            f.key
                .strip_prefix(ADDRESS_PREFIX)
                .is_some_and(|name| ADDRESS_FIELDS.contains(&name))
                && hit(&f.value)
        })
    {
        return true;
    }
    entry.attachments.iter().any(|a| hit(&a.name))
}

fn address_of(entry: &Entry) -> AddressDto {
    let get = |name: &str| {
        entry
            .field(&format!("{ADDRESS_PREFIX}{name}"))
            .unwrap_or("")
            .to_owned()
    };
    let mut label = get("label");
    if !ADDRESS_LABELS.contains(&label.as_str()) {
        label = "Other".into();
    }
    let given = get("givenName");
    let family = get("familyName");
    let mut full_name = get("fullName");
    if full_name.is_empty() {
        // Never split a full name by guessing; join the explicit parts only.
        full_name = [given.as_str(), family.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    AddressDto {
        label,
        full_name,
        given_name: given,
        family_name: family,
        organization: get("organization"),
        address_line1: get("addressLine1"),
        address_line2: get("addressLine2"),
        city: get("city"),
        region: get("region"),
        postal_code: get("postalCode"),
        country_code: get("countryCode"),
        country_name: get("countryName"),
        phone: get("phone"),
        email: get("email"),
    }
}

fn subtitle(entry: &Entry) -> String {
    match entry.entry_type() {
        EntryType::Login => {
            let user = entry.field("UserName").unwrap_or("");
            if !user.is_empty() {
                return user.to_owned();
            }
            entry
                .urls()
                .first()
                .map(|u| host_of(u).unwrap_or_else(|| u.clone()))
                .unwrap_or_default()
        }
        EntryType::Address => {
            let a = address_of(entry);
            [a.label, a.full_name, a.city]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" \u{b7} ")
        }
        EntryType::Card => entry.field(CARD_HOLDER).unwrap_or("").to_owned(),
        // Notes are secrets: nothing of them in a list row.
        EntryType::Notes => String::new(),
        EntryType::Files => format!("{} files", entry.attachments.len()),
    }
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn summary(item: &Item<'_>, audit: &Audit) -> EntrySummary {
    let e = item.entry;
    EntrySummary {
        id: e.uuid.simple().to_string(),
        title: e.title().to_owned(),
        subtitle: subtitle(e),
        entry_type: e.entry_type().into(),
        favorite: e.is_favorite(),
        has_totp: e.otp_source().is_some(),
        has_passkey: e.has_passkey(),
        attachment_count: count_u32(e.attachments.len()),
        modified: timestamp_ms(e.modified),
        group_id: item.group_id.simple().to_string(),
        in_recycle_bin: item.in_bin,
        flags: audit.flags(e),
    }
}

/// Lists the entries of `q.scope`, filtered by `q.search` and sorted by `q.sort`.
///
/// A category scope searches inside the category; a group scope with a non-empty search searches
/// the whole vault (like the old app), and never the bin unless the group is in the bin.
/// `scope` is `SearchScope::App` for the in-app command and `SearchScope::Autofill` for the stage 3 bridge.
pub fn list_entries(db: &Database, q: &EntryQuery, scope: SearchScope) -> Vec<EntrySummary> {
    let walked = walk(db);
    let audit = Audit::new(&walked.items);
    let searching = !q.search.is_empty();
    let mut out: Vec<EntrySummary> = Vec::new();
    let selected: Vec<&Item<'_>> = match &q.scope {
        Scope::Category { category } => walked
            .items
            .iter()
            .filter(|i| in_category(*category, i, &audit))
            .collect(),
        Scope::Group { group_id } => {
            let Ok(target) = Uuid::parse_str(group_id) else {
                return Vec::new();
            };
            let Some(group) = db.find_group(target) else {
                return Vec::new();
            };
            if is_history_group(group) {
                return Vec::new();
            }
            let target_in_bin = walked.bin_groups.contains(&target);
            let mut subtree = HashSet::new();
            group_subtree(group, &mut subtree);
            walked
                .items
                .iter()
                .filter(|i| {
                    let visible = target_in_bin || !i.in_bin;
                    visible && (searching || subtree.contains(&i.group_id))
                })
                .collect()
        }
    };
    for item in selected {
        if entry_matches(item.entry, item.group_name, &q.search, scope) {
            out.push(summary(item, &audit));
        }
    }
    match q.sort {
        SortKey::Title => out.sort_by_cached_key(|s| s.title.to_lowercase()),
        SortKey::Modified => out.sort_by(|a, b| b.modified.total_cmp(&a.modified)),
    }
    out
}

fn group_node(group: &Group, bin: Option<Uuid>) -> GroupNode {
    let is_bin = bin == Some(group.uuid);
    let mut children: Vec<GroupNode> = group
        .groups
        .iter()
        .filter(|g| !is_history_group(g))
        .map(|g| group_node(g, bin))
        .collect();
    // Stable: the recycle bin moves behind its siblings, everything else keeps document order.
    children.sort_by_key(|c| c.is_recycle_bin);
    let count = group.entries.len() + children.iter().map(|c| c.count as usize).sum::<usize>();
    GroupNode {
        id: group.uuid.simple().to_string(),
        name: group.name.clone(),
        emoji: group
            .custom(GROUP_EMOJI)
            .filter(|e| !e.is_empty())
            .map(str::to_owned),
        count: count_u32(count),
        is_recycle_bin: is_bin,
        children,
    }
}

/// The category counts and the group tree (the children of the root, recycle bin last). The
/// `Security` count is `None`: the sidebar shows an em dash.
pub fn sidebar(db: &Database) -> Sidebar {
    let walked = walk(db);
    let audit = Audit::new(&walked.items);
    let categories = CATEGORY_ORDER
        .into_iter()
        .map(|category| CategoryCount {
            category,
            count: (category != Category::Security).then(|| {
                count_u32(
                    walked
                        .items
                        .iter()
                        .filter(|i| in_category(category, i, &audit))
                        .count(),
                )
            }),
        })
        .collect();
    let bin = bin_id(db);
    let mut groups: Vec<GroupNode> = db
        .root
        .groups
        .iter()
        .filter(|g| !is_history_group(g))
        .map(|g| group_node(g, bin))
        .collect();
    groups.sort_by_key(|g| g.is_recycle_bin);
    Sidebar {
        categories,
        root_group_id: db.root_id().simple().to_string(),
        groups,
        recycle_bin_enabled: db.meta.recycle_bin_enabled,
    }
}

fn last_digits(number: &str) -> String {
    let digits: Vec<char> = number
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    // A number too short to hide anything shows nothing.
    if digits.len() <= CARD_LAST_DIGITS {
        return String::new();
    }
    digits
        .iter()
        .skip(digits.len() - CARD_LAST_DIGITS)
        .collect()
}

/// The secret-free detail of one entry (also for an entry in the bin, flagged `in_recycle_bin`).
/// Protected custom values, the password, TOTP secret, notes body, card number and CVV are never
/// included; only whether they exist.
pub fn entry_detail(db: &Database, id: Uuid) -> Option<EntryDetailDto> {
    let entry = db.find_entry(id)?;
    let group_id = db.parent_of_entry(id)?;
    let group_name = db
        .find_group(group_id)
        .map(|g| g.name.clone())
        .unwrap_or_default();
    let entry_type = entry.entry_type();
    let urls = entry.urls();
    let totp = entry.totp();
    let passkey = entry.has_passkey().then(|| PasskeyMeta {
        relying_party: entry.field(PASSKEY_RELYING_PARTY).unwrap_or("").to_owned(),
        username: entry.field(PASSKEY_USERNAME).unwrap_or("").to_owned(),
        created: entry
            .field(PASSKEY_CREATED)
            .and_then(Timestamp::parse_iso8601)
            .map(timestamp_ms),
    });
    let card = (entry_type == EntryType::Card).then(|| CardDto {
        holder: entry.field(CARD_HOLDER).unwrap_or("").to_owned(),
        number_last4: last_digits(entry.field(CARD_NUMBER).unwrap_or("")),
        expiry: entry.field(CARD_EXPIRY).unwrap_or("").to_owned(),
        has_cvv: entry.field(CARD_CVV).is_some_and(|v| !v.is_empty()),
    });
    let has_notes = ["Notes", NOTES_BODY]
        .into_iter()
        .any(|k| entry.field(k).is_some_and(|v| !v.is_empty()));
    let custom_fields = entry
        .fields
        .iter()
        .filter(|f| !Entry::is_reserved_key(&f.key))
        .map(|f| CustomFieldDto {
            key: f.key.clone(),
            protected: f.protected,
            value: (!f.protected).then(|| f.value.clone()),
        })
        .collect();
    Some(EntryDetailDto {
        id: id.simple().to_string(),
        title: entry.title().to_owned(),
        entry_type: EntryTypeDto::from(entry_type),
        group_id: group_id.simple().to_string(),
        group_name,
        favorite: entry.is_favorite(),
        created: timestamp_ms(entry.created),
        modified: timestamp_ms(entry.modified),
        username: entry.field("UserName").unwrap_or("").to_owned(),
        url: urls.first().cloned(),
        urls,
        uri_match_mode: entry.uri_match_mode().as_str().to_owned(),
        has_password: entry.field("Password").is_some_and(|p| !p.is_empty()),
        totp_period: totp.as_ref().map(|t| t.period),
        totp_digits: totp.as_ref().map(|t| t.digits),
        passkey,
        address: (entry_type == EntryType::Address).then(|| address_of(entry)),
        card,
        has_notes,
        custom_fields,
        attachments: entry
            .attachments
            .iter()
            .map(|a| AttachmentMeta {
                name: a.name.clone(),
                size: a.data.len() as f64,
            })
            .collect(),
        history_count: count_u32(entry.history.len()),
        in_recycle_bin: db.is_in_recycle_bin(id),
    })
}

/// The current TOTP code of an entry, from its own `otp` parameters (digits, period, algorithm).
pub fn totp_now(db: &Database, id: Uuid, unix_secs: u64) -> Option<TotpDto> {
    let config = db.find_entry(id)?.totp()?;
    let code = config.code_at(unix_secs).ok()?;
    Some(TotpDto {
        code,
        seconds_remaining: config.seconds_remaining(unix_secs),
        period: config.period,
    })
}

/// The most recently accessed visible entries, newest first, at most `limit`.
pub fn recent_entries(db: &Database, limit: usize) -> Vec<EntrySummary> {
    let walked = walk(db);
    let audit = Audit::new(&walked.items);
    let mut items: Vec<&Item<'_>> = walked.items.iter().filter(|i| !i.in_bin).collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.entry.accessed));
    items
        .into_iter()
        .take(limit)
        .map(|i| summary(i, &audit))
        .collect()
}

// ---- tests ----
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::{EntryFields, PASSKEY_CREDENTIAL_ID, PASSKEY_PRIVATE_KEY_PEM};
    use crate::testing::{fast_database, has_passkey_cases};
    use mypass_core::entry_ext::{EntryExt, ENTRY_TYPE_FIELD, GENERATOR_HISTORY_GROUP_UUID};
    use mypass_kdbx::{Attachment, Group, StringArray, Timestamp};

    const NOW: Timestamp = Timestamp(1_700_000_000);

    fn simple(id: Uuid) -> String {
        id.simple().to_string()
    }

    struct Sample {
        db: Database,
        a: Uuid,
        b: Uuid,
        c: Uuid,
        d: Uuid,
        e: Uuid,
        f: Uuid,
        g: Uuid,
        h: Uuid,
        personal: Uuid,
        docs: Uuid,
    }

    fn entry(title: &str) -> Entry {
        let mut e = Entry::new(NOW);
        e.set_field("Title", title, false);
        e
    }

    fn login(title: &str, user: &str, password: &str, url: &str) -> Entry {
        let mut e = entry(title);
        e.set_field("UserName", user, false);
        e.set_field("Password", password, true);
        e.set_field("URL", url, false);
        e
    }

    fn typed(mut e: Entry, kind: &str) -> Entry {
        e.set_field(ENTRY_TYPE_FIELD, kind, false);
        e
    }

    fn attachment(name: &str, bytes: usize) -> Attachment {
        Attachment {
            name: name.into(),
            data: vec![7; bytes],
            protected: false,
        }
    }

    fn sample_database() -> Sample {
        let mut db = fast_database("Sample");
        let root = db.root_id();

        // G is deleted first, so the recycle bin is created before the other groups and is not
        // last in document order.
        let g_entry = login("Old Login", "gone", "oldpassword", "https://old.test");
        let g = g_entry.uuid;
        db.add_entry(root, g_entry, NOW).unwrap();
        db.delete_entry(g, NOW).unwrap();

        let history = {
            let mut grp = Group::new("Password Generator History", NOW);
            grp.uuid = GENERATOR_HISTORY_GROUP_UUID;
            grp
        };
        db.add_group(root, history, NOW).unwrap();
        let personal_group = Group::new("Personal", NOW);
        let personal = personal_group.uuid;
        db.add_group(root, personal_group, NOW).unwrap();
        let docs_group = Group::new("Docs", NOW);
        let docs = docs_group.uuid;
        db.add_group(personal, docs_group, NOW).unwrap();

        let mut a = login("GitHub", "alice", "hunter2", "https://github.com");
        a.set_field("Favorite", "1", false);
        a.set_field("otp", "JBSWY3DPEHPK3PXP", true);
        a.attachments.push(attachment("a.txt", 3));
        a.set_field("MyPass.Address.city", "Lviv", false);
        a.string_arrays.push(StringArray {
            key: "KP2A_URL".into(),
            values: vec!["https://gh.example".into()],
        });
        let a_id = a.uuid;
        db.add_entry(root, a, NOW).unwrap();

        let mut b = login("Passkey Site", "", "", "");
        b.set_field(PASSKEY_CREDENTIAL_ID, "abc", true);
        b.set_field(PASSKEY_PRIVATE_KEY_PEM, "pem", true);
        let b_id = b.uuid;
        db.add_entry(root, b, NOW).unwrap();

        let mut c = typed(entry("Home Address"), "Address");
        c.set_field("MyPass.Address.label", "Home", true);
        c.set_field("MyPass.Address.fullName", "Alice Example", true);
        c.set_field("MyPass.Address.addressLine1", "1 Khreshchatyk", true);
        c.set_field("MyPass.Address.city", "Kyiv", true);
        let c_id = c.uuid;
        db.add_entry(personal, c, NOW).unwrap();

        let mut d = typed(entry("Visa Card"), "Card");
        d.set_field("MyPass.CardholderName", "Alice A", false);
        d.set_field("MyPass.CardNumber", "4111111111111111", true);
        d.set_field("MyPass.ExpiryDate", "12/30", false);
        d.set_field("MyPass.CVV", "7391", true);
        let d_id = d.uuid;
        db.add_entry(personal, d, NOW).unwrap();

        let mut e = typed(entry("Journal"), "Notes");
        e.set_field("MyPass.NotesBody", "secret note text", false);
        let e_id = e.uuid;
        db.add_entry(docs, e, NOW).unwrap();

        let mut f = typed(entry("Documents"), "Files");
        f.attachments.push(attachment("one.pdf", 10));
        f.attachments.push(attachment("two.pdf", 20));
        let f_id = f.uuid;
        db.add_entry(root, f, NOW).unwrap();

        let h = login("Generated", "", "gen-password-1", "");
        let h_id = h.uuid;
        db.add_entry(GENERATOR_HISTORY_GROUP_UUID, h, NOW).unwrap();

        Sample {
            db,
            a: a_id,
            b: b_id,
            c: c_id,
            d: d_id,
            e: e_id,
            f: f_id,
            g,
            h: h_id,
            personal,
            docs,
        }
    }

    fn cat(c: Category, search: &str) -> EntryQuery {
        EntryQuery {
            scope: Scope::Category { category: c },
            search: search.into(),
            sort: SortKey::Title,
        }
    }

    fn ids(list: &[EntrySummary]) -> Vec<String> {
        list.iter().map(|s| s.id.clone()).collect()
    }

    fn titles(list: &[EntrySummary]) -> Vec<String> {
        list.iter().map(|s| s.title.clone()).collect()
    }

    fn search_ids(s: &Sample, text: &str) -> Vec<String> {
        ids(&list_entries(
            &s.db,
            &cat(Category::All, text),
            SearchScope::App,
        ))
    }

    #[test]
    fn category_counts_match_spec() {
        let s = sample_database();
        let counts = sidebar(&s.db).categories;
        let get = |c: Category| counts.iter().find(|x| x.category == c).unwrap().count;
        assert_eq!(get(Category::All), Some(6));
        assert_eq!(get(Category::Favorites), Some(1));
        assert_eq!(get(Category::Codes), Some(1));
        assert_eq!(get(Category::Passkeys), Some(1));
        assert_eq!(get(Category::Addresses), Some(1));
        assert_eq!(get(Category::Cards), Some(1));
        assert_eq!(get(Category::Notes), Some(1));
        assert_eq!(get(Category::Files), Some(1));
        assert_eq!(get(Category::Deleted), Some(1));
        assert_eq!(get(Category::Security), None);
        assert_eq!(counts.len(), 10);
    }

    #[test]
    fn bin_uuid_pointing_at_the_root_means_no_bin() {
        let mut s = sample_database();
        let root = s.db.root_id();
        s.db.meta.recycle_bin_uuid = Some(root);
        assert_eq!(bin_id(&s.db), None);
        assert!(list_entries(&s.db, &cat(Category::Deleted, ""), SearchScope::App).is_empty());
        let all = list_entries(&s.db, &cat(Category::All, ""), SearchScope::App);
        assert!(all.iter().all(|e| !e.in_recycle_bin));
        assert!(ids(&all).contains(&simple(s.a)));
        assert!(!entry_detail(&s.db, s.a).unwrap().in_recycle_bin);
        let sb = sidebar(&s.db);
        let count = |c: Category| {
            sb.categories
                .iter()
                .find(|x| x.category == c)
                .unwrap()
                .count
        };
        assert_eq!(count(Category::Deleted), Some(0));
        assert!(sb.groups.iter().all(|g| !g.is_recycle_bin));
    }

    #[test]
    fn sidebar_reports_whether_the_recycle_bin_is_enabled() {
        let mut s = sample_database();
        s.db.meta.recycle_bin_enabled = true;
        assert!(sidebar(&s.db).recycle_bin_enabled);
        s.db.meta.recycle_bin_enabled = false;
        assert!(!sidebar(&s.db).recycle_bin_enabled);
    }

    #[test]
    fn deleted_lists_bin_only() {
        let s = sample_database();
        let list = list_entries(&s.db, &cat(Category::Deleted, ""), SearchScope::App);
        assert_eq!(ids(&list), [simple(s.g)]);
        assert!(list[0].in_recycle_bin);
    }

    #[test]
    fn history_group_entries_never_listed_or_counted() {
        let s = sample_database();
        let h = simple(s.h);
        for c in [
            Category::All,
            Category::Deleted,
            Category::Security,
            Category::Codes,
        ] {
            assert!(!ids(&list_entries(&s.db, &cat(c, ""), SearchScope::App)).contains(&h));
        }
        assert!(!search_ids(&s, "Generated").contains(&h));
        let q = EntryQuery {
            scope: Scope::Group {
                group_id: simple(GENERATOR_HISTORY_GROUP_UUID),
            },
            search: String::new(),
            sort: SortKey::Title,
        };
        assert!(list_entries(&s.db, &q, SearchScope::App).is_empty());
        assert!(!ids(&recent_entries(&s.db, 50)).contains(&h));
        // Recognised by the marker as well, whatever the UUID.
        let mut db = fast_database("m");
        let root = db.root_id();
        let mut grp = Group::new("Hidden", NOW);
        grp.set_custom(crate::fields::GENERATOR_HISTORY_MARKER, "1");
        let gid = grp.uuid;
        db.add_group(root, grp, NOW).unwrap();
        db.add_entry(gid, entry("x"), NOW).unwrap();
        assert!(list_entries(&db, &cat(Category::All, ""), SearchScope::App).is_empty());
    }

    #[test]
    fn search_matches_title_user_url_group_case_insensitively() {
        let s = sample_database();
        assert_eq!(search_ids(&s, "ALICE"), [simple(s.a), simple(s.c)]);
        assert_eq!(search_ids(&s, "github.com"), [simple(s.a)]);
        assert_eq!(search_ids(&s, "GITHUB"), [simple(s.a)]);
        // The group name matches every entry directly inside that group.
        assert_eq!(search_ids(&s, "docs"), [simple(s.e)]);
        assert_eq!(search_ids(&s, "personal"), [simple(s.c), simple(s.d)]);
    }

    #[test]
    fn search_finds_notes_body() {
        let mut s = sample_database();
        assert_eq!(search_ids(&s, "secret note"), [simple(s.e)]);
        let a = s.a;
        s.db.find_entry_mut(a)
            .unwrap()
            .set_field("Notes", "plain keepass note", true);
        assert_eq!(search_ids(&s, "keepass note"), [simple(s.a)]);
    }

    #[test]
    fn search_finds_kp2a_additional_urls() {
        let s = sample_database();
        assert_eq!(search_ids(&s, "gh.example"), [simple(s.a)]);
        let mut e = entry("Field style");
        e.set_field("KP2A_URL_1", "https://third.example", false);
        assert!(entry_matches(
            &e,
            "",
            "third.example",
            SearchScope::Autofill
        ));
    }

    #[test]
    fn search_finds_address_fields_for_address_entries_only() {
        let s = sample_database();
        assert_eq!(search_ids(&s, "khreshchatyk"), [simple(s.c)]);
        // `MyPass.Address.city=Lviv` sits on a login, so it is not searched.
        assert!(search_ids(&s, "lviv").is_empty());
    }

    #[test]
    fn search_does_not_match_card_number_cvv_or_passwords() {
        let s = sample_database();
        for q in ["4111", "7391", "hunter2", "JBSWY3"] {
            assert!(search_ids(&s, q).is_empty(), "{q}");
        }
    }

    #[test]
    fn app_search_finds_attachment_names_bridge_does_not() {
        let s = sample_database();
        let q = cat(Category::All, "a.txt");
        assert_eq!(
            ids(&list_entries(&s.db, &q, SearchScope::App)),
            [simple(s.a)]
        );
        assert!(list_entries(&s.db, &q, SearchScope::Autofill).is_empty());
    }

    #[test]
    fn app_scope_matches_notes_group_and_address_fields() {
        let s = sample_database();
        let hits = |text: &str| {
            ids(&list_entries(
                &s.db,
                &cat(Category::All, text),
                SearchScope::App,
            ))
        };
        assert_eq!(hits("secret note"), [simple(s.e)]);
        assert_eq!(hits("khreshchatyk"), [simple(s.c)]);
        assert_eq!(hits("docs"), [simple(s.e)]);
        assert_eq!(hits("a.txt"), [simple(s.a)]);
        assert_eq!(hits("github.com"), [simple(s.a)]);
    }

    #[test]
    fn autofill_scope_matches_only_title_username_and_urls() {
        let mut s = sample_database();
        let a = s.a;
        s.db.find_entry_mut(a)
            .unwrap()
            .set_field("Notes", "plain keepass note", true);
        let hits = |text: &str| {
            ids(&list_entries(
                &s.db,
                &cat(Category::All, text),
                SearchScope::Autofill,
            ))
        };
        // Title, username, URL and additional URLs still match.
        assert_eq!(hits("GITHUB"), [simple(s.a)]);
        // `c` also matches "ALICE" in the app scope, through a field autofill does not read.
        assert_eq!(hits("ALICE"), [simple(s.a)]);
        assert_eq!(hits("github.com"), [simple(s.a)]);
        assert_eq!(hits("gh.example"), [simple(s.a)]);
        // Notes, group name, address fields and attachment names do not.
        assert!(hits("secret note").is_empty());
        assert!(hits("plain keepass note").is_empty());
        assert!(hits("docs").is_empty());
        assert!(hits("khreshchatyk").is_empty());
        assert!(hits("a.txt").is_empty());
    }

    #[test]
    fn has_passkey_contract_table() {
        for (fields, expected) in has_passkey_cases() {
            let mut e = Entry::new(NOW);
            for (k, v) in &fields {
                e.set_field(k, v, true);
            }
            assert_eq!(e.has_passkey(), expected, "{fields:?}");
        }
    }

    #[test]
    fn search_in_deleted_scope_searches_bin_only() {
        let s = sample_database();
        assert_eq!(
            ids(&list_entries(
                &s.db,
                &cat(Category::Deleted, "old login"),
                SearchScope::App
            )),
            [simple(s.g)]
        );
        assert!(search_ids(&s, "old login").is_empty());
        assert!(
            list_entries(&s.db, &cat(Category::Deleted, "github"), SearchScope::App).is_empty()
        );
    }

    #[test]
    fn group_scope_lists_subtree() {
        let s = sample_database();
        let group = |id: Uuid, search: &str| EntryQuery {
            scope: Scope::Group {
                group_id: simple(id),
            },
            search: search.into(),
            sort: SortKey::Title,
        };
        assert_eq!(
            titles(&list_entries(
                &s.db,
                &group(s.personal, ""),
                SearchScope::App
            )),
            ["Home Address", "Journal", "Visa Card"]
        );
        assert_eq!(
            titles(&list_entries(&s.db, &group(s.docs, ""), SearchScope::App)),
            ["Journal"]
        );
        // Searching inside a group searches the whole vault, like the old app.
        assert_eq!(
            ids(&list_entries(
                &s.db,
                &group(s.docs, "github"),
                SearchScope::App
            )),
            [simple(s.a)]
        );
        // The bin group lists its entries; a bad id lists nothing.
        let bin = s.db.meta.recycle_bin_uuid.unwrap();
        assert_eq!(
            ids(&list_entries(&s.db, &group(bin, ""), SearchScope::App)),
            [simple(s.g)]
        );
        let bad = EntryQuery {
            scope: Scope::Group {
                group_id: "nope".into(),
            },
            search: String::new(),
            sort: SortKey::Title,
        };
        assert!(list_entries(&s.db, &bad, SearchScope::App).is_empty());
    }

    #[test]
    fn sort_title_is_case_insensitive_and_modified_is_newest_first() {
        let mut db = fast_database("s");
        let root = db.root_id();
        let mut ids_by_title = Vec::new();
        for (title, modified) in [("b", 100), ("A", 300), ("c", 200)] {
            let mut e = entry(title);
            e.modified = Timestamp(modified);
            ids_by_title.push(e.uuid);
            db.add_entry(root, e, NOW).unwrap();
        }
        // add_entry may touch times; pin them explicitly.
        for (id, modified) in ids_by_title.iter().zip([100, 300, 200]) {
            db.find_entry_mut(*id).unwrap().modified = Timestamp(modified);
        }
        let mut q = cat(Category::All, "");
        assert_eq!(
            titles(&list_entries(&db, &q, SearchScope::App)),
            ["A", "b", "c"]
        );
        q.sort = SortKey::Modified;
        let list = list_entries(&db, &q, SearchScope::App);
        assert_eq!(titles(&list), ["A", "c", "b"]);
        assert_eq!(list[0].modified, 300_000.0);
    }

    #[test]
    fn subtitle_rules() {
        let s = sample_database();
        let list = list_entries(&s.db, &cat(Category::All, ""), SearchScope::App);
        let sub = |id: Uuid| {
            list.iter()
                .find(|x| x.id == simple(id))
                .unwrap()
                .subtitle
                .clone()
        };
        assert_eq!(sub(s.a), "alice");
        assert_eq!(sub(s.c), "Home \u{b7} Alice Example \u{b7} Kyiv");
        assert_eq!(sub(s.d), "Alice A");
        assert_eq!(sub(s.e), "");
        assert_eq!(sub(s.f), "2 files");
        // A login without a username falls back to the URL host.
        let mut db = fast_database("u");
        let root = db.root_id();
        db.add_entry(
            root,
            login("x", "", "pw", "https://www.example.com/path"),
            NOW,
        )
        .unwrap();
        let l = list_entries(&db, &cat(Category::All, ""), SearchScope::App);
        assert_eq!(l[0].subtitle, "www.example.com");
    }

    #[test]
    fn summary_fields() {
        let s = sample_database();
        let list = list_entries(&s.db, &cat(Category::All, ""), SearchScope::App);
        let a = list.iter().find(|x| x.id == simple(s.a)).unwrap();
        assert!(a.favorite && a.has_totp && !a.has_passkey);
        assert_eq!(a.attachment_count, 1);
        assert_eq!(a.entry_type, EntryTypeDto::Login);
        assert_eq!(a.group_id, simple(s.db.root_id()));
        assert!(!a.in_recycle_bin);
        let b = list.iter().find(|x| x.id == simple(s.b)).unwrap();
        assert!(b.has_passkey);
        let c = list.iter().find(|x| x.id == simple(s.c)).unwrap();
        assert_eq!(c.entry_type, EntryTypeDto::Address);
    }

    #[test]
    fn detail_dto_contains_no_secret_values() {
        let mut s = sample_database();
        let mut rec = login("Rec", "u", "pw-rec-1", "https://rec.test");
        rec.set_field("Recovery", "zz-secret-zz", true);
        let rec_id = rec.uuid;
        let root = s.db.root_id();
        s.db.add_entry(root, rec, NOW).unwrap();

        for id in [s.a, s.d, s.e, rec_id] {
            let detail = entry_detail(&s.db, id).unwrap();
            let json = serde_json::to_string(&detail).unwrap();
            for secret in [
                "hunter2",
                "JBSWY3DPEHPK3PXP",
                "4111111111111111",
                "7391",
                "secret note text",
                "zz-secret-zz",
                "pw-rec-1",
            ] {
                assert!(!json.contains(secret), "{secret} leaked in {json}");
            }
        }
        let a = entry_detail(&s.db, s.a).unwrap();
        assert!(a.has_password);
        assert_eq!(a.title, "GitHub");
        assert_eq!(a.username, "alice");
        assert_eq!(a.url.as_deref(), Some("https://github.com"));
        assert_eq!(a.urls, ["https://github.com", "https://gh.example"]);
        assert_eq!(a.uri_match_mode, "registrable-domain");
        assert_eq!(a.totp_period, Some(30));
        assert_eq!(a.totp_digits, Some(6));
        assert_eq!(
            a.attachments,
            [AttachmentMeta {
                name: "a.txt".into(),
                size: 3.0
            }]
        );
        assert_eq!(a.group_name, "Sample");
        assert!(a.favorite && !a.in_recycle_bin);
        // The stray address-named field is reserved, so it is not a custom field row.
        assert!(a.custom_fields.is_empty());

        let d = entry_detail(&s.db, s.d).unwrap();
        let card = d.card.unwrap();
        assert_eq!(card.number_last4, "1111");
        assert_eq!(card.holder, "Alice A");
        assert_eq!(card.expiry, "12/30");
        assert!(card.has_cvv);

        let e = entry_detail(&s.db, s.e).unwrap();
        assert!(e.has_notes);

        let r = entry_detail(&s.db, rec_id).unwrap();
        assert_eq!(r.custom_fields.len(), 1);
        assert_eq!(r.custom_fields[0].key, "Recovery");
        assert!(r.custom_fields[0].protected);
        assert_eq!(r.custom_fields[0].value, None);

        let c = entry_detail(&s.db, s.c).unwrap();
        let addr = c.address.unwrap();
        assert_eq!(addr.label, "Home");
        assert_eq!(addr.city, "Kyiv");

        let b = entry_detail(&s.db, s.b).unwrap();
        assert!(b.passkey.is_some());
        let g = entry_detail(&s.db, s.g).unwrap();
        assert!(g.in_recycle_bin);
    }

    #[test]
    fn detail_for_missing_id_is_none() {
        let s = sample_database();
        assert!(entry_detail(&s.db, Uuid::new_v4()).is_none());
        assert!(totp_now(&s.db, Uuid::new_v4(), 0).is_none());
    }

    #[test]
    fn totp_now_uses_otpauth_params() {
        let mut db = fast_database("t");
        let root = db.root_id();
        let mut e = entry("otp");
        e.set_field(
            "otp",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=8&period=60&algorithm=SHA256",
            true,
        );
        let id = e.uuid;
        db.add_entry(root, e, NOW).unwrap();
        let t = totp_now(&db, id, 1_700_000_000).unwrap();
        assert_eq!(t.period, 60);
        assert_eq!(t.code.len(), 8);
        assert_eq!(t.seconds_remaining, 60 - (1_700_000_000 % 60) as u32);
        let detail = entry_detail(&db, id).unwrap();
        assert_eq!(
            (detail.totp_period, detail.totp_digits),
            (Some(60), Some(8))
        );
        // An entry without otp has no code.
        let plain = entry("plain");
        let plain_id = plain.uuid;
        db.add_entry(root, plain, NOW).unwrap();
        assert!(totp_now(&db, plain_id, 0).is_none());
    }

    #[test]
    fn security_flags() {
        let mut db = fast_database("sec");
        let root = db.root_id();
        let mut add = |e: Entry| {
            let id = e.uuid;
            db.add_entry(root, e, NOW).unwrap();
            id
        };
        let r1 = add(login("r1", "u", "samepass99", "https://r1.test"));
        let r2 = add(login("r2", "u", "samepass99", "https://r2.test"));
        let weak = add(login("weak", "u", "abc", "https://w.test"));
        let empty = add(login("empty", "u", "", "https://e.test"));
        let insecure = add(login("insecure", "u", "Str0ng!Passw0rd#1", "http://x.test"));
        let fine = add(login("fine", "u", "Str0ng!Passw0rd#2", "https://f.test"));
        let list = list_entries(&db, &cat(Category::All, ""), SearchScope::App);
        let flags = |id: Uuid| {
            list.iter()
                .find(|x| x.id == simple(id))
                .unwrap()
                .flags
                .clone()
        };
        assert_eq!(flags(r1), [SecurityFlag::Reused]);
        assert_eq!(flags(r2), [SecurityFlag::Reused]);
        assert_eq!(flags(weak), [SecurityFlag::Weak]);
        assert_eq!(flags(empty), [SecurityFlag::EmptyPassword]);
        assert_eq!(flags(insecure), [SecurityFlag::InsecureUrl]);
        assert!(flags(fine).is_empty());
    }

    #[test]
    fn weak_rule_boundaries() {
        let weak = |pw: &str| {
            let mut db = fast_database("w");
            let root = db.root_id();
            db.add_entry(root, login("x", "u", pw, "https://x.test"), NOW)
                .unwrap();
            list_entries(&db, &cat(Category::All, ""), SearchScope::App)[0]
                .flags
                .contains(&SecurityFlag::Weak)
        };
        assert!(weak("1234567"));
        assert!(!weak("a1b2c3d4"));
        assert!(weak("abcdefgh"));
        assert!(weak("abcdefghijk"));
        assert!(!weak("abcdefghijkl"));
        assert!(weak("aaaaaaaaa"));
    }

    #[test]
    fn security_scope_lists_only_flagged() {
        let mut db = fast_database("sec2");
        let root = db.root_id();
        db.add_entry(root, login("weak", "u", "abc", "https://w.test"), NOW)
            .unwrap();
        db.add_entry(
            root,
            login("fine", "u", "Str0ng!Passw0rd#2", "https://f.test"),
            NOW,
        )
        .unwrap();
        let list = list_entries(&db, &cat(Category::Security, ""), SearchScope::App);
        assert_eq!(titles(&list), ["weak"]);
    }

    #[test]
    fn sidebar_group_tree_counts_and_bin_last_and_history_omitted() {
        let s = sample_database();
        let sb = sidebar(&s.db);
        assert_eq!(sb.root_group_id, simple(s.db.root_id()));
        let names: Vec<_> = sb.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Personal", "Recycle Bin"]);
        let personal = &sb.groups[0];
        assert_eq!(personal.count, 3);
        assert!(!personal.is_recycle_bin);
        assert_eq!(personal.children.len(), 1);
        assert_eq!(personal.children[0].name, "Docs");
        assert_eq!(personal.children[0].count, 1);
        let bin = &sb.groups[1];
        assert!(bin.is_recycle_bin);
        assert_eq!(bin.count, 1);
        let mut db = s.db.clone();
        db.find_group_mut(s.personal)
            .unwrap()
            .set_custom(crate::fields::GROUP_EMOJI, "\u{1F3E0}");
        assert_eq!(sidebar(&db).groups[0].emoji.as_deref(), Some("\u{1F3E0}"));
    }

    #[test]
    fn recent_entries_orders_by_access_and_limits_to_five() {
        let mut db = fast_database("r");
        let root = db.root_id();
        let mut made = Vec::new();
        for i in 0..7 {
            let e = entry(&format!("e{i}"));
            made.push(e.uuid);
            db.add_entry(root, e, NOW).unwrap();
        }
        for (i, id) in made.iter().enumerate() {
            db.find_entry_mut(*id).unwrap().accessed = Timestamp(1000 + i as i64);
        }
        let mut bin_entry = entry("binned");
        bin_entry.accessed = Timestamp(99_999);
        let bin_id = bin_entry.uuid;
        db.add_entry(root, bin_entry, NOW).unwrap();
        db.delete_entry(bin_id, NOW).unwrap();
        db.find_entry_mut(bin_id).unwrap().accessed = Timestamp(99_999);
        let recent = recent_entries(&db, 5);
        assert_eq!(titles(&recent), ["e6", "e5", "e4", "e3", "e2"]);
        assert_eq!(recent_entries(&db, 2).len(), 2);
        assert!(recent_entries(&db, 0).is_empty());
    }

    #[test]
    fn reserved_keys_cover_all_app_fields() {
        for key in [
            "Password",
            "otp",
            "MyPass.CVV",
            "KPEX_PASSKEY_USERNAME",
            "Favorite",
            "KP2A_URL",
            "KP2A_URL_1",
            "MyPass.URIMatchMode",
            "MyPass.EntryType",
            "MyPass.Address.city",
        ] {
            assert!(Entry::is_reserved_key(key), "{key}");
        }
        assert!(!Entry::is_reserved_key("Recovery"));
        assert!(!Entry::is_reserved_key("MyPass.Address.nonsense"));
    }

    #[test]
    fn passkey_created_unparseable_is_none() {
        let mut db = fast_database("p");
        let root = db.root_id();
        let mut e = entry("pk");
        e.set_field(PASSKEY_CREDENTIAL_ID, "abc", true);
        e.set_field(PASSKEY_PRIVATE_KEY_PEM, "pem", true);
        e.set_field(
            crate::fields::PASSKEY_CREATED,
            "2023-11-14T22:13:20Z",
            false,
        );
        let id = e.uuid;
        db.add_entry(root, e, NOW).unwrap();
        let created = |db: &Database| entry_detail(db, id).unwrap().passkey.unwrap().created;
        assert_eq!(created(&db), Some(1_700_000_000_000.0));
        db.find_entry_mut(id).unwrap().set_field(
            crate::fields::PASSKEY_CREATED,
            "2023-02-31T00:00:00Z",
            false,
        );
        assert_eq!(created(&db), None);
    }

    #[test]
    fn address_entry_uses_given_and_family_name_when_full_name_is_empty() {
        let mut db = fast_database("a");
        let root = db.root_id();
        let mut e = typed(entry("addr"), "Address");
        e.set_field("MyPass.Address.givenName", "Ann", true);
        e.set_field("MyPass.Address.familyName", "Lee", true);
        e.set_field("MyPass.Address.label", "Vacation", true);
        let id = e.uuid;
        db.add_entry(root, e, NOW).unwrap();
        let d = entry_detail(&db, id).unwrap().address.unwrap();
        assert_eq!(d.full_name, "Ann Lee");
        assert_eq!(d.label, "Other");
        assert_eq!(
            entry_detail(&db, id).unwrap().entry_type,
            EntryTypeDto::Address
        );
        assert_eq!(
            list_entries(&db, &cat(Category::All, ""), SearchScope::App)[0].subtitle,
            "Other \u{b7} Ann Lee"
        );
        assert_eq!(e_type(&db, id), mypass_core::entry_ext::EntryType::Address);
    }

    fn e_type(db: &Database, id: Uuid) -> mypass_core::entry_ext::EntryType {
        db.find_entry(id).unwrap().entry_type()
    }
}
