//! `addEntryIdentifier`: remembers an identifier (phone, email, login name) the user typed on a
//! page as part of the matching login.
//!
//! The lookup, the duplicate check and the write happen inside ONE `mutate_tracked` call, so the
//! entry cannot change between the check and the write, and an identifier that is already stored
//! (or an entry that does not match the page) writes nothing and does not touch the autosave.

use mypass_core::matcher::credential_candidates;
use mypass_kdbx::Entry;
use mypass_session::fields::EntryFields;
use uuid::Uuid;

use super::text::{is_clean_line, MAX_IDENTIFIER_CHARS};
use crate::protocol::Response;
use crate::service::Ctx;
use crate::views::{error, parse_wire_id, session_error_response};

const ALLOWED_KEYS: [&str; 3] = ["Phone", "Email", "Login"];
const NOT_FOUND: &str = "entry not found or domain mismatch";

/// The page rule of the extension: ignore case and every whitespace character.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// `Phone`, then `Phone 2`, `Phone 3`, ...: the first key no field of the entry uses.
fn next_key(entry: &Entry, base: &str) -> String {
    let taken = |key: &str| entry.fields.iter().any(|f| f.key == key);
    if !taken(base) {
        return base.to_owned();
    }
    (2_u32..)
        .map(|n| format!("{base} {n}"))
        .find(|key| !taken(key))
        .unwrap_or_else(|| base.to_owned())
}

/// Whether the username or any custom field (protected ones too) already holds `needle`.
fn already_present(entry: &Entry, needle: &str) -> bool {
    let username = entry.field("UserName").unwrap_or("");
    normalize(username) == needle
        || entry
            .fields
            .iter()
            .filter(|f| !Entry::is_reserved_key(&f.key))
            .any(|f| normalize(&f.value) == needle)
}

enum Plan {
    Skip,
    Username,
    Custom(String),
}

