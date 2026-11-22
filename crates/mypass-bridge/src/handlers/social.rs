//! `recordSocialSignIn` and `getSocialSignIn`: the "Sign in with ..." method remembered per site.
//!
//! Both answer `socialSignIn`. With the "Remember social sign-in" setting off they answer
//! `enabled: false` and neither read nor write the vault. The page URL must parse as `https` with
//! a host; anything else is refused before the vault is touched (the matcher would never match
//! it anyway, and a record must never create an entry for a plain-HTTP page).
//!
//! A record goes through one `session.mutate_tracked` call (all or nothing, refused while
//! quitting, autosaved). The target is the most recently used matching login (the
//! `credential_candidates` rules: PSL-aware, https only, no bin, generator history or `never`
//! mode). A host is written at most once a day, whatever the provider: a record on the host the
//! target's method was recorded on less than a day ago writes nothing (no history snapshot per
//! click, and a page cannot churn the entry), except a direct `click` naming another provider
//! than the stored one. With no matching login a `click` creates a new one in the root group:
//! title the registrable domain, URL the page origin, no user name or password. A `frame` record
//! (the heuristic for a provider's cross-origin button frame) never creates an entry.
//!
//! Every record stores the page host (`MyPass.SignIn.host`). A lookup prefers the matching login
//! recorded on exactly the page host and answers that host as `host`; the extension draws its
//! ring on a page only when `host` equals the page's own host (a sibling site under the same
//! registrable domain sees nothing), while its popup, which the page cannot observe, shows the
//! method for any matching login.

use mypass_core::entry_ext::{EntryExt, EntryType};
use mypass_core::matcher::{credential_candidates, registrable_domain_of};
use mypass_kdbx::{Database, Entry, Timestamp};
use mypass_session::fields::is_valid_user_target;
use mypass_session::sign_in::{
    last_used_rank, record_sign_in, sign_in_host, sign_in_of, SignIn, SignInProvider,
};
use url::Url;

use super::text::{is_clean_line, MAX_URL_CHARS};
use crate::protocol::{Response, SocialSignInSource};
use crate::service::Ctx;
use crate::views::{error, session_error_response};

/// A host recorded less than this many seconds ago is not written again (see the module doc).
const REFRESH_AFTER_SECS: i64 = 24 * 60 * 60;

/// An https page: its origin (`https://host[:port]`) and its lowercase host.
struct HttpsPage {
    origin: String,
    host: String,
}

/// The origin and host of an https URL with a host, or `None`.
fn https_page(page_url: &str) -> Option<HttpsPage> {
    if !is_clean_line(page_url, MAX_URL_CHARS) {
        return None;
    }
    let url = Url::parse(page_url.trim()).ok()?;
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())?
        .to_ascii_lowercase();
    if url.scheme() != "https" {
        return None;
    }
    Some(HttpsPage {
        origin: url.origin().ascii_serialization(),
        host,
    })
}

/// The matching logins in document order.
fn matching_logins<'a>(db: &'a Database, page_url: &str) -> Vec<&'a Entry> {
    credential_candidates(db, page_url)
        .into_iter()
        .filter(|e| e.entry_type() == EntryType::Login)
        .collect()
}

/// The most recently used entry; on a tie the first in document order.
fn most_recent<'a>(entries: impl DoubleEndedIterator<Item = &'a Entry>) -> Option<&'a Entry> {
    // `max_by_key` keeps the last maximum, so walk backwards to keep the first.
    entries.rev().max_by_key(|e| last_used_rank(e).0)
}

fn answer(sign_in: Option<SignIn>, host: Option<String>) -> Response {
    Response::SocialSignIn {
        enabled: true,
        provider: sign_in.map(|s| s.provider.as_str().to_owned()),
        last_used: sign_in
            .and_then(|s| s.last_used)
            .and_then(Timestamp::to_iso8601),
        host: sign_in.and(host),
    }
}

fn disabled() -> Response {
    Response::SocialSignIn {
        enabled: false,
        provider: None,
        last_used: None,
        host: None,
    }
}

/// The method of `entry` with its recorded host.
fn method_of(entry: &Entry) -> (Option<SignIn>, Option<String>) {
    (sign_in_of(entry), sign_in_host(entry).map(str::to_owned))
}

