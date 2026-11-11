//! RFC 4180 CSV import of logins (Chrome, Bitwarden and similar exports). Replaces the naive
//! split-on-comma importer of the Swift app.
//!
//! The parser is a hand-written state machine over a `&str`: no `csv` crate, no regex. A CSV file
//! is hostile input, so it never panics, never indexes unchecked, and bounds its work: the facade
//! reads at most [`MAX_CSV_BYTES`] and [`parse_csv`] itself refuses more. Every row is read once,
//! in linear time; an unterminated quote consumes the rest of the file as one skipped row.
//!
//! Errors and skip reports name a column or a line, never a cell value: the cells are passwords.

pub use crate::dto::{CsvPreview, ImportResult, SkipReason, SkippedRow};
use crate::editing::put;
use crate::fields::GENERATOR_HISTORY_MARKER;
use crate::session::{SessionError, VaultSession};
use mypass_core::entry_ext::{EntryExt, GENERATOR_HISTORY_GROUP_UUID};
use mypass_kdbx::{Entry, Group};
use uuid::Uuid;
use zeroize::Zeroize;

/// The largest CSV file the facade reads and [`parse_csv`] accepts (16 MiB, tens of thousands of
/// logins): a password export is kilobytes to a few megabytes, so more is a wrong file.
pub const MAX_CSV_BYTES: usize = 16 * 1024 * 1024;

/// The most fields one record may have. A 16 MiB row of commas would otherwise build millions of
/// empty strings; real exports have a handful of columns.
const MAX_FIELDS_PER_RECORD: usize = 256;

/// How many titles a [`CsvPreview`] carries.
const PREVIEW_TITLES: usize = 5;

/// One login read from a CSV row. Not an IPC type (it carries a password): only [`CsvPreview`]
/// and [`ImportResult`] cross into the webview. `Debug` is redacted and the password is wiped on
/// drop (copies made while parsing cannot be reached, so this is best effort).
#[derive(Clone, PartialEq)]
pub struct ImportedLogin {
    pub title: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub notes: String,
}

impl std::fmt::Debug for ImportedLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImportedLogin(..)")
    }
}

impl Drop for ImportedLogin {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

/// The rows that were read and the rows that were skipped (with the reason and line).
#[derive(Debug)]
pub struct CsvParse {
    pub logins: Vec<ImportedLogin>,
    pub skipped: Vec<SkippedRow>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CsvImportError {
    #[error("file is not valid UTF-8")]
    NotUtf8,
    #[error("missing required column {0}")]
    MissingRequiredColumn(String),
    #[error("file is empty")]
    Empty,
    #[error("file is too large")]
    TooLarge,
}

impl CsvPreview {
    /// The first five titles and the counts; never a password.
    pub fn from_parse(parse: &CsvParse) -> Self {
        Self {
            total: count_u32(parse.logins.len()),
            skipped: parse.skipped.clone(),
            sample_titles: parse
                .logins
                .iter()
                .take(PREVIEW_TITLES)
                .map(|l| l.title.clone())
                .collect(),
        }
    }
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// One record as the tokenizer sees it.
struct Record {
    /// 1-based physical line on which the record starts.
    line: u32,
    /// `None` when a quote was never closed.
    fields: Option<Vec<String>>,
    /// More than [`MAX_FIELDS_PER_RECORD`] fields: the extra ones were not kept.
    too_wide: bool,
    /// The record was a blank line (a single empty unquoted field).
    blank: bool,
}

/// Splits `text` into records, tracking physical lines.
struct Tokenizer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: u32,
}

impl<'a> Tokenizer<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            chars: text.chars().peekable(),
            line: 1,
        }
    }

    fn bump_line(&mut self) {
        self.line = self.line.saturating_add(1);
    }

    fn next_record(&mut self) -> Option<Record> {
        self.chars.peek()?;
        let line = self.line;
        let mut fields: Vec<String> = Vec::new();
        let mut cell = String::new();
        let mut in_quotes = false;
        let mut quoted_seen = false;
        let mut too_wide = false;
        // True until the first character of a field is read: only there does `"` open a quote.
        let mut at_start = true;
        loop {
            let Some(c) = self.chars.next() else {
                if in_quotes {
                    return Some(Record {
                        line,
                        fields: None,
                        too_wide,
                        blank: false,
                    });
                }
                break;
            };
            if in_quotes {
                match c {
                    '"' if self.chars.peek() == Some(&'"') => {
                        self.chars.next();
                        cell.push('"');
                    }
                    '"' => in_quotes = false,
                    '\n' => {
                        self.bump_line();
                        cell.push(c);
                    }
                    _ => cell.push(c),
                }
                continue;
            }
            match c {
                ',' => {
                    // Past the cap the record is already lost; keep reading to its end (so the
                    // line count stays right) but stop accumulating.
                    if fields.len() < MAX_FIELDS_PER_RECORD {
                        fields.push(std::mem::take(&mut cell));
                    } else {
                        too_wide = true;
                        cell.clear();
                    }
                    at_start = true;
                }
                '\n' => {
                    self.bump_line();
                    break;
                }
                '\r' if self.chars.peek() == Some(&'\n') => {
                    self.chars.next();
                    self.bump_line();
                    break;
                }
                // A quote opens a quoted field only at the start of a field; elsewhere it is
                // data (lenient, like most exporters' readers).
                '"' if at_start => {
                    in_quotes = true;
                    quoted_seen = true;
                    at_start = false;
                }
                _ => {
                    cell.push(c);
                    at_start = false;
                }
            }
        }
        let blank = fields.is_empty() && cell.is_empty() && !quoted_seen;
        if fields.len() < MAX_FIELDS_PER_RECORD {
            fields.push(cell);
        } else {
            too_wide = true;
        }
        Some(Record {
            line,
            fields: Some(fields),
            too_wide,
            blank,
        })
    }
}

