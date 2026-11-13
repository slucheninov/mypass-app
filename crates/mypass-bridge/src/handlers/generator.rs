//! `generatePassword` and the generated-password history.
//!
//! Generation uses the app's own generator preferences (`Settings.generator`) and the bundled site
//! rules; nothing is read from the vault. The history lives in the hidden generator group, which
//! credential matching, search and the lists never offer.

use mypass_kdbx::Timestamp;
use mypass_session::dto::GenerateRequest;
use mypass_session::generator_service::{
    generate_password as generate, history_delete, history_list, history_reveal, record_history,
    record_history_at,
};
use mypass_session::session::{GeneratorRulesError, SessionError};
use uuid::Uuid;

use super::text::{is_clean_line, is_clean_text, MAX_PASSWORD_CHARS, MAX_URL_CHARS};
use crate::protocol::{ExtensionPasswordHistoryItem, Response};
use crate::service::Ctx;
use crate::views::{error, parse_wire_id, session_error_response, wire_id};

/// The latest creation time the extension may report: year 9999 in Unix milliseconds.
const MAX_CREATED_AT_MS: f64 = 253_402_300_799_000.0;

/// The fixed text for a generator failure. Exhaustive on purpose: a new reason must pick a text.
pub(crate) fn generator_error_message(e: &GeneratorRulesError) -> &'static str {
    match e {
        GeneratorRulesError::NoClass => {
            "Your password generator has no character types enabled. Turn on at least one in Settings and try again."
        }
        GeneratorRulesError::Unsatisfiable => {
            "This site's password requirements could not be satisfied"
        }
    }
}

pub(crate) fn generate_password(ctx: &Ctx<'_>, domain: &str) -> Response {
    if !ctx.deps.session.is_unlocked() {
        return Response::Locked;
    }
    let request = GenerateRequest {
        options: ctx.deps.settings.get().generator,
        host: Some(domain.trim().to_owned()),
        record_url: None,
    };
    match generate(None, &request) {
        Ok(generated) => Response::GeneratedPassword {
            password: generated.value,
        },
        Err(SessionError::GeneratorRules(reason)) => error(generator_error_message(&reason)),
        Err(_) => error("internal error"),
    }
}

pub(crate) fn record_generated_password(
    ctx: &Ctx<'_>,
    password: &str,
    url: &str,
    created_at: Option<f64>,
) -> Response {
    let password = password.trim();
    if password.is_empty() {
        return error("password required");
    }
    if !is_clean_text(password, MAX_PASSWORD_CHARS, false) || !is_clean_line(url, MAX_URL_CHARS) {
        return error("invalid request");
    }
    let now_secs = ctx
        .now
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0);
    let created = match created_at {
        None => None,
        Some(ms) if ms.is_finite() && (0.0..=MAX_CREATED_AT_MS).contains(&ms) => {
            // A future time would pin the row at the top and out of the purge: never later than now.
            Some(Timestamp(((ms / 1000.0).floor() as i64).min(now_secs)))
        }
        Some(_) => return error("invalid request"),
    };
    let result = match created {
        Some(at) => record_history_at(&ctx.deps.session, password, url, at),
        None => record_history(&ctx.deps.session, password, url),
    };
    match result {
        Ok(_) => Response::Acknowledged,
        Err(e) => session_error_response(&e),
    }
}

pub(crate) fn get_generated_password_history(ctx: &Ctx<'_>) -> Response {
    // One read of the session: the list and every password come from the same snapshot.
    let rows = ctx.deps.session.with_db(|db| {
        history_list(db)
            .into_iter()
            .filter_map(|item| {
                let id = Uuid::parse_str(&item.id).ok()?;
                let password = history_reveal(db, id).ok()?;
                Some(ExtensionPasswordHistoryItem {
                    id: wire_id(id),
                    password,
                    url: item.url,
                    created_at: item.created,
                })
            })
            .collect::<Vec<_>>()
    });
    match rows {
        Ok(entries) => Response::PasswordHistory { entries },
        Err(e) => session_error_response(&e),
    }
}

