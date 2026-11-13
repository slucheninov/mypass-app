//! `getCredentials`, `getFavorites`, `listEntries`, `searchEntries`.
//!
//! Every handler reads through ONE `session.with_db` call, which takes the session's own lock: a
//! lock that races the request makes `with_db` fail and the answer is `Locked` with no data. The
//! gate's `is_unlocked` snapshot is never relied on for this.

use mypass_core::entry_ext::{EntryExt, EntryType, UriMatchMode};
use mypass_core::matcher::credential_candidates;
use mypass_kdbx::{Database, Entry};
use mypass_session::fields::{bin_id, is_history_group, EntryFields};
use mypass_session::queries::{entry_matches, SearchScope};

use crate::protocol::{ExtensionCredentialSummary, Response};
use crate::service::Ctx;
use crate::views::{browse_summary, error, summary};

/// The answer for a failed `with_db`: the vault is not unlocked any more.
fn rows(
    r: Result<Vec<ExtensionCredentialSummary>, mypass_session::session::SessionError>,
) -> Response {
    match r {
        Ok(credentials) => Response::CredentialList { credentials },
        Err(_) => Response::Locked,
    }
}

/// Every entry outside the recycle bin and the generator history group, with its group name, in
/// document order. Files entries are left out: the browser never lists, searches or fills them.
fn live_entries(db: &Database) -> Vec<(&Entry, &str)> {
    let bin = bin_id(db);
    let mut out = Vec::new();
    let mut stack = vec![&db.root];
    while let Some(g) = stack.pop() {
        if Some(g.uuid) == bin || is_history_group(g) {
            continue;
        }
        out.extend(
            g.entries
                .iter()
                .filter(|e| e.entry_type() != EntryType::Files)
                .map(|e| (e, g.name.as_str())),
        );
        stack.extend(g.groups.iter().rev());
    }
    out
}

fn sort_by_title(entries: &mut [&Entry]) {
    entries.sort_by_cached_key(|e| e.title().to_lowercase());
}

fn favorites(db: &Database) -> Vec<&Entry> {
    let mut v: Vec<&Entry> = live_entries(db)
        .into_iter()
        .map(|(e, _)| e)
        .filter(|e| e.is_favorite() && e.uri_match_mode() != UriMatchMode::Never)
        .collect();
    sort_by_title(&mut v);
    v
}

/// `None` for a category the popup does not offer.
fn category<'a>(db: &'a Database, name: &str) -> Option<Vec<&'a Entry>> {
    if name == "favorites" {
        return Some(favorites(db));
    }
    let keep: fn(&Entry) -> bool = match name {
        "all" => |_| true,
        "logins" => |e| e.entry_type() == EntryType::Login,
        "codes" => |e| e.otp_source().is_some(),
        "passkeys" => |e| e.has_passkey(),
        "addresses" => |e| e.entry_type() == EntryType::Address,
        "cards" => |e| e.entry_type() == EntryType::Card,
        "notes" => |e| e.entry_type() == EntryType::Notes,
        _ => return None,
    };
    let mut v: Vec<&Entry> = live_entries(db)
        .into_iter()
        .map(|(e, _)| e)
        .filter(|e| keep(e))
        .collect();
    sort_by_title(&mut v);
    Some(v)
}

pub(crate) fn get_credentials(ctx: &Ctx<'_>, url: &str) -> Response {
    rows(ctx.deps.session.with_db(|db| {
        credential_candidates(db, url)
            .into_iter()
            .map(summary)
            .collect()
    }))
}

pub(crate) fn get_favorites(ctx: &Ctx<'_>) -> Response {
    rows(
        ctx.deps
            .session
            .with_db(|db| favorites(db).into_iter().map(browse_summary).collect()),
    )
}

pub(crate) fn list_entries(ctx: &Ctx<'_>, name: &str) -> Response {
    match ctx
        .deps
        .session
        .with_db(|db| category(db, name).map(|v| v.into_iter().map(browse_summary).collect()))
    {
        Ok(Some(credentials)) => Response::CredentialList { credentials },
        Ok(None) => error("unknown category"),
        Err(_) => Response::Locked,
    }
}