/// Column positions of the fields the importer reads.
struct Columns {
    width: usize,
    title: usize,
    username: usize,
    password: usize,
    url: Option<usize>,
    notes: Option<usize>,
}

/// The first column named `name`, else the first named `alias`.
fn column(header: &[String], name: &str, alias: Option<&str>) -> Option<usize> {
    let find = |wanted: &str| header.iter().position(|h| h == wanted);
    find(name).or_else(|| alias.and_then(find))
}

fn read_columns(raw: &[String]) -> Result<Columns, CsvImportError> {
    let header: Vec<String> = raw.iter().map(|h| h.trim().to_lowercase()).collect();
    let required = |name: &str, alias: Option<&str>| {
        column(&header, name, alias)
            .ok_or_else(|| CsvImportError::MissingRequiredColumn(name.to_owned()))
    };
    Ok(Columns {
        width: header.len(),
        title: required("title", Some("name"))?,
        username: required("username", None)?,
        password: required("password", None)?,
        url: column(&header, "url", Some("login_uri")),
        notes: column(&header, "notes", Some("note")),
    })
}

fn cell(fields: &mut [String], index: Option<usize>) -> String {
    index
        .and_then(|i| fields.get_mut(i))
        .map(std::mem::take)
        .unwrap_or_default()
}

