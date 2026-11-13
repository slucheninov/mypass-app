//! `createEntry`.
//!
//! The entry is appended to the root group through `session.mutate`, so it is all or nothing, it
//! refuses while the app is quitting and it marks the session dirty (autosave). It never goes
//! into the recycle bin or the generator history group. Every string is length-capped and checked
//! for control and format characters before anything is written.

use mypass_core::entry_ext::{EntryExt, UriMatchMode, ENTRY_TYPE_FIELD};
use mypass_core::matcher::host_of;
use mypass_core::TotpConfig;
use mypass_kdbx::Entry;
use mypass_session::fields::{
    is_valid_user_target, EntryFields, CARD_CVV, CARD_EXPIRY, CARD_HOLDER, CARD_NUMBER, NOTES_BODY,
    OTP,
};
use mypass_session::session::SessionError;

use super::text::{
    is_clean_line, is_clean_text, MAX_CARDHOLDER_CHARS, MAX_CARD_NUMBER_CHARS, MAX_CVV_CHARS,
    MAX_EXPIRY_CHARS, MAX_MATCH_CHARS, MAX_NOTES_CHARS, MAX_OTP_CHARS, MAX_PASSWORD_CHARS,
    MAX_TITLE_CHARS, MAX_URL_CHARS, MAX_USERNAME_CHARS,
};
use crate::protocol::Response;
use crate::service::Ctx;
use crate::views::{error, session_error_response};

/// The fields of a `createEntry` request.
pub(crate) struct CreateEntryArgs<'a> {
    pub entry_type: &'a str,
    pub title: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    pub url: &'a str,
    pub card_number: &'a str,
    pub cardholder_name: &'a str,
    pub expiry: &'a str,
    pub cvv: &'a str,
    pub notes: &'a str,
    pub favorite: bool,
    pub match_mode: &'a str,
    pub otp: &'a str,
}

/// The entry kinds the extension may name; `Address` and `Files` are parsed only to be refused.
enum Kind {
    Login,
    Card,
    Notes,
    Address,
    Files,
}

impl Kind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "Login" => Some(Self::Login),
            "Card" => Some(Self::Card),
            "Notes" => Some(Self::Notes),
            "Address" => Some(Self::Address),
            "Files" => Some(Self::Files),
            _ => None,
        }
    }
}

/// Every string fits its cap and its character rule.
fn fields_are_valid(a: &CreateEntryArgs<'_>) -> bool {
    is_clean_line(a.title, MAX_TITLE_CHARS)
        && is_clean_line(a.username, MAX_USERNAME_CHARS)
        && is_clean_text(a.password, MAX_PASSWORD_CHARS, false)
        && is_clean_line(a.url, MAX_URL_CHARS)
        && is_clean_line(a.card_number, MAX_CARD_NUMBER_CHARS)
        && is_clean_line(a.cardholder_name, MAX_CARDHOLDER_CHARS)
        && is_clean_line(a.expiry, MAX_EXPIRY_CHARS)
        && is_clean_line(a.cvv, MAX_CVV_CHARS)
        && is_clean_text(a.notes, MAX_NOTES_CHARS, true)
        && is_clean_line(a.match_mode, MAX_MATCH_CHARS)
        && is_clean_line(a.otp, MAX_OTP_CHARS)
}