pub(crate) fn search_entries(ctx: &Ctx<'_>, query: &str) -> Response {
    let q = query.trim();
    rows(ctx.deps.session.with_db(|db| {
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<&Entry> = live_entries(db)
            .into_iter()
            .filter(|(e, group)| entry_matches(e, group, q, SearchScope::Autofill))
            .map(|(e, _)| e)
            .collect();
        sort_by_title(&mut hits);
        hits.into_iter().map(browse_summary).collect()
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::protocol::Request;
    use crate::testing::{credential_rows, fx, sample_fx, send};
    use crate::views::wire_id;
    use mypass_session::session::LockReason;

    fn creds(url: &str) -> impl FnOnce(String, Option<String>) -> Request + '_ {
        move |token, pairing| Request::GetCredentials {
            token,
            url: url.into(),
            pairing,
        }
    }

    fn list(category: &'static str) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::ListEntries {
            token,
            category: category.into(),
            pairing,
        }
    }

    fn search(query: &'static str) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::SearchEntries {
            token,
            query: query.into(),
            pairing,
        }
    }

    fn favorites() -> impl FnOnce(String, Option<String>) -> Request {
        |token, pairing| Request::GetFavorites { token, pairing }
    }

    fn titles(r: &Response) -> Vec<String> {
        credential_rows(r).iter().map(|c| c.title.clone()).collect()
    }

    #[test]
    fn get_credentials_excludes_bin_address_files_history_and_never() {
        let (f, ids) = sample_fx();
        let r = send(&f, creds("https://github.com/login"));
        let rows = credential_rows(&r);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].id, wire_id(ids.a));
    }

    #[test]
    fn get_credentials_summary_has_totp_passkey_and_identifiers() {
        let (f, _) = sample_fx();
        let r = send(&f, creds("https://github.com/login"));
        let a = &credential_rows(&r)[0];
        assert!(a.has_totp);
        assert!(a.has_passkey);
        assert_eq!(a.identifiers, vec!["555-0100".to_owned()]);
        assert_eq!(a.username, "alice");
        assert_eq!(a.url, None);
        assert_eq!(a.entry_type, None);
    }

    #[test]
    fn get_credentials_never_contains_password_key() {
        let (f, _) = sample_fx();
        let r = send(&f, creds("https://github.com/login"));
        let text = serde_json::to_string(&r).unwrap();
        assert!(!text.to_lowercase().contains("password"));
        assert!(!text.contains(crate::testing::A_PASSWORD));
        assert!(!text.contains(crate::testing::A_OTP_SECRET));
        assert!(!text.contains("recovery-code-xyz"));
    }

    #[test]
    fn get_credentials_needs_an_https_page() {
        let (f, _) = sample_fx();
        let r = send(&f, creds("http://github.com/login"));
        assert!(credential_rows(&r).is_empty());
    }

    #[test]
    fn favorites_sorted_exclude_never_files_and_bin() {
        let (f, ids) = sample_fx();
        let r = send(&f, favorites());
        // `alpha` sorts before `GitHub` only with a case-insensitive sort.
        assert_eq!(titles(&r), ["alpha", "GitHub"]);
        let text = serde_json::to_string(&r).unwrap();
        for hidden in [ids.never, ids.f, ids.g, ids.h] {
            assert!(!text.contains(&wire_id(hidden)));
        }
    }

    #[test]
    fn favorites_row_carries_url_and_entry_type_and_omits_empty_url() {
        let (f, _) = sample_fx();
        let r = send(&f, favorites());
        let rows = credential_rows(&r);
        let github = rows.iter().find(|c| c.title == "GitHub").unwrap();
        assert_eq!(github.url.as_deref(), Some("https://github.com"));
        assert_eq!(github.entry_type.as_deref(), Some("Login"));
        // A browse row with an empty URL leaves the key out of the JSON.
        let r = send(&f, list("cards"));
        let json = serde_json::to_value(&r).unwrap();
        let card = &json["credentials"][0];
        assert!(card.get("url").is_none());
        assert_eq!(card["entryType"], "Card");
    }

    #[test]
    fn list_entries_all_categories_and_unknown_category_error() {
        let (f, _) = sample_fx();
        let count = |c: &'static str| credential_rows(&send(&f, list(c))).len();
        // Live entries: A, alpha, C, D, E and the never-mode login; no Files, bin or history.
        assert_eq!(count("all"), 6);
        assert_eq!(count("logins"), 3);
        assert_eq!(count("codes"), 1);
        assert_eq!(count("passkeys"), 1);
        assert_eq!(count("addresses"), 1);
        assert_eq!(count("cards"), 1);
        assert_eq!(count("notes"), 1);
        assert_eq!(count("favorites"), 2);
        // `all` keeps the never-mode login the favorites list hides.
        let r = send(&f, list("all"));
        assert!(titles(&r).contains(&"GitHub Never".to_owned()));
        assert_eq!(
            send(&f, list("bogus")),
            Response::Error {
                message: "unknown category".into()
            }
        );
    }

    #[test]
    fn list_entries_addresses_username_is_full_name() {
        let (f, ids) = sample_fx();
        let r = send(&f, list("addresses"));
        let rows = credential_rows(&r);
        assert_eq!(rows[0].id, wire_id(ids.c));
        assert_eq!(rows[0].username, "Alice Example");
        assert_eq!(rows[0].entry_type.as_deref(), Some("Address"));
    }

    #[test]
    fn list_entries_codes_uses_otp_field() {
        let (f, ids) = sample_fx();
        let r = send(&f, list("codes"));
        assert_eq!(credential_rows(&r)[0].id, wire_id(ids.a));
    }

    #[test]
    fn search_blank_empty_and_excludes_files_and_bin() {
        let (f, ids) = sample_fx();
        assert!(credential_rows(&send(&f, search("   "))).is_empty());
        let r = send(&f, search("github"));
        let text = serde_json::to_string(&r).unwrap();
        // `Documents` (Files) and the binned and history entries match by URL but never appear.
        assert!(text.contains(&wire_id(ids.a)));
        for hidden in [ids.f, ids.g, ids.h] {
            assert!(!text.contains(&wire_id(hidden)));
        }
    }

    #[test]
    fn search_is_case_insensitive_and_sorted() {
        let (f, _) = sample_fx();
        let r = send(&f, search("  GITHUB "));
        assert_eq!(titles(&r), ["GitHub", "GitHub Never", "Home Address"]);
    }

    #[test]
    fn search_matches_title_username_urls_never_notes() {
        let (f, ids) = sample_fx();
        let ids_of = |q: &'static str| -> Vec<String> {
            credential_rows(&send(&f, search(q)))
                .iter()
                .map(|c| c.id.clone())
                .collect()
        };
        assert_eq!(ids_of("alpha"), [wire_id(ids.b)]);
        assert_eq!(ids_of("bob"), [wire_id(ids.b)]);
        assert_eq!(ids_of("example.org"), [wire_id(ids.b)]);
        assert_eq!(ids_of("gist.github"), [wire_id(ids.a)]);
        // The note, the address city and the card number are never searched.
        assert!(ids_of("zebra-in-notes").is_empty());
        assert!(ids_of("Kyiv").is_empty());
        assert!(ids_of("4111").is_empty());
    }

    #[test]
    fn all_browse_requests_locked_when_locked() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        for r in [
            send(&f, creds("https://github.com")),
            send(&f, favorites()),
            send(&f, list("all")),
            send(&f, list("bogus")),
            send(&f, search("x")),
            send(&f, search("")),
        ] {
            assert_eq!(r, Response::Locked);
        }
    }

    #[test]
    fn handlers_answer_locked_when_the_session_locks_under_them() {
        // Called directly (no gate): the handlers rely on `with_db`, not on a snapshot.
        let (f, _) = sample_fx();
        f.session.lock(LockReason::Manual);
        let ctx = Ctx {
            deps: f.svc.deps_for_test(),
            now: std::time::SystemTime::now(),
        };
        assert_eq!(
            get_credentials(&ctx, "https://github.com"),
            Response::Locked
        );
        assert_eq!(get_favorites(&ctx), Response::Locked);
        assert_eq!(list_entries(&ctx, "all"), Response::Locked);
        assert_eq!(search_entries(&ctx, "git"), Response::Locked);
    }
}