pub(crate) fn add_entry_identifier(
    ctx: &Ctx<'_>,
    id: &str,
    url: &str,
    value: &str,
    field_key: &str,
) -> Response {
    let trimmed = value.trim();
    let needle = normalize(trimmed);
    if needle.is_empty()
        || !is_clean_line(trimmed, MAX_IDENTIFIER_CHARS)
        || !ALLOWED_KEYS.contains(&field_key)
    {
        return error("identifier required");
    }
    let wanted: Option<Uuid> = parse_wire_id(id);
    let result = ctx.deps.session.mutate_tracked(|db, now| {
        let found = credential_candidates(db, url)
            .into_iter()
            .find(|e| Some(e.uuid) == wanted)
            .map(|e| {
                let plan = if already_present(e, &needle) {
                    Plan::Skip
                } else if e.field("UserName").unwrap_or("").trim().is_empty() {
                    Plan::Username
                } else {
                    Plan::Custom(next_key(e, field_key))
                };
                (e.uuid, plan)
            });
        let Some((uuid, plan)) = found else {
            return Ok((error(NOT_FOUND), false));
        };
        match plan {
            Plan::Skip => Ok((Response::Acknowledged, false)),
            Plan::Username => {
                let changed = db.update_entry(uuid, now, |e| {
                    let protected = e.fields.iter().any(|f| f.key == "UserName" && f.protected);
                    e.set_field("UserName", trimmed, protected);
                })?;
                Ok((Response::Acknowledged, changed))
            }
            Plan::Custom(key) => {
                let changed = db.update_entry(uuid, now, |e| e.set_field(&key, trimmed, false))?;
                Ok((Response::Acknowledged, changed))
            }
        }
    });
    result.unwrap_or_else(|e| session_error_response(&e))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::protocol::Request;
    use crate::testing::{error_message, sample_fx, send, Fx, Ids};
    use crate::views::wire_id;
    use mypass_session::session::{LockReason, SaveState};

    const GITHUB: &str = "https://github.com/login";
    const EXAMPLE: &str = "https://example.org/";

    fn add(
        id: Uuid,
        url: &'static str,
        value: &str,
        key: &str,
    ) -> impl FnOnce(String, Option<String>) -> Request {
        let (value, key) = (value.to_owned(), key.to_owned());
        move |token, pairing| Request::AddEntryIdentifier {
            token,
            id: wire_id(id),
            url: url.into(),
            value,
            field_key: key,
            pairing,
        }
    }

    fn entry(f: &Fx, id: Uuid) -> Entry {
        f.session
            .with_db(|db| db.find_entry(id).unwrap().clone())
            .unwrap()
    }

    fn state(f: &Fx) -> SaveState {
        f.session.status().save.state
    }

    fn custom(e: &Entry) -> Vec<(String, String, bool)> {
        e.fields
            .iter()
            .filter(|f| !Entry::is_reserved_key(&f.key))
            .map(|f| (f.key.clone(), f.value.clone(), f.protected))
            .collect()
    }

    #[test]
    fn identifier_validation_and_custom_field_key_numbering() {
        let (f, Ids { b, .. }) = sample_fx();
        let bad = |value: &str, key: &str| {
            let r = send(&f, add(b, EXAMPLE, value, key));
            assert_eq!(error_message(&r), "identifier required", "{value:?} {key}");
        };
        bad(&"x".repeat(201), "Phone");
        bad("a\u{0007}b", "Phone");
        bad("a\u{200D}b", "Phone");
        bad("  ", "Phone");
        bad("555", "Fax");
        bad("555", "phone");
        bad("555", "");
        assert_eq!(state(&f), SaveState::Idle);
        let ok = |value: &str, key: &str| {
            assert_eq!(
                send(&f, add(b, EXAMPLE, value, key)),
                Response::Acknowledged
            );
        };
        ok(&"x".repeat(200), "Email");
        ok("555-0001", "Phone");
        ok("555-0002", "Phone");
        ok("555-0003", "Phone");
        let names: Vec<String> = custom(&entry(&f, b)).into_iter().map(|c| c.0).collect();
        assert_eq!(names, ["Email", "Phone", "Phone 2", "Phone 3"]);
        let values: Vec<String> = custom(&entry(&f, b)).into_iter().map(|c| c.1).collect();
        assert_eq!(values[2], "555-0002");
        assert!(custom(&entry(&f, b)).iter().all(|c| !c.2));
    }

    #[test]
    fn identifier_trims_the_stored_value() {
        let (f, Ids { b, .. }) = sample_fx();
        assert_eq!(
            send(&f, add(b, EXAMPLE, "  +380 50 000  ", "Phone")),
            Response::Acknowledged
        );
        assert_eq!(custom(&entry(&f, b))[0].1, "+380 50 000");
    }

    #[test]
    fn identifier_empty_username_becomes_login_value() {
        let (f, _) = sample_fx();
        let mut n = crate::testing::named("createEntry", Some(&f.secret));
        if let Request::CreateEntry {
            title,
            url,
            username,
            entry_type,
            otp,
            r#match,
            ..
        } = &mut n
        {
            *entry_type = "Login".into();
            *title = "Shop".into();
            *url = "https://shop.example.com".into();
            *username = String::new();
            *otp = String::new();
            *r#match = String::new();
        }
        assert_eq!(f.svc.handle(n), Response::EntryCreated);
        let id = f
            .session
            .with_db(|db| {
                db.root
                    .entries
                    .iter()
                    .find(|e| e.title() == "Shop")
                    .unwrap()
                    .uuid
            })
            .unwrap();
        let r = send(
            &f,
            add(id, "https://shop.example.com/", " me@example.com ", "Email"),
        );
        assert_eq!(r, Response::Acknowledged);
        let e = entry(&f, id);
        assert_eq!(e.field("UserName"), Some("me@example.com"));
        assert!(custom(&e).is_empty());
    }

    #[test]
    fn identifier_already_present_in_username_or_custom_field_is_acknowledged_without_write() {
        let (f, Ids { a, .. }) = sample_fx();
        // Wait for nothing to be pending, then count on the state staying Idle.
        assert_eq!(state(&f), SaveState::Idle);
        let before = entry(&f, a);
        for value in [
            "ALICE",
            " a l i c e ",
            "555-0100",
            "  555-0100",
            // A protected custom field counts as present too.
            " Recovery-Code-XYZ ",
        ] {
            assert_eq!(
                send(&f, add(a, GITHUB, value, "Phone")),
                Response::Acknowledged,
                "{value:?}"
            );
        }
        assert_eq!(state(&f), SaveState::Idle);
        assert_eq!(entry(&f, a).fields, before.fields);
        assert!(entry(&f, a).history.is_empty());
    }

    #[test]
    fn identifier_snapshots_history() {
        let (f, Ids { a, .. }) = sample_fx();
        let before = entry(&f, a);
        assert!(before.history.is_empty());
        assert_eq!(
            send(&f, add(a, GITHUB, "555-0199", "Phone")),
            Response::Acknowledged
        );
        let after = entry(&f, a);
        assert_eq!(after.history.len(), 1);
        assert_eq!(after.history[0].fields, before.fields);
        assert_eq!(
            custom(&after).iter().filter(|c| c.0 == "Phone 2").count(),
            1
        );
        assert_eq!(state(&f), SaveState::Pending);
    }

    #[test]
    fn identifier_domain_mismatch_error() {
        let (f, Ids { a, c, .. }) = sample_fx();
        let msg = "entry not found or domain mismatch";
        for (id, url) in [
            (a, "https://evil.example/"),
            (a, "http://github.com/"),
            (c, GITHUB), // an address is not offered for sign-in
        ] {
            let r = send(&f, add(id, url, "555-0777", "Phone"));
            assert_eq!(error_message(&r), msg, "{url}");
        }
        let r = send(&f, add(Uuid::nil(), GITHUB, "555-0777", "Phone"));
        assert_eq!(error_message(&r), msg);
        assert_eq!(state(&f), SaveState::Idle);
    }

    #[test]
    fn identifier_locked_and_quitting() {
        let (f, Ids { a, .. }) = sample_fx();
        f.session.begin_quit();
        let r = send(&f, add(a, GITHUB, "555-0777", "Phone"));
        assert_eq!(error_message(&r), "MyPass is quitting");
        f.session.cancel_quit();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            send(&f, add(a, GITHUB, "555-0777", "Phone")),
            Response::Locked
        );
    }
}