pub(crate) fn create_entry(ctx: &Ctx<'_>, args: &CreateEntryArgs<'_>) -> Response {
    let Some(kind) = Kind::parse(args.entry_type) else {
        return error("unknown entry type");
    };
    match kind {
        Kind::Address => return error("Create addresses in MyPass using Add > Address"),
        Kind::Files => return error("Create files in MyPass using Add > Files"),
        Kind::Login | Kind::Card | Kind::Notes => {}
    }
    if !fields_are_valid(args) {
        return error("invalid request");
    }
    let title = args.title.trim();
    let holder = args.cardholder_name.trim();
    let url = args.url.trim();
    let resolved = match kind {
        Kind::Login if !title.is_empty() => title.to_owned(),
        Kind::Login => host_of(url).unwrap_or_default(),
        Kind::Card if !title.is_empty() => title.to_owned(),
        Kind::Card => holder.to_owned(),
        Kind::Notes | Kind::Address | Kind::Files => title.to_owned(),
    };
    if resolved.is_empty() {
        return error("title required");
    }
    let otp = args.otp.trim();
    if matches!(kind, Kind::Login) && !otp.is_empty() && TotpConfig::parse(otp).is_none() {
        return error("invalid otp");
    }

    let result = ctx.deps.session.mutate(|db, now| {
        let root = db.root_id();
        // The root is never the bin or a history group; the check keeps that true by construction.
        is_valid_user_target(db, root)?;
        let mut entry = Entry::new(now);
        entry.set_field("Title", &resolved, false);
        if args.favorite {
            entry.set_favorite(true);
        }
        match kind {
            Kind::Login => {
                set_if_present(&mut entry, "UserName", args.username, false);
                set_if_present(&mut entry, "Password", args.password, true);
                set_if_present(&mut entry, "URL", url, false);
                set_if_present(&mut entry, "Notes", args.notes, false);
                if let Some(mode) = UriMatchMode::parse(args.match_mode.trim()) {
                    entry.set_uri_match_mode(mode);
                }
                set_if_present(&mut entry, OTP, otp, true);
            }
            Kind::Card => {
                entry.set_field(ENTRY_TYPE_FIELD, "Card", false);
                let number: String = args
                    .card_number
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect();
                set_if_present(&mut entry, CARD_HOLDER, holder, false);
                set_if_present(&mut entry, CARD_NUMBER, &number, true);
                set_if_present(&mut entry, CARD_EXPIRY, args.expiry.trim(), false);
                set_if_present(&mut entry, CARD_CVV, args.cvv.trim(), true);
                set_if_present(&mut entry, "UserName", holder, false);
            }
            Kind::Notes => {
                entry.set_field(ENTRY_TYPE_FIELD, "Notes", false);
                set_if_present(&mut entry, NOTES_BODY, args.notes, false);
            }
            Kind::Address | Kind::Files => {
                return Err(SessionError::InvalidInput("unsupported entry type".into()));
            }
        }
        db.add_entry(root, entry, now)?;
        Ok(())
    });
    match result {
        Ok(()) => Response::EntryCreated,
        Err(e) => session_error_response(&e),
    }
}