pub(crate) fn record_social_sign_in(
    ctx: &Ctx<'_>,
    page_url: &str,
    provider: &str,
    source: SocialSignInSource,
) -> Response {
    if !ctx.deps.settings.get().remember_social_sign_in {
        return disabled();
    }
    let Some(HttpsPage { origin, host }) = https_page(page_url) else {
        return error("social sign-in needs an https page");
    };
    let Some(provider) = SignInProvider::parse(provider) else {
        return error("unknown sign-in provider");
    };
    let page_url = page_url.trim();
    let result = ctx.deps.session.mutate_tracked(|db, now| {
        let target = most_recent(matching_logins(db, page_url).into_iter()).map(|e| e.uuid);
        if let Some(id) = target {
            let current = db.find_entry(id).and_then(sign_in_of);
            if let Some(SignIn {
                provider: stored,
                last_used: Some(when),
            }) = current
            {
                let same_host = db.find_entry(id).and_then(sign_in_host) == Some(host.as_str());
                let written_today = same_host && now.0.saturating_sub(when.0) < REFRESH_AFTER_SECS;
                let direct_change = source == SocialSignInSource::Click && stored != provider;
                if written_today && !direct_change {
                    return Ok((db.find_entry(id).map(method_of), false));
                }
            }
            let changed = db.update_entry(id, now, |e| record_sign_in(e, provider, &host, now))?;
            return Ok((db.find_entry(id).map(method_of), changed));
        }
        // The frame heuristic only ever updates a login that already exists.
        if source != SocialSignInSource::Click {
            return Ok((None, false));
        }
        let root = db.root_id();
        // The root is never the bin or a history group; the check keeps that true by construction.
        is_valid_user_target(db, root)?;
        let mut entry = Entry::new(now);
        let title = registrable_domain_of(&origin).unwrap_or_else(|| origin.clone());
        entry.set_field("Title", &title, false);
        entry.set_field("URL", &origin, false);
        record_sign_in(&mut entry, provider, &host, now);
        let stored = method_of(&entry);
        db.add_entry(root, entry, now)?;
        Ok((Some(stored), true))
    });
    match result {
        Ok(Some((stored, host))) => answer(stored, host),
        Ok(None) => answer(None, None),
        Err(e) => session_error_response(&e),
    }
}