pub(crate) fn delete_generated_password(ctx: &Ctx<'_>, id: &str) -> Response {
    let Some(id) = parse_wire_id(id) else {
        return error("entry not found");
    };
    match history_delete(&ctx.deps.session, id) {
        Ok(()) => Response::Acknowledged,
        Err(e) => session_error_response(&e),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::protocol::Request;
    use crate::testing::{error_message, fx, named, sample_fx, send, Fx};
    use mypass_session::session::LockReason;
    use mypass_session::settings::{GeneratorOptionsDto, SettingsPatch};
    use std::time::Duration;

    fn gen(domain: &'static str) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::GeneratePassword {
            token,
            domain: domain.into(),
            pairing,
        }
    }

    fn record(
        password: &str,
        url: &str,
        created_at: Option<f64>,
    ) -> impl FnOnce(String, Option<String>) -> Request {
        let (password, url) = (password.to_owned(), url.to_owned());
        move |token, pairing| Request::RecordGeneratedPassword {
            token,
            password,
            url,
            created_at,
            pairing,
        }
    }

    fn history(f: &Fx) -> Vec<ExtensionPasswordHistoryItem> {
        match f
            .svc
            .handle(named("getGeneratedPasswordHistory", Some(&f.secret)))
        {
            Response::PasswordHistory { entries } => entries,
            other => panic!("expected history, got {other:?}"),
        }
    }

    fn set_length(f: &Fx, length: u32) {
        f.svc
            .ctx_for_test()
            .deps
            .settings
            .update(SettingsPatch {
                generator: Some(GeneratorOptionsDto {
                    length,
                    ..GeneratorOptionsDto::default()
                }),
                ..SettingsPatch::default()
            })
            .unwrap();
    }

    fn password_of(r: &Response) -> &str {
        let Response::GeneratedPassword { password } = r else {
            panic!("expected GeneratedPassword, got {r:?}");
        };
        password
    }

    fn created_secs(f: &Fx, id: &str) -> i64 {
        let id = parse_wire_id(id).unwrap();
        f.session
            .with_db(|db| db.find_entry(id).unwrap().created.0)
            .unwrap()
    }

    #[test]
    fn generate_password_uses_site_rules_and_settings() {
        let f = fx();
        // The default preference is 20 characters.
        let r = send(&f, gen("unknown.example"));
        assert_eq!(password_of(&r).chars().count(), 20);
        set_length(&f, 32);
        let r = send(&f, gen("unknown.example"));
        assert_eq!(password_of(&r).chars().count(), 32);
        // A site rule caps the length, whatever the preference says.
        let r = send(&f, gen("account.samsung.com"));
        assert_eq!(password_of(&r).chars().count(), 15);
    }

    #[test]
    fn generate_password_works_unlocked_only() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(send(&f, gen("example.com")), Response::Locked);
        // Past the gate the handler refuses too.
        assert_eq!(
            generate_password(&f.svc.ctx_for_test(), "example.com"),
            Response::Locked
        );
    }

    #[test]
    fn generate_password_no_classes_error_exact_text() {
        // Settings refuse a generator with no class, so the handler cannot be driven there; the
        // texts are pinned through the mapping the handler uses.
        let f = fx();
        let none = GeneratorOptionsDto {
            lowercase: false,
            uppercase: false,
            digits: false,
            symbols: false,
            ..GeneratorOptionsDto::default()
        };
        assert!(f
            .svc
            .ctx_for_test()
            .deps
            .settings
            .update(SettingsPatch {
                generator: Some(none),
                ..SettingsPatch::default()
            })
            .is_err());
        assert_eq!(
            generator_error_message(&GeneratorRulesError::NoClass),
            "Your password generator has no character types enabled. Turn on at least one in Settings and try again."
        );
    }

    #[test]
    fn generate_password_unsatisfiable_rules_error_exact_text() {
        assert_eq!(
            generator_error_message(&GeneratorRulesError::Unsatisfiable),
            "This site's password requirements could not be satisfied"
        );
    }

    #[test]
    fn generator_rules_error_mapping_is_exhaustive() {
        // `generator_error_message` matches without a wildcard; both reasons have distinct text.
        let a = generator_error_message(&GeneratorRulesError::NoClass);
        let b = generator_error_message(&GeneratorRulesError::Unsatisfiable);
        assert_ne!(a, b);
        assert!(include_str!("generator.rs")
            .split("pub(crate) fn generator_error_message")
            .nth(1)
            .unwrap()
            .split("pub(crate) fn generate_password")
            .next()
            .unwrap()
            .lines()
            .all(|l| !l.trim_start().starts_with("_ =>")));
    }

    #[test]
    fn record_generated_password_blank_createdat_means_now() {
        let f = fx();
        f.sched.advance(Duration::from_secs(20));
        assert_eq!(
            send(&f, record("  pw-now  ", "https://example.com/a", None)),
            Response::Acknowledged
        );
        let rows = history(&f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].password, "pw-now");
        assert_eq!(rows[0].url, "https://example.com/a");
        assert_eq!(created_secs(&f, &rows[0].id), 1_700_000_020);
    }

    #[test]
    fn record_generated_password_uses_epoch_milliseconds() {
        let f = fx();
        send(
            &f,
            record("pw", "https://example.com", Some(1_700_000_000_000.0)),
        );
        send(
            &f,
            record("old", "https://example.com", Some(1_600_000_123_456.0)),
        );
        let rows = history(&f);
        let secs: Vec<i64> = rows.iter().map(|r| created_secs(&f, &r.id)).collect();
        assert_eq!(secs, [1_700_000_000, 1_600_000_123]);
        // A time in the future is held at now; junk is refused.
        send(
            &f,
            record("future", "https://example.com", Some(2_000_000_000_000.0)),
        );
        let id = history(&f)[0].id.clone();
        assert_eq!(created_secs(&f, &id), 1_700_000_000);
        for bad in [f64::NAN, f64::INFINITY, -1.0, 1e18] {
            let r = send(&f, record("x", "https://example.com", Some(bad)));
            assert_eq!(error_message(&r), "invalid request");
        }
        assert_eq!(history(&f).len(), 3);
    }

    #[test]
    fn record_generated_password_empty_is_password_required() {
        let f = fx();
        for pw in ["", "   ", "\t\n"] {
            let r = send(&f, record(pw, "https://example.com", None));
            assert_eq!(error_message(&r), "password required");
        }
        let r = send(&f, record("a\u{0000}b", "https://example.com", None));
        assert_eq!(error_message(&r), "invalid request");
        assert!(history(&f).is_empty());
    }

    #[test]
    fn history_newest_first_with_passwords_and_ms_timestamps() {
        let f = fx();
        for (pw, ms) in [
            ("p1", 1_699_000_000_000.0),
            ("p3", 1_699_000_300_000.0),
            ("p2", 1_699_000_100_000.0),
        ] {
            send(&f, record(pw, "https://example.com", Some(ms)));
        }
        let rows = history(&f);
        let passwords: Vec<&str> = rows.iter().map(|r| r.password.as_str()).collect();
        assert_eq!(passwords, ["p3", "p2", "p1"]);
        assert_eq!(rows[0].created_at, 1_699_000_300_000.0);
        assert!(rows
            .iter()
            .all(|r| r.id == r.id.to_uppercase() && r.id.len() == 36));
    }

    #[test]
    fn history_cap_30_through_bridge() {
        let f = fx();
        for n in 0..31 {
            f.sched.advance(Duration::from_secs(1));
            assert_eq!(
                send(&f, record(&format!("pw{n}"), "https://example.com", None)),
                Response::Acknowledged
            );
        }
        let rows = history(&f);
        assert_eq!(rows.len(), 30);
        assert_eq!(rows[0].password, "pw30");
        assert!(rows.iter().all(|r| r.password != "pw0"));
    }

    #[test]
    fn delete_generated_password_unknown_id_is_entry_not_found() {
        let (f, ids) = sample_fx();
        send(&f, record("pw", "https://example.com", None));
        for id in [
            "",
            "garbage",
            "00000000-0000-0000-0000-000000000000",
            &wire_id(ids.a),
        ] {
            let id = id.to_owned();
            let r = send(&f, move |token, pairing| Request::DeleteGeneratedPassword {
                token,
                id,
                pairing,
            });
            assert_eq!(error_message(&r), "entry not found");
        }
        // A login is never deleted through the history call.
        assert!(f
            .session
            .with_db(|db| db.find_entry(ids.a).is_some())
            .unwrap());
        assert_eq!(history(&f).iter().filter(|r| r.password == "pw").count(), 1);
    }

    #[test]
    fn delete_generated_password_removes_row() {
        let f = fx();
        send(
            &f,
            record("keep", "https://example.com", Some(1_699_000_000_000.0)),
        );
        send(
            &f,
            record("drop", "https://example.com", Some(1_699_000_100_000.0)),
        );
        let id = history(&f)[0].id.clone();
        let wanted = id.clone();
        let r = send(&f, move |token, pairing| Request::DeleteGeneratedPassword {
            token,
            id: wanted,
            pairing,
        });
        assert_eq!(r, Response::Acknowledged);
        let rows = history(&f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].password, "keep");
        // Gone for good: the same id is unknown now.
        let r = send(&f, move |token, pairing| Request::DeleteGeneratedPassword {
            token,
            id,
            pairing,
        });
        assert_eq!(error_message(&r), "entry not found");
    }

    #[test]
    fn generator_history_group_never_offered_by_credentials() {
        let f = fx();
        send(&f, record("pw", "https://example.com", None));
        assert_eq!(history(&f).len(), 1);
        let r = send(&f, |token, pairing| Request::GetCredentials {
            token,
            url: "https://example.com/login".into(),
            pairing,
        });
        assert!(crate::testing::credential_rows(&r).is_empty());
    }

    #[test]
    fn history_calls_answer_locked_when_locked() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        let ctx = f.svc.ctx_for_test();
        assert_eq!(get_generated_password_history(&ctx), Response::Locked);
        assert_eq!(
            delete_generated_password(&ctx, &wire_id(Uuid::nil())),
            Response::Locked
        );
    }

    #[test]
    fn no_stub_handlers_remain() {
        let needle = ["not", "implemented"].join(" ");
        for (name, src) in [
            ("browse.rs", include_str!("browse.rs")),
            ("create.rs", include_str!("create.rs")),
            ("fill.rs", include_str!("fill.rs")),
            ("generator.rs", include_str!("generator.rs")),
            ("identifier.rs", include_str!("identifier.rs")),
            ("mod.rs", include_str!("mod.rs")),
            ("session_ops.rs", include_str!("session_ops.rs")),
            ("text.rs", include_str!("text.rs")),
            ("service.rs", include_str!("../service.rs")),
        ] {
            assert!(!src.contains(&needle), "{name}");
        }
    }
}