/// Stores `value` unless it is empty (an absent field and an empty one read the same).
fn set_if_present(entry: &mut Entry, key: &str, value: &str, protected: bool) {
    if !value.is_empty() {
        entry.set_field(key, value, protected);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::protocol::Request;
    use crate::testing::{fx, send, Fx};
    use mypass_core::matcher::credential_candidates;
    use mypass_session::session::{LockReason, SaveState};

    /// A `createEntry` request with every field empty except the ones set by `f`.
    #[derive(Clone, Default)]
    struct New {
        entry_type: &'static str,
        title: String,
        username: String,
        password: String,
        url: String,
        card_number: String,
        cardholder_name: String,
        expiry: String,
        cvv: String,
        notes: String,
        favorite: bool,
        r#match: String,
        otp: String,
    }

    fn new(entry_type: &'static str) -> New {
        New {
            entry_type,
            ..New::default()
        }
    }

    fn create(f: &Fx, n: &New) -> Response {
        let n = n.clone();
        send(f, move |token, pairing| Request::CreateEntry {
            token,
            entry_type: n.entry_type.into(),
            title: n.title,
            username: n.username,
            password: n.password,
            url: n.url,
            card_number: n.card_number,
            cardholder_name: n.cardholder_name,
            expiry: n.expiry,
            cvv: n.cvv,
            notes: n.notes,
            favorite: n.favorite,
            r#match: n.r#match,
            otp: n.otp,
            pairing,
        })
    }

    fn root_entries(f: &Fx) -> Vec<Entry> {
        f.session.with_db(|db| db.root.entries.clone()).unwrap()
    }

    fn created(f: &Fx, title: &str) -> Entry {
        root_entries(f)
            .into_iter()
            .find(|e| e.title() == title)
            .unwrap_or_else(|| panic!("no entry titled {title}"))
    }

    fn msg(r: &Response) -> &str {
        crate::testing::error_message(r)
    }

    #[test]
    fn create_entry_rejects_address_and_files_with_exact_messages() {
        let f = fx();
        let mut n = new("Address");
        n.title = "Home".into();
        assert_eq!(
            msg(&create(&f, &n)),
            "Create addresses in MyPass using Add > Address"
        );
        n.entry_type = "Files";
        assert_eq!(
            msg(&create(&f, &n)),
            "Create files in MyPass using Add > Files"
        );
        assert_eq!(root_entries(&f).len(), 1);
    }

    #[test]
    fn create_entry_unknown_type_error() {
        let f = fx();
        for t in ["Passkey", "login", ""] {
            let n = new(t);
            assert_eq!(msg(&create(&f, &n)), "unknown entry type", "{t:?}");
        }
        assert_eq!(root_entries(&f).len(), 1);
    }

    #[test]
    fn create_login_title_falls_back_to_url_host() {
        let f = fx();
        let mut n = new("Login");
        n.url = "https://www.example.com/x".into();
        n.username = "u".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        let e = created(&f, "www.example.com");
        assert_eq!(e.field("URL"), Some("https://www.example.com/x"));
        assert_eq!(e.field("UserName"), Some("u"));
    }

    #[test]
    fn create_login_with_blank_title_and_url_is_title_required() {
        let f = fx();
        let mut n = new("Login");
        n.title = "   ".into();
        assert_eq!(msg(&create(&f, &n)), "title required");
        n.url = "   ".into();
        assert_eq!(msg(&create(&f, &n)), "title required");
        for t in ["Card", "Notes"] {
            assert_eq!(msg(&create(&f, &new(t))), "title required", "{t}");
        }
        assert_eq!(root_entries(&f).len(), 1);
    }

    #[test]
    fn create_login_trims_the_title_and_stores_password_protected() {
        let f = fx();
        let mut n = new("Login");
        n.title = "  Shop  ".into();
        n.password = "s3cret".into();
        n.notes = "line one\nline two".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        let e = created(&f, "Shop");
        let pw = e.fields.iter().find(|x| x.key == "Password").unwrap();
        assert_eq!((pw.value.as_str(), pw.protected), ("s3cret", true));
        assert_eq!(e.field("Notes"), Some("line one\nline two"));
        assert_eq!(e.entry_type(), mypass_core::entry_ext::EntryType::Login);
    }

    #[test]
    fn create_card_title_falls_back_to_cardholder_and_number_keeps_digits_only() {
        let f = fx();
        let mut n = new("Card");
        n.cardholder_name = "  Alice Example ".into();
        n.card_number = "4111 1111-1111 1111".into();
        n.expiry = "12/30".into();
        n.cvv = "123".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        let e = created(&f, "Alice Example");
        assert_eq!(e.entry_type(), mypass_core::entry_ext::EntryType::Card);
        assert_eq!(e.field(CARD_NUMBER), Some("4111111111111111"));
        assert_eq!(e.field(CARD_HOLDER), Some("Alice Example"));
        assert_eq!(e.field("UserName"), Some("Alice Example"));
        assert_eq!(e.field(CARD_EXPIRY), Some("12/30"));
        assert_eq!(e.field(CARD_CVV), Some("123"));
        let protected = |k: &str| e.fields.iter().find(|x| x.key == k).unwrap().protected;
        assert!(protected(CARD_NUMBER) && protected(CARD_CVV));
        assert!(!protected(CARD_HOLDER) && !protected(CARD_EXPIRY));
    }

    #[test]
    fn create_notes_stores_body_field() {
        let f = fx();
        let mut n = new("Notes");
        n.title = "Journal".into();
        n.notes = "body\twith tab\nand line".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        let e = created(&f, "Journal");
        assert_eq!(e.entry_type(), mypass_core::entry_ext::EntryType::Notes);
        assert_eq!(e.field(NOTES_BODY), Some("body\twith tab\nand line"));
        assert!(e.field("Notes").is_none());
    }

    #[test]
    fn create_login_stores_otp_and_match_mode() {
        let f = fx();
        let mut n = new("Login");
        n.title = "Host".into();
        n.r#match = "host".into();
        n.otp = " JBSWY3DPEHPK3PXP ".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        let e = created(&f, "Host");
        assert_eq!(e.uri_match_mode(), UriMatchMode::Host);
        assert_eq!(e.field(OTP), Some("JBSWY3DPEHPK3PXP"));
        assert!(e.fields.iter().find(|x| x.key == OTP).unwrap().protected);
        assert!(e.totp().is_some());
        for (title, mode) in [("Bogus", "bogus"), ("Blank", "  "), ("Empty", "")] {
            let mut n = new("Login");
            n.title = title.into();
            n.r#match = mode.into();
            assert_eq!(create(&f, &n), Response::EntryCreated);
            let e = created(&f, title);
            assert_eq!(e.uri_match_mode(), UriMatchMode::RegistrableDomain);
            assert!(e
                .field(mypass_core::entry_ext::URI_MATCH_MODE_FIELD)
                .is_none());
        }
    }

    #[test]
    fn create_login_with_garbage_otp_is_invalid_otp() {
        let f = fx();
        let mut n = new("Login");
        n.title = "Bad".into();
        n.otp = "not base32 !!".into();
        assert_eq!(msg(&create(&f, &n)), "invalid otp");
        assert_eq!(root_entries(&f).len(), 1);
        // A card or a note ignores the field instead of rejecting the request.
        let mut n = new("Notes");
        n.title = "Note".into();
        n.otp = "garbage".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        assert!(created(&f, "Note").field(OTP).is_none());
    }

    #[test]
    fn create_entry_sets_favorite() {
        let f = fx();
        let mut n = new("Login");
        n.title = "Fav".into();
        n.favorite = true;
        assert_eq!(create(&f, &n), Response::EntryCreated);
        assert!(created(&f, "Fav").is_favorite());
        n.title = "Plain".into();
        n.favorite = false;
        assert_eq!(create(&f, &n), Response::EntryCreated);
        assert!(!created(&f, "Plain").is_favorite());
    }

    #[test]
    fn create_entry_marks_dirty_and_schedules_save() {
        let f = fx();
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        let mut n = new("Notes");
        n.title = "N".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        assert_eq!(f.session.status().save.state, SaveState::Pending);
    }

    #[test]
    fn create_entry_locked_is_locked_and_writes_nothing() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        let mut n = new("Notes");
        n.title = "N".into();
        assert_eq!(create(&f, &n), Response::Locked);
        // The handler itself also answers Locked when the lock races the gate.
        let ctx = f.svc.ctx_for_test();
        let args = CreateEntryArgs {
            entry_type: "Notes",
            title: "N",
            username: "",
            password: "",
            url: "",
            card_number: "",
            cardholder_name: "",
            expiry: "",
            cvv: "",
            notes: "",
            favorite: false,
            match_mode: "",
            otp: "",
        };
        assert_eq!(create_entry(&ctx, &args), Response::Locked);
    }

    #[test]
    fn create_entry_is_refused_while_quitting_and_writes_nothing() {
        let f = fx();
        f.session.begin_quit();
        let mut n = new("Notes");
        n.title = "N".into();
        assert_eq!(msg(&create(&f, &n)), "MyPass is quitting");
        // Past the gate, `mutate` itself refuses.
        let ctx = f.svc.ctx_for_test();
        let args = CreateEntryArgs {
            entry_type: "Notes",
            title: "N",
            username: "",
            password: "",
            url: "",
            card_number: "",
            cardholder_name: "",
            expiry: "",
            cvv: "",
            notes: "",
            favorite: false,
            match_mode: "",
            otp: "",
        };
        assert_eq!(msg(&create_entry(&ctx, &args)), "MyPass is quitting");
        f.session.cancel_quit();
        assert_eq!(root_entries(&f).len(), 1);
    }

    #[test]
    fn create_entry_rejects_overlong_and_unsafe_strings_and_writes_nothing() {
        let f = fx();
        let base = || {
            let mut n = new("Login");
            n.title = "T".into();
            n
        };
        let cases: Vec<(&str, New)> = vec![
            ("title too long", {
                let mut n = base();
                n.title = "x".repeat(MAX_TITLE_CHARS + 1);
                n
            }),
            ("title control", {
                let mut n = base();
                n.title = "a\u{0007}b".into();
                n
            }),
            ("title bidi override", {
                let mut n = base();
                n.title = "a\u{202E}b".into();
                n
            }),
            ("username newline", {
                let mut n = base();
                n.username = "a\nb".into();
                n
            }),
            ("password too long", {
                let mut n = base();
                n.password = "x".repeat(MAX_PASSWORD_CHARS + 1);
                n
            }),
            ("password nul", {
                let mut n = base();
                n.password = "a\u{0000}b".into();
                n
            }),
            ("url too long", {
                let mut n = base();
                n.url = "x".repeat(MAX_URL_CHARS + 1);
                n
            }),
            ("notes too long", {
                let mut n = base();
                n.notes = "x".repeat(MAX_NOTES_CHARS + 1);
                n
            }),
            ("notes control", {
                let mut n = base();
                n.notes = "a\u{0001}b".into();
                n
            }),
            ("otp too long", {
                let mut n = base();
                n.otp = "x".repeat(MAX_OTP_CHARS + 1);
                n
            }),
            ("card number too long", {
                let mut n = new("Card");
                n.title = "C".into();
                n.card_number = "1".repeat(MAX_CARD_NUMBER_CHARS + 1);
                n
            }),
            ("cvv control", {
                let mut n = new("Card");
                n.title = "C".into();
                n.cvv = "1\u{0000}".into();
                n
            }),
        ];
        for (name, n) in cases {
            assert_eq!(msg(&create(&f, &n)), "invalid request", "{name}");
        }
        assert_eq!(root_entries(&f).len(), 1);
        // The caps are inclusive: a value at the limit is stored.
        let mut n = base();
        n.title = "x".repeat(MAX_TITLE_CHARS);
        n.notes = "a\r\n\tb".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
    }

    #[test]
    fn created_entries_go_to_the_root_never_the_bin_or_the_history_group() {
        let (f, _) = crate::testing::sample_fx();
        let mut n = new("Login");
        n.title = "Fresh".into();
        n.url = "https://fresh.example".into();
        assert_eq!(create(&f, &n), Response::EntryCreated);
        let (in_root, in_bin, in_history, offered) = f
            .session
            .with_db(|db| {
                let e = db.root.entries.iter().find(|e| e.title() == "Fresh");
                let id = e.map(|e| e.uuid);
                (
                    e.is_some(),
                    id.is_some_and(|id| db.is_in_recycle_bin(id)),
                    id.is_some_and(|id| {
                        db.parent_of_entry(id)
                            == Some(mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID)
                    }),
                    credential_candidates(db, "https://fresh.example/login").len(),
                )
            })
            .unwrap();
        assert!(in_root && !in_bin && !in_history);
        assert_eq!(offered, 1);
    }
}