pub(crate) fn get_social_sign_in(ctx: &Ctx<'_>, url: &str) -> Response {
    if !ctx.deps.settings.get().remember_social_sign_in {
        return disabled();
    }
    let Some(page) = https_page(url) else {
        return error("social sign-in needs an https page");
    };
    let url = url.trim();
    let found = ctx.deps.session.with_db(|db| {
        let with_method: Vec<&Entry> = matching_logins(db, url)
            .into_iter()
            .filter(|e| sign_in_of(e).is_some())
            .collect();
        // The method recorded on exactly this host wins: it is the only one the page may see.
        let on_this_host = most_recent(
            with_method
                .iter()
                .copied()
                .filter(|e| sign_in_host(e) == Some(page.host.as_str())),
        );
        on_this_host
            .or_else(|| most_recent(with_method.iter().copied()))
            .map(method_of)
    });
    match found {
        Ok(Some((sign_in, host))) => answer(sign_in, host),
        Ok(None) => answer(None, None),
        Err(_) => Response::Locked,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::protocol::Request;
    use crate::testing::{error_message, fx, resets_timer, sample_fx, send, Fx};
    use mypass_session::session::{LockReason, SaveState};
    use mypass_session::settings::SettingsPatch;
    use mypass_session::sign_in::{SIGN_IN_HOST, SIGN_IN_LAST_USED, SIGN_IN_PROVIDER};
    use std::time::Duration;
    use uuid::Uuid;

    fn record(f: &Fx, url: &str, provider: &str) -> Response {
        record_from(f, url, provider, SocialSignInSource::Click)
    }

    fn record_from(f: &Fx, url: &str, provider: &str, source: SocialSignInSource) -> Response {
        send(f, |token, pairing| Request::RecordSocialSignIn {
            token,
            page_url: url.to_owned(),
            provider: provider.to_owned(),
            source,
            pairing,
        })
    }

    fn get(f: &Fx, url: &str) -> Response {
        send(f, |token, pairing| Request::GetSocialSignIn {
            token,
            url: url.to_owned(),
            pairing,
        })
    }

    fn provider_of(r: &Response) -> Option<String> {
        let Response::SocialSignIn {
            enabled: true,
            provider,
            ..
        } = r
        else {
            panic!("expected an enabled SocialSignIn, got {r:?}");
        };
        provider.clone()
    }

    fn entry_count(f: &Fx) -> usize {
        f.session
            .with_db(|db| db.entries_excluding(&[]).len())
            .unwrap()
    }

    fn field(f: &Fx, id: Uuid, key: &str) -> Option<String> {
        f.session
            .with_db(|db| db.find_entry(id).unwrap().field(key).map(str::to_owned))
            .unwrap()
    }

    fn dirty(f: &Fx) -> bool {
        f.session.status().save.state != SaveState::Idle
    }

    fn history_len(f: &Fx, id: Uuid) -> usize {
        f.session
            .with_db(|db| db.find_entry(id).unwrap().history.len())
            .unwrap()
    }

    #[test]
    fn plain_http_and_malformed_pages_are_refused_without_a_write() {
        let (f, _) = sample_fx();
        let before = entry_count(&f);
        for url in [
            "http://github.com/login",
            "github.com/login",
            "javascript:alert(1)",
            "https://",
            "https://github.com/\u{202e}x",
            "",
        ] {
            let r = record(&f, url, "github");
            assert_eq!(
                error_message(&r),
                "social sign-in needs an https page",
                "{url}"
            );
            let r = get(&f, url);
            assert_eq!(
                error_message(&r),
                "social sign-in needs an https page",
                "{url}"
            );
        }
        assert_eq!(entry_count(&f), before);
        assert!(!dirty(&f));
    }

    #[test]
    fn unknown_providers_are_refused() {
        let (f, _) = sample_fx();
        for p in ["twitter", "Google", ""] {
            let r = record(&f, "https://github.com/login", p);
            assert_eq!(error_message(&r), "unknown sign-in provider", "{p}");
        }
        assert!(!dirty(&f));
    }

    #[test]
    fn records_on_the_psl_matching_login_and_reads_it_back() {
        let (f, ids) = sample_fx();
        let before = entry_count(&f);
        // A subdomain of the stored https://github.com matches through the registrable domain.
        let r = record(&f, "https://docs.github.com/en/login?next=/", "github");
        assert_eq!(provider_of(&r).as_deref(), Some("github"));
        assert_eq!(entry_count(&f), before, "an existing login is updated");
        assert_eq!(
            field(&f, ids.a, SIGN_IN_PROVIDER).as_deref(),
            Some("github")
        );
        assert_eq!(
            field(&f, ids.a, SIGN_IN_LAST_USED).as_deref(),
            Some("2023-11-14T22:13:20Z")
        );
        assert!(dirty(&f));
        // The `never`-mode, binned and history copies are never touched.
        for id in [ids.g, ids.h, ids.never] {
            assert_eq!(field(&f, id, SIGN_IN_PROVIDER), None);
        }
        let r = get(&f, "https://github.com/");
        assert_eq!(provider_of(&r).as_deref(), Some("github"));
        let Response::SocialSignIn { last_used, .. } = r else {
            unreachable!()
        };
        assert_eq!(last_used.as_deref(), Some("2023-11-14T22:13:20Z"));
        // A look-alike host is not the same site.
        let r = get(&f, "https://github.com.evil.example/");
        assert_eq!(provider_of(&r), None);
    }

    fn host_of(r: &Response) -> Option<String> {
        let Response::SocialSignIn { host, .. } = r else {
            panic!("expected SocialSignIn, got {r:?}");
        };
        host.clone()
    }

    #[test]
    fn a_record_stores_the_page_host_and_a_lookup_answers_it() {
        let (f, ids) = sample_fx();
        let r = record(&f, "https://Docs.GitHub.com/en/login", "github");
        assert_eq!(host_of(&r).as_deref(), Some("docs.github.com"));
        assert_eq!(
            field(&f, ids.a, SIGN_IN_HOST).as_deref(),
            Some("docs.github.com")
        );
        // The same host reads the host back, so the extension may ring the button there.
        let r = get(&f, "https://docs.github.com/en/login");
        assert_eq!(provider_of(&r).as_deref(), Some("github"));
        assert_eq!(host_of(&r).as_deref(), Some("docs.github.com"));
        // A sibling host still learns the method for the popup, with the host it was recorded
        // on, which is not its own: the extension never rings anything there.
        let r = get(&f, "https://github.com/");
        assert_eq!(provider_of(&r).as_deref(), Some("github"));
        assert_eq!(host_of(&r).as_deref(), Some("docs.github.com"));
    }

    #[test]
    fn a_method_picked_in_the_app_has_no_host() {
        let (f, ids) = sample_fx();
        f.session
            .mutate(|db, now| {
                db.update_entry(ids.a, now, |e| {
                    mypass_session::sign_in::set_sign_in_provider(e, Some(SignInProvider::Apple));
                })?;
                Ok(())
            })
            .unwrap();
        let r = get(&f, "https://github.com/");
        assert_eq!(provider_of(&r).as_deref(), Some("apple"));
        assert_eq!(host_of(&r), None);
    }

    #[test]
    fn the_method_recorded_on_the_page_host_wins_over_a_newer_sibling() {
        let (f, ids) = sample_fx();
        f.session
            .mutate(|db, now| {
                let mut e = Entry::new(now);
                e.set_field("Title", "second", false);
                e.set_field("URL", "https://www.example.org", false);
                record_sign_in(&mut e, SignInProvider::Github, "login.example.org", now);
                e.accessed = Timestamp(2_000_000_000);
                let id = e.uuid;
                db.add_entry(db.root_id(), e, now)?;
                db.update_entry(ids.b, now, |e| {
                    record_sign_in(
                        e,
                        SignInProvider::Google,
                        "www.example.org",
                        Timestamp(1_000),
                    );
                })?;
                Ok(id)
            })
            .unwrap();
        let r = get(&f, "https://www.example.org/signin");
        assert_eq!(provider_of(&r).as_deref(), Some("google"));
        assert_eq!(host_of(&r).as_deref(), Some("www.example.org"));
        let r = get(&f, "https://login.example.org/");
        assert_eq!(provider_of(&r).as_deref(), Some("github"));
        assert_eq!(host_of(&r).as_deref(), Some("login.example.org"));
    }

    #[test]
    fn with_no_matching_login_a_new_login_is_created_in_the_root() {
        let (f, _) = sample_fx();
        let before = entry_count(&f);
        let r = record(&f, "https://login.shop.example.co.uk:8443/start", "apple");
        assert_eq!(provider_of(&r).as_deref(), Some("apple"));
        assert_eq!(entry_count(&f), before + 1);
        let (title, url, user, pass, parent_is_root) = f
            .session
            .with_db(|db| {
                let e = db
                    .entries_excluding(&[])
                    .into_iter()
                    .find(|e| e.field(SIGN_IN_PROVIDER) == Some("apple"))
                    .unwrap();
                (
                    e.field("Title").map(str::to_owned),
                    e.field("URL").map(str::to_owned),
                    e.field("UserName").map(str::to_owned),
                    e.field("Password").map(str::to_owned),
                    db.parent_of_entry(e.uuid) == Some(db.root_id()),
                )
            })
            .unwrap();
        assert_eq!(title.as_deref(), Some("example.co.uk"));
        assert_eq!(
            url.as_deref(),
            Some("https://login.shop.example.co.uk:8443")
        );
        assert_eq!(user.filter(|u| !u.is_empty()), None);
        assert_eq!(pass.filter(|p| !p.is_empty()), None);
        assert!(parent_is_root);
        // The page reads it back.
        let r = get(&f, "https://login.shop.example.co.uk:8443/start");
        assert_eq!(provider_of(&r).as_deref(), Some("apple"));
    }

    #[test]
    fn the_most_recently_used_of_several_matching_logins_is_updated() {
        let (f, ids) = sample_fx();
        // A second example.org login, used after `b`.
        let newer = f
            .session
            .mutate(|db, now| {
                let mut e = Entry::new(now);
                e.set_field("Title", "second", false);
                e.set_field("URL", "https://www.example.org", false);
                let id = e.uuid;
                db.add_entry(db.root_id(), e, now)?;
                Ok(id)
            })
            .unwrap();
        f.session
            .mutate(|db, _| {
                db.find_entry_mut(ids.b).unwrap().accessed = Timestamp(1_000);
                db.find_entry_mut(ids.b).unwrap().modified = Timestamp(1_000);
                let e = db.find_entry_mut(newer).unwrap();
                e.accessed = Timestamp(2_000_000_000);
                Ok(())
            })
            .unwrap();
        record(&f, "https://example.org/signin", "facebook");
        assert_eq!(
            field(&f, newer, SIGN_IN_PROVIDER).as_deref(),
            Some("facebook")
        );
        assert_eq!(field(&f, ids.b, SIGN_IN_PROVIDER), None);
    }

    #[test]
    fn a_host_is_written_once_a_day_unless_a_click_changes_the_provider() {
        let (f, ids) = sample_fx();
        record(&f, "https://github.com/login", "github");
        let snapshots = history_len(&f, ids.a);
        f.session.flush().unwrap();
        assert!(!dirty(&f));
        f.sched.advance(Duration::from_secs(30));
        // The same provider again, by click or frame: nothing is written.
        record(&f, "https://github.com/login", "github");
        record_from(
            &f,
            "https://github.com/login",
            "github",
            SocialSignInSource::Frame,
        );
        assert!(!dirty(&f), "a repeat within a day is a no-op");
        assert_eq!(history_len(&f, ids.a), snapshots);
        // Another provider through the frame heuristic: the daily limit holds.
        let r = record_from(
            &f,
            "https://github.com/login",
            "google",
            SocialSignInSource::Frame,
        );
        assert_eq!(provider_of(&r).as_deref(), Some("github"));
        assert_eq!(
            field(&f, ids.a, SIGN_IN_PROVIDER).as_deref(),
            Some("github")
        );
        assert!(!dirty(&f));
        // Another provider through a direct click is written at once.
        record(&f, "https://github.com/login", "google");
        assert_eq!(
            field(&f, ids.a, SIGN_IN_PROVIDER).as_deref(),
            Some("google")
        );
        assert_eq!(history_len(&f, ids.a), snapshots + 1);
        // A day later the frame heuristic may refresh the entry.
        f.session.flush().unwrap();
        f.sched
            .advance(Duration::from_secs(REFRESH_AFTER_SECS.unsigned_abs() + 1));
        // The idle timer locked the vault meanwhile.
        if f.session.with_db(|_| ()).is_err() {
            f.session
                .unlock(secrecy::SecretString::from("pw".to_owned()))
                .unwrap();
        }
        record_from(
            &f,
            "https://github.com/login",
            "apple",
            SocialSignInSource::Frame,
        );
        assert_eq!(field(&f, ids.a, SIGN_IN_PROVIDER).as_deref(), Some("apple"));
    }

    #[test]
    fn another_host_of_the_site_is_not_limited_by_this_hosts_write() {
        let (f, ids) = sample_fx();
        record(&f, "https://github.com/login", "github");
        f.sched.advance(Duration::from_secs(30));
        record_from(
            &f,
            "https://gist.github.com/",
            "github",
            SocialSignInSource::Frame,
        );
        assert_eq!(
            field(&f, ids.a, SIGN_IN_HOST).as_deref(),
            Some("gist.github.com")
        );
    }

    #[test]
    fn the_frame_heuristic_never_creates_an_entry() {
        let (f, _) = sample_fx();
        let before = entry_count(&f);
        let r = record_from(
            &f,
            "https://new.example/login",
            "google",
            SocialSignInSource::Frame,
        );
        assert_eq!(provider_of(&r), None);
        assert_eq!(entry_count(&f), before);
        assert!(!dirty(&f));
        // A direct click on the same page does.
        record(&f, "https://new.example/login", "google");
        assert_eq!(entry_count(&f), before + 1);
    }

    #[test]
    fn the_setting_off_answers_disabled_and_touches_nothing() {
        let (f, ids) = sample_fx();
        f.settings
            .update(SettingsPatch {
                remember_social_sign_in: Some(false),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(record(&f, "https://github.com/login", "github"), disabled());
        assert_eq!(get(&f, "https://github.com/login"), disabled());
        // Off wins even over a malformed request: nothing is looked at.
        assert_eq!(record(&f, "http://x", "nope"), disabled());
        assert_eq!(field(&f, ids.a, SIGN_IN_PROVIDER), None);
        assert!(!dirty(&f));
    }

    #[test]
    fn both_requests_answer_locked_while_the_vault_is_locked() {
        let (f, ids) = sample_fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            record(&f, "https://github.com/login", "github"),
            Response::Locked
        );
        assert_eq!(get(&f, "https://github.com/login"), Response::Locked);
        f.session
            .unlock(secrecy::SecretString::from("pw".to_owned()))
            .unwrap();
        assert_eq!(field(&f, ids.a, SIGN_IN_PROVIDER), None);
    }

    #[test]
    fn only_the_record_resets_the_idle_timer() {
        let f = fx();
        assert!(resets_timer(&f, || {
            record(&f, "https://a.example/login", "google");
        }));
        let f = fx();
        assert!(!resets_timer(&f, || {
            get(&f, "https://a.example/login");
        }));
    }
}