/// Parses an RFC 4180 CSV export: UTF-8 with an optional BOM, `,` separator, quoted fields with
/// commas, newlines and `""`. Header names are trimmed and lower-cased; `title`, `username` and
/// `password` are required, `url` and `notes` optional (aliases `name`, `login_uri`, `note`),
/// other columns are ignored. A data row whose field count differs from the header, or whose quote
/// is never closed, is skipped and reported with its line. Blank lines are ignored.
pub fn parse_csv(bytes: &[u8]) -> Result<CsvParse, CsvImportError> {
    if bytes.len() > MAX_CSV_BYTES {
        return Err(CsvImportError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| CsvImportError::NotUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut tokens = Tokenizer::new(text);

    let header = loop {
        match tokens.next_record() {
            None => return Err(CsvImportError::Empty),
            Some(r) if r.blank => {}
            // An unterminated header quote leaves no usable header.
            Some(Record { fields: None, .. }) | Some(Record { too_wide: true, .. }) => {
                return Err(CsvImportError::MissingRequiredColumn("title".into()))
            }
            Some(Record {
                fields: Some(f), ..
            }) => break f,
        }
    };
    let cols = read_columns(&header)?;

    let mut logins = Vec::new();
    let mut skipped = Vec::new();
    while let Some(record) = tokens.next_record() {
        if record.blank {
            continue;
        }
        let line = record.line;
        let reason = match record.fields {
            None => Some(SkipReason::UnterminatedQuote),
            Some(_) if record.too_wide => Some(SkipReason::FieldCountMismatch),
            Some(ref f) if f.len() != cols.width => Some(SkipReason::FieldCountMismatch),
            Some(_) => None,
        };
        let Some(mut fields) = record.fields else {
            skipped.push(SkippedRow {
                line,
                reason: SkipReason::UnterminatedQuote,
            });
            continue;
        };
        if let Some(reason) = reason {
            // A skipped row may still hold a password.
            fields.iter_mut().for_each(Zeroize::zeroize);
            skipped.push(SkippedRow { line, reason });
            continue;
        }
        logins.push(ImportedLogin {
            title: cell(&mut fields, Some(cols.title)),
            url: cell(&mut fields, cols.url),
            username: cell(&mut fields, Some(cols.username)),
            password: cell(&mut fields, Some(cols.password)),
            notes: cell(&mut fields, cols.notes),
        });
        // Whatever is left (unused columns, a duplicated header name) may still hold secrets.
        fields.iter_mut().for_each(Zeroize::zeroize);
    }
    Ok(CsvParse { logins, skipped })
}

/// `ancestor` is the group `id` or has it somewhere below.
fn contains_group(ancestor: &Group, id: Uuid) -> bool {
    ancestor.uuid == id || ancestor.groups.iter().any(|g| contains_group(g, id))
}

/// `id` lies in (or is) a generator history group somewhere under `group`.
fn in_history_tree(group: &Group, id: Uuid) -> bool {
    if group.uuid == GENERATOR_HISTORY_GROUP_UUID
        || group.custom(GENERATOR_HISTORY_MARKER).is_some()
    {
        return contains_group(group, id);
    }
    group.groups.iter().any(|g| in_history_tree(g, id))
}

/// Adds every login to `group` in one mutation: all of them or none. Each is a Login with a fresh
/// UUID and the mutation's time, its password protected like the editor's. Importing into a
/// missing group is `NotFound`; the recycle bin and the generator history group are not valid
/// targets (`InvalidInput`). Returns how many entries were added.
pub fn import_logins(
    session: &VaultSession,
    group: Uuid,
    logins: Vec<ImportedLogin>,
) -> Result<u32, SessionError> {
    if logins.is_empty() {
        // Nothing to write must not dirty the vault (and does not need the group either).
        return Ok(0);
    }
    session.mutate(|db, now| {
        db.find_group(group).ok_or(SessionError::NotFound)?;
        // The bin and the generator history group, and everything nested in either, are off limits.
        let in_bin = db
            .meta
            .recycle_bin_uuid
            .and_then(|id| db.find_group(id))
            .is_some_and(|bin| contains_group(bin, group));
        let in_history = in_history_tree(&db.root, group);
        if in_bin || in_history {
            return Err(SessionError::InvalidInput(
                "this group cannot receive imported logins".into(),
            ));
        }
        for login in &logins {
            let mut entry = Entry::new(now);
            put(&mut entry, "Title", &login.title, false, true);
            put(&mut entry, "UserName", &login.username, false, false);
            put(&mut entry, "Password", &login.password, true, false);
            put(&mut entry, "Notes", &login.notes, false, false);
            let url = login.url.trim();
            if !url.is_empty() {
                entry.set_urls(&[url.to_owned()]);
            }
            db.add_entry(group, entry, now)?;
        }
        Ok(count_u32(logins.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::ManualScheduler;
    use crate::session::{SaveState, SessionDeps};
    use crate::settings::SettingsStore;
    use crate::testing::fast_database;
    use mypass_core::entry_ext::EntryExt;
    use mypass_kdbx::{Database, PlainFileIo};
    use secrecy::SecretString;
    use std::sync::Arc;
    use std::time::{Duration, Instant, UNIX_EPOCH};

    fn parse(text: &str) -> CsvParse {
        parse_csv(text.as_bytes()).unwrap()
    }

    fn one(text: &str) -> ImportedLogin {
        let mut p = parse(text);
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);
        assert_eq!(p.logins.len(), 1);
        p.logins.remove(0)
    }

    #[test]
    fn quoted_field_with_comma_is_one_field() {
        let l = one("title,url,username,password\n\"Acme, Inc\",https://a.test,bob,pw");
        assert_eq!(l.title, "Acme, Inc");
        assert_eq!(l.url, "https://a.test");
        assert_eq!(l.username, "bob");
        assert_eq!(l.password, "pw");
    }

    #[test]
    fn escaped_quotes_decode() {
        let l = one("title,username,password\n\"He said \"\"hi\"\"\",bob,pw");
        assert_eq!(l.title, "He said \"hi\"");
    }

    #[test]
    fn embedded_newline_in_notes_kept() {
        let l = one("title,username,password,notes\r\nT,bob,pw,\"line1\r\nline2\"\r\n");
        assert_eq!(l.notes, "line1\r\nline2");
        let l = one("title,username,password,notes\nT,bob,pw,\"a\nb\"\n");
        assert_eq!(l.notes, "a\nb");
    }

    #[test]
    fn password_with_quote_and_comma_survives() {
        let l = one("title,username,password\nT,bob,\"p,\"\"w\"");
        assert_eq!(l.password, "p,\"w");
    }

    #[test]
    fn bom_and_header_case_and_spaces_tolerated() {
        let l = one("\u{feff} Title , USERNAME ,Password\nT,bob,pw");
        assert_eq!((l.title.as_str(), l.username.as_str()), ("T", "bob"));
        assert_eq!(l.password, "pw");
    }

    #[test]
    fn aliases_name_note_login_uri() {
        let l = one("name,login_uri,username,password,note\nT,https://x.test,u,p,n");
        assert_eq!(l.title, "T");
        assert_eq!(l.url, "https://x.test");
        assert_eq!(l.notes, "n");
    }

    #[test]
    fn unknown_columns_ignored() {
        let l = one("folder,title,favorite,username,password,totp\nF,T,1,u,p,JBSW");
        assert_eq!((l.title.as_str(), l.username.as_str()), ("T", "u"));
        assert_eq!(l.url, "");
    }

    #[test]
    fn row_with_wrong_field_count_skipped_with_line_number() {
        let p = parse("title,username,password\nA,u,p\nB,u\nC,u,p\n");
        assert_eq!(p.logins.len(), 2);
        assert_eq!(
            p.skipped,
            vec![SkippedRow {
                line: 3,
                reason: SkipReason::FieldCountMismatch
            }]
        );
    }

    #[test]
    fn line_numbers_are_physical_start_lines_across_multiline_rows() {
        let p = parse("title,username,password,notes\nA,u,p,\"x\ny\nz\"\nB,u\n");
        assert_eq!(p.logins.len(), 1);
        assert_eq!(p.skipped.len(), 1);
        assert_eq!(p.skipped.first().map(|s| s.line), Some(5));
    }

    #[test]
    fn unterminated_quote_row_skipped_not_panicking() {
        let p = parse("title,username,password\nA,u,p\nB,u,\"oops\nC,u,p\n");
        assert_eq!(p.logins.len(), 1);
        assert_eq!(
            p.skipped,
            vec![SkippedRow {
                line: 3,
                reason: SkipReason::UnterminatedQuote
            }]
        );
    }

    #[test]
    fn missing_password_column_error() {
        assert_eq!(
            parse_csv(b"title,username\nA,u").unwrap_err(),
            CsvImportError::MissingRequiredColumn("password".into())
        );
    }

    #[test]
    fn non_utf8_is_not_utf8_error() {
        assert_eq!(
            parse_csv(&[0x74, 0xff, 0xfe]).unwrap_err(),
            CsvImportError::NotUtf8
        );
    }

    #[test]
    fn empty_file_is_empty_error() {
        assert_eq!(parse_csv(b"").unwrap_err(), CsvImportError::Empty);
        assert_eq!(
            parse_csv("\u{feff}".as_bytes()).unwrap_err(),
            CsvImportError::Empty
        );
        assert_eq!(parse_csv(b"\n\r\n").unwrap_err(), CsvImportError::Empty);
    }

    #[test]
    fn header_only_yields_zero_logins() {
        let p = parse("title,username,password\n");
        assert!(p.logins.is_empty() && p.skipped.is_empty());
        let p = parse("title,username,password");
        assert!(p.logins.is_empty() && p.skipped.is_empty());
    }

    #[test]
    fn blank_lines_between_rows_are_ignored() {
        let p = parse("title,username,password\n\nA,u,p\n\r\n\nB,u,p\n");
        assert_eq!(p.logins.len(), 2);
        assert!(p.skipped.is_empty());
    }

    #[test]
    fn oversized_input_is_refused_without_parsing() {
        let big = vec![b'a'; MAX_CSV_BYTES + 1];
        assert_eq!(parse_csv(&big).unwrap_err(), CsvImportError::TooLarge);
    }

    #[test]
    fn debug_of_a_login_never_prints_values() {
        let l = one("title,username,password,notes\nTitleX,UserX,PassX,NoteX");
        let text = format!("{l:?}");
        for v in ["TitleX", "UserX", "PassX", "NoteX"] {
            assert!(!text.contains(v), "{v}");
        }
    }

    #[test]
    fn errors_never_carry_cell_content() {
        let p = parse("title,username,password\nSECRETCELL,u\n");
        assert!(!format!("{:?}", p.skipped).contains("SECRETCELL"));
    }

    #[test]
    fn ten_thousand_rows_parse_under_one_second() {
        let mut text = String::from("title,url,username,password,notes\n");
        for i in 0..10_000 {
            text.push_str(&format!("T{i},https://e{i}.test,u{i},\"p,\"\"{i}\",n\n"));
        }
        let start = Instant::now();
        let p = parse(&text);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(p.logins.len(), 10_000);
        assert!(p.skipped.is_empty());
    }

    #[test]
    fn preview_has_first_five_titles_and_total() {
        let mut text = String::from("title,username,password\n");
        for i in 0..8 {
            text.push_str(&format!("T{i},u,p\n"));
        }
        text.push_str("bad\n");
        let preview = CsvPreview::from_parse(&parse(&text));
        assert_eq!(preview.total, 8);
        assert_eq!(preview.sample_titles, vec!["T0", "T1", "T2", "T3", "T4"]);
        assert_eq!(preview.skipped.len(), 1);
    }

    struct Fx {
        _dir: tempfile::TempDir,
        session: VaultSession,
        root: Uuid,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let session = VaultSession::new(SessionDeps {
            settings: SettingsStore::in_memory(),
            scheduler: ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        let db: Database = fast_database("T");
        let root = db.root_id();
        session
            .create_vault_with(
                &dir.path().join("v.kdbx"),
                SecretString::from("pw".to_owned()),
                db,
            )
            .unwrap();
        Fx {
            _dir: dir,
            session,
            root,
        }
    }

    fn login(n: usize) -> ImportedLogin {
        ImportedLogin {
            title: format!("T{n}"),
            url: format!("https://e{n}.test"),
            username: format!("u{n}"),
            password: format!("p{n}"),
            notes: format!("n{n}"),
        }
    }

    #[test]
    fn import_logins_is_one_mutation_and_creates_logins() {
        let f = fx();
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        let n = import_logins(&f.session, f.root, vec![login(1), login(2), login(3)]).unwrap();
        assert_eq!(n, 3);
        assert_eq!(f.session.status().save.state, SaveState::Pending);
        f.session
            .with_db(|d| {
                let root = d.find_group(f.root).unwrap();
                assert_eq!(root.entries.len(), 3);
                let mut ids = std::collections::HashSet::new();
                for e in &root.entries {
                    assert!(ids.insert(e.uuid));
                    assert_eq!(e.entry_type(), mypass_core::entry_ext::EntryType::Login);
                    let pw = e.fields.iter().find(|f| f.key == "Password").unwrap();
                    assert!(pw.protected);
                    assert!(e.field("Title").unwrap().starts_with('T'));
                    assert!(e.field("UserName").unwrap().starts_with('u'));
                    assert!(e.field("Notes").unwrap().starts_with('n'));
                    assert_eq!(e.urls().len(), 1);
                    assert_eq!(e.created.0, 1_700_000_000);
                }
            })
            .unwrap();
    }

    #[test]
    fn import_into_missing_group_not_found_and_nothing_added() {
        let f = fx();
        let r = import_logins(&f.session, Uuid::new_v4(), vec![login(1)]);
        assert_eq!(r, Err(SessionError::NotFound));
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        let count = f
            .session
            .with_db(|d| d.entries_excluding(&[]).len())
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn bin_and_generator_history_are_not_import_targets() {
        let f = fx();
        let bin = f
            .session
            .mutate(|db, now| {
                let g = mypass_kdbx::Group::new("Bin", now);
                let id = g.uuid;
                db.add_group(db.root_id(), g, now)?;
                db.meta.recycle_bin_uuid = Some(id);
                Ok(id)
            })
            .unwrap();
        assert!(matches!(
            import_logins(&f.session, bin, vec![login(1)]),
            Err(SessionError::InvalidInput(_))
        ));
        let hist = f
            .session
            .mutate(|db, now| {
                let mut g = mypass_kdbx::Group::new("H", now);
                g.set_custom(crate::fields::GENERATOR_HISTORY_MARKER, "1");
                let id = g.uuid;
                db.add_group(db.root_id(), g, now)?;
                Ok(id)
            })
            .unwrap();
        assert!(matches!(
            import_logins(&f.session, hist, vec![login(1)]),
            Err(SessionError::InvalidInput(_))
        ));
    }

    #[test]
    fn subgroups_of_the_bin_and_history_are_not_import_targets() {
        let f = fx();
        let (bin_child, hist_child) = f
            .session
            .mutate(|db, now| {
                let root = db.root_id();
                let bin = mypass_kdbx::Group::new("Bin", now);
                let bin_id = bin.uuid;
                db.add_group(root, bin, now)?;
                db.meta.recycle_bin_uuid = Some(bin_id);
                let c1 = mypass_kdbx::Group::new("InBin", now);
                let c1_id = c1.uuid;
                db.add_group(bin_id, c1, now)?;
                let mut hist = mypass_kdbx::Group::new("H", now);
                hist.set_custom(crate::fields::GENERATOR_HISTORY_MARKER, "1");
                let hist_id = hist.uuid;
                db.add_group(root, hist, now)?;
                let c2 = mypass_kdbx::Group::new("InHist", now);
                let c2_id = c2.uuid;
                db.add_group(hist_id, c2, now)?;
                Ok((c1_id, c2_id))
            })
            .unwrap();
        for target in [bin_child, hist_child] {
            assert!(matches!(
                import_logins(&f.session, target, vec![login(1)]),
                Err(SessionError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn row_of_100_000_commas_is_skipped_and_the_next_row_imports() {
        let mut text = String::from("title,username,password\nA,u,p\n");
        text.push_str(&",".repeat(100_000));
        text.push_str("\nB,u,p\n");
        let p = parse(&text);
        assert_eq!(p.logins.len(), 2);
        assert_eq!(
            p.skipped,
            vec![SkippedRow {
                line: 3,
                reason: SkipReason::FieldCountMismatch
            }]
        );
        let header = format!("title,username,password{}\nA,u,p\n", ",".repeat(100_000));
        assert!(matches!(
            parse_csv(header.as_bytes()),
            Err(CsvImportError::MissingRequiredColumn(_))
        ));
    }

    #[test]
    fn importing_nothing_does_not_dirty_the_vault() {
        let f = fx();
        assert_eq!(import_logins(&f.session, f.root, Vec::new()), Ok(0));
        assert_eq!(f.session.status().save.state, SaveState::Idle);
    }
}
