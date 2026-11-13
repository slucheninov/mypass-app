//! Session, proposal, pairing and diagnostics requests.
//!
//! A save proposal keeps its secret only in the [`ProposalStore`](crate::ui::ProposalStore): the
//! UI call carries the proposal id, the URL and the username, never the password or the TOTP
//! secret. The store clears itself when the vault locks.

use std::time::{Duration, UNIX_EPOCH};

use mypass_core::matcher::credential_candidates;
use mypass_session::session::LockReason;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::text::{
    is_clean_line, is_clean_text, MAX_OTP_CHARS, MAX_PASSWORD_CHARS, MAX_URL_CHARS,
    MAX_USERNAME_CHARS,
};
use crate::pairing::StartOutcome;
use crate::protocol::{BrowserDiagnosticsReport, Response};
use crate::service::Ctx;
use crate::ui::Proposal;
use crate::views::{error, parse_wire_id};

const NOT_FOUND: &str = "entry not found or domain mismatch";
const INVALID: &str = "invalid request";

/// Two `requestUnlock` calls closer than this show the unlock prompt once.
const UNLOCK_DEBOUNCE: Duration = Duration::from_secs(2);

/// The answer for a vault that locked between the gate and the handler: the proposal that was just
/// stored is dropped again and nothing is shown.
fn withdraw(ctx: &Ctx<'_>, id: &str) -> Response {
    let _ = ctx.deps.proposals.take(id);
    Response::Locked
}

pub(crate) fn propose_save_login(
    ctx: &Ctx<'_>,
    url: &str,
    username: &str,
    password: &str,
) -> Response {
    if !is_clean_line(url, MAX_URL_CHARS)
        || !is_clean_line(username, MAX_USERNAME_CHARS)
        || !is_clean_text(password, MAX_PASSWORD_CHARS, false)
    {
        return error(INVALID);
    }
    let id = ctx.deps.proposals.put(Proposal::Login {
        url: url.to_owned(),
        username: username.to_owned(),
        password: Zeroizing::new(password.to_owned()),
    });
    // A lock that raced the gate has already cleared the store; a proposal stored after it must not
    // outlive it.
    if !ctx.deps.session.is_unlocked() {
        return withdraw(ctx, &id);
    }
    ctx.deps.ui.save_login_proposal(&id, url, username);
    Response::SaveProposalAccepted
}

pub(crate) fn propose_save_totp(
    ctx: &Ctx<'_>,
    url: &str,
    otpauth_or_secret: &str,
    entry_id: Option<&str>,
) -> Response {
    if !is_clean_line(url, MAX_URL_CHARS) || !is_clean_line(otpauth_or_secret, MAX_OTP_CHARS) {
        return error(INVALID);
    }
    let resolved: Option<Uuid> = match entry_id {
        None => None,
        Some(wire) => {
            let wanted = parse_wire_id(wire);
            match ctx.deps.session.with_db(|db| {
                credential_candidates(db, url)
                    .into_iter()
                    .find(|e| Some(e.uuid) == wanted)
                    .map(|e| e.uuid)
            }) {
                Ok(Some(id)) => Some(id),
                Ok(None) => return error(NOT_FOUND),
                Err(_) => return Response::Locked,
            }
        }
    };
    let id = ctx.deps.proposals.put(Proposal::Totp {
        url: url.to_owned(),
        otp: Zeroizing::new(otpauth_or_secret.to_owned()),
        entry_id: resolved,
    });
    if !ctx.deps.session.is_unlocked() {
        return withdraw(ctx, &id);
    }
    ctx.deps.ui.save_totp_proposal(&id, url, resolved);
    Response::SaveProposalAccepted
}

pub(crate) fn start_pairing(ctx: &Ctx<'_>, browser: Option<&str>) -> Response {
    match ctx.deps.pairing.start(browser) {
        StartOutcome::Started(challenge) => {
            ctx.deps.ui.pairing_code(&challenge);
            Response::PairingStarted
        }
        StartOutcome::Throttled => error("pairing_throttled"),
    }
}

pub(crate) fn complete_pairing(ctx: &Ctx<'_>, code: &str) -> Response {
    match ctx.deps.pairing.complete(code) {
        Ok(Some(secret)) => {
            ctx.deps.ui.pairing_finished();
            Response::Paired { secret }
        }
        Ok(None) => error("invalid_code"),
        // The store failed: no detail (a path, a keychain status) goes to the extension.
        Err(_) => error("internal error"),
    }
}

pub(crate) fn request_unlock(ctx: &Ctx<'_>) -> Response {
    // Checked before the debounce, so a click just after a successful unlock always tells the popup
    // to re-query instead of being swallowed, and nothing is noted.
    if ctx.deps.session.is_unlocked() {
        return Response::UnlockNotNeeded;
    }
    let now = ctx.deps.scheduler.monotonic_now();
    let present = {
        let mut last = ctx
            .last_unlock_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let recent = last.is_some_and(|t| now.saturating_sub(t) < UNLOCK_DEBOUNCE);
        if !recent {
            *last = Some(now);
        }
        !recent
    };
    if present {
        ctx.deps.ui.unlock_requested();
    }
    Response::UnlockPresented
}

pub(crate) fn lock_vault(ctx: &Ctx<'_>) -> Response {
    ctx.deps.session.lock(LockReason::Manual);
    Response::Acknowledged
}

pub(crate) fn open_app(ctx: &Ctx<'_>) -> Response {
    ctx.deps.ui.open_app_requested();
    Response::Acknowledged
}

pub(crate) fn open_browser_settings(ctx: &Ctx<'_>) -> Response {
    ctx.deps.ui.open_browser_settings_requested();
    Response::Acknowledged
}

pub(crate) fn report_browser_diagnostics(
    ctx: &Ctx<'_>,
    report: &BrowserDiagnosticsReport,
    pairing: Option<&str>,
) -> Response {
    // The browser is the one the secret belongs to, never an id the client names.
    let Some(browser) = ctx.deps.pairing.authorized_browser(pairing) else {
        return Response::PairingRequired;
    };
    let received_at = ctx
        .now
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0);
    // `record` validates: an invalid report is ignored and the previous snapshot stays.
    if ctx
        .deps
        .diagnostics
        .record(report.clone(), browser.id, received_at)
    {
        Response::Acknowledged
    } else {
        error("invalid_browser_diagnostics")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::pairing::START_MIN_INTERVAL;
    use crate::protocol::Request;
    use crate::testing::{
        all_requests, error_message, fx, named, report, resets_timer, sample_fx, send, Fx,
    };
    use crate::views::wire_id;

    const GITHUB: &str = "https://github.com/login";

    /// No idle lock: the test moves the clock by minutes and means to stay unlocked.
    fn never_idle_lock(f: &Fx) {
        let deps = f.svc.ctx_for_test().deps;
        deps.settings
            .update(mypass_session::settings::SettingsPatch {
                auto_lock_idle_timeout: Some(mypass_session::settings::IdleTimeout::Never),
                ..Default::default()
            })
            .unwrap();
        deps.autolock.settings_changed();
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn ask(f: &Fx, name: &str) -> Response {
        f.svc.handle(named(name, Some(&f.secret)))
    }

    /// The first argument after the call name of the last UI call called `name`.
    fn last_arg(f: &Fx, name: &str) -> String {
        f.ui.details()
            .iter()
            .rev()
            .find_map(|d| d.strip_prefix(&format!("{name} ")))
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_owned()
    }

    fn propose_login(password: &str) -> impl FnOnce(String, Option<String>) -> Request {
        let password = password.to_owned();
        move |token, pairing| Request::ProposeSaveLogin {
            token,
            url: "https://a.example/login".into(),
            username: "alice".into(),
            password,
            pairing,
        }
    }

    fn propose_totp(entry_id: Option<String>) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::ProposeSaveTotp {
            token,
            url: GITHUB.into(),
            otpauth_or_secret: "JBSWY3DPEHPK3PXP".into(),
            entry_id,
            pairing,
        }
    }

    #[test]
    fn propose_save_login_stores_password_only_in_proposal_store() {
        let f = fx();
        let pw = "Sup3rSecretPw-xyz";
        let r = send(&f, propose_login(pw));
        assert_eq!(r, Response::SaveProposalAccepted);
        assert!(!serde_json::to_string(&r).unwrap().contains(pw));
        assert_eq!(f.ui.calls(), ["save_login_proposal"]);
        assert!(f.ui.details().iter().all(|d| !d.contains(pw)));
        let id = last_arg(&f, "save_login_proposal");
        let Some(Proposal::Login {
            url,
            username,
            password,
        }) = f.proposals.take(&id)
        else {
            panic!("no login proposal");
        };
        assert_eq!(
            (url.as_str(), username.as_str(), password.as_str()),
            ("https://a.example/login", "alice", pw)
        );
        assert!(f.proposals.take(&id).is_none());
    }

    #[test]
    fn propose_save_login_rejects_overlong_or_unsafe_strings() {
        let f = fx();
        for pw in ["a\u{0000}b".to_owned(), "x".repeat(MAX_PASSWORD_CHARS + 1)] {
            let r = send(&f, propose_login(&pw));
            assert_eq!(error_message(&r), "invalid request");
        }
        assert!(f.ui.calls().is_empty());
    }

    #[test]
    fn proposal_expires_after_120s_and_is_cleared_on_lock() {
        let f = fx();
        never_idle_lock(&f);
        send(&f, propose_login("pw1"));
        let id = last_arg(&f, "save_login_proposal");
        f.sched.advance(secs(119));
        assert!(f.proposals.take(&id).is_some());
        send(&f, propose_login("pw2"));
        let id = last_arg(&f, "save_login_proposal");
        f.sched.advance(secs(120));
        assert!(f.proposals.take(&id).is_none());
        send(&f, propose_login("pw3"));
        let id = last_arg(&f, "save_login_proposal");
        f.session.lock(LockReason::Manual);
        assert!(f.proposals.take(&id).is_none());
    }

    #[test]
    fn proposals_are_refused_while_locked_and_a_racing_lock_withdraws_them() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(send(&f, propose_login("pw")), Response::Locked);
        assert!(f.ui.calls().is_empty());
        // Called directly, past the gate: the handler notices the lock itself.
        let ctx = f.svc.ctx_for_test();
        assert_eq!(
            propose_save_login(&ctx, "https://a.example", "u", "pw"),
            Response::Locked
        );
        assert_eq!(
            propose_save_totp(&ctx, "https://a.example", "JBSWY3DPEHPK3PXP", None),
            Response::Locked
        );
        assert!(f.ui.calls().is_empty());
    }

    #[test]
    fn propose_save_totp_with_foreign_entry_id_is_domain_mismatch() {
        let (f, ids) = sample_fx();
        for id in [wire_id(ids.b), wire_id(ids.c), "garbage".to_owned()] {
            let r = send(&f, propose_totp(Some(id)));
            assert_eq!(error_message(&r), "entry not found or domain mismatch");
        }
        assert!(f.ui.calls().is_empty());
    }

    #[test]
    fn propose_save_totp_without_entry_id_is_accepted() {
        let (f, ids) = sample_fx();
        assert_eq!(send(&f, propose_totp(None)), Response::SaveProposalAccepted);
        let id = last_arg(&f, "save_totp_proposal");
        let Some(Proposal::Totp { entry_id, otp, .. }) = f.proposals.take(&id) else {
            panic!("no totp proposal");
        };
        assert_eq!((entry_id, otp.as_str()), (None, "JBSWY3DPEHPK3PXP"));
        // A matching entry id resolves and travels to the UI.
        let r = send(&f, propose_totp(Some(wire_id(ids.a))));
        assert_eq!(r, Response::SaveProposalAccepted);
        let last = f.ui.details().pop().unwrap();
        assert!(last.contains(&ids.a.to_string()), "{last}");
    }

    #[test]
    fn request_unlock_debounced_and_unlock_not_needed_when_unlocked() {
        let f = fx();
        // Unlocked: never needed, nothing noted, even inside a window.
        assert_eq!(ask(&f, "requestUnlock"), Response::UnlockNotNeeded);
        f.session.lock(LockReason::Manual);
        let count = |f: &Fx| {
            f.ui.calls()
                .iter()
                .filter(|c| *c == "unlock_requested")
                .count()
        };
        assert_eq!(ask(&f, "requestUnlock"), Response::UnlockPresented);
        assert_eq!(count(&f), 1);
        f.sched.advance(Duration::from_millis(1900));
        assert_eq!(ask(&f, "requestUnlock"), Response::UnlockPresented);
        assert_eq!(count(&f), 1);
        f.sched.advance(Duration::from_millis(100));
        assert_eq!(ask(&f, "requestUnlock"), Response::UnlockPresented);
        assert_eq!(count(&f), 2);
    }

    #[test]
    fn unlock_not_needed_inside_the_window_and_notes_nothing() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(ask(&f, "requestUnlock"), Response::UnlockPresented);
        f.session
            .unlock(secrecy::SecretString::from("pw".to_owned()))
            .unwrap();
        assert_eq!(ask(&f, "requestUnlock"), Response::UnlockNotNeeded);
        f.session.lock(LockReason::Manual);
        // Still inside the 2 s window of the first accepted request.
        assert_eq!(ask(&f, "requestUnlock"), Response::UnlockPresented);
        assert_eq!(
            f.ui.calls()
                .iter()
                .filter(|c| *c == "unlock_requested")
                .count(),
            1
        );
    }

    #[test]
    fn lock_vault_locks_and_never_resets_idle_timer() {
        // Paired: acknowledged and locked at once. The timer is not what locks it.
        let f = fx();
        assert!(!resets_timer(&f, || {
            f.sched.advance(secs(0));
            assert_eq!(ask(&f, "lockVault"), Response::Acknowledged);
            assert!(!f.session.is_unlocked());
        }));
        // Unpaired: refused, the vault stays unlocked, and the refused request does not count as
        // activity: the idle lock still fires 60 s after the last real one.
        let f = fx();
        assert!(!resets_timer(&f, || {
            assert_eq!(
                f.svc.handle(named("lockVault", None)),
                Response::PairingRequired
            );
            assert!(f.session.is_unlocked());
        }));
        assert!(!Request::LockVault {
            token: String::new(),
            pairing: None
        }
        .resets_auto_lock_timer());
    }

    #[test]
    fn open_app_answerable_while_locked_once_paired() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.svc.handle(named("openApp", None)),
            Response::PairingRequired
        );
        assert!(f.ui.calls().is_empty());
        assert_eq!(ask(&f, "openApp"), Response::Acknowledged);
        assert_eq!(f.ui.calls(), ["open_app_requested"]);
    }

    #[test]
    fn open_browser_settings_answerable_while_locked_once_paired() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.svc.handle(named("openBrowserSettings", None)),
            Response::PairingRequired
        );
        assert_eq!(ask(&f, "openBrowserSettings"), Response::Acknowledged);
        assert_eq!(f.ui.calls(), ["open_browser_settings_requested"]);
    }

    fn browser_id(f: &Fx) -> Uuid {
        f.svc.ctx_for_test().deps.pairing.list()[0].id
    }

    fn snapshot(f: &Fx, id: Uuid) -> Option<crate::diagnostics::DiagnosticsSnapshot> {
        f.svc.ctx_for_test().deps.diagnostics.get(id)
    }

    fn diag(f: &Fx, r: crate::protocol::BrowserDiagnosticsReport) -> Response {
        f.svc.handle(Request::ReportBrowserDiagnostics {
            token: "tok".into(),
            report: r,
            pairing: Some(f.secret.clone()),
        })
    }

    fn invalid_report() -> crate::protocol::BrowserDiagnosticsReport {
        let mut r = report();
        r.enabled_managers = vec!["Some Manager".into()];
        r
    }

    #[test]
    fn diagnostics_recorded_for_authenticated_browser_and_invalid_rejected() {
        let f = fx();
        let id = browser_id(&f);
        assert!(snapshot(&f, id).is_none());
        assert_eq!(diag(&f, report()), Response::Acknowledged);
        let snap = snapshot(&f, id).unwrap();
        assert_eq!(snap.report, report());
        assert_eq!(snap.received_at, 1_700_000_000);
        // Invalid: the exact error, and the earlier snapshot is kept untouched.
        f.sched.advance(secs(10));
        let r = diag(&f, invalid_report());
        assert_eq!(error_message(&r), "invalid_browser_diagnostics");
        assert_eq!(snapshot(&f, id).unwrap(), snap);
        // Only the secret's own browser has a snapshot.
        assert_eq!(f.svc.ctx_for_test().deps.diagnostics.all().len(), 1);
        // An unpaired caller is refused and records nothing.
        let r = f.svc.handle(Request::ReportBrowserDiagnostics {
            token: "tok".into(),
            report: report(),
            pairing: Some("nope".into()),
        });
        assert_eq!(r, Response::PairingRequired);
        assert_eq!(f.svc.ctx_for_test().deps.diagnostics.all().len(), 1);
    }

    #[test]
    fn diagnostics_while_locked_are_accepted_and_do_not_reset_timer() {
        let f = fx();
        assert!(!resets_timer(&f, || {
            assert_eq!(diag(&f, report()), Response::Acknowledged);
        }));
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(diag(&f, report()), Response::Acknowledged);
        assert!(snapshot(&f, browser_id(&f)).is_some());
    }

    #[test]
    fn start_pairing_shows_code_via_ui_and_answers_pairing_started() {
        let f = fx();
        f.sched.advance(START_MIN_INTERVAL);
        let r = f.svc.handle(named("startPairing", None));
        assert_eq!(r, Response::PairingStarted);
        let code = last_arg(&f, "pairing_code");
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert!(!serde_json::to_string(&r).unwrap().contains(&code));
    }

    #[test]
    fn start_pairing_while_locked_is_locked() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.sched.advance(START_MIN_INTERVAL);
        assert_eq!(f.svc.handle(named("startPairing", None)), Response::Locked);
        assert!(f.ui.calls().is_empty());
    }

    #[test]
    fn start_pairing_throttled_message() {
        let f = fx();
        f.sched.advance(START_MIN_INTERVAL);
        assert_eq!(
            f.svc.handle(named("startPairing", None)),
            Response::PairingStarted
        );
        let r = f.svc.handle(named("startPairing", None));
        assert_eq!(error_message(&r), "pairing_throttled");
        assert_eq!(f.ui.calls(), ["pairing_code"]);
    }

    fn complete(f: &Fx, code: &str) -> Response {
        f.svc.handle(Request::CompletePairing {
            token: "tok".into(),
            code: code.into(),
        })
    }

    #[test]
    fn complete_pairing_returns_secret_once_and_wrong_code_is_invalid_code() {
        let f = fx();
        f.sched.advance(START_MIN_INTERVAL);
        f.svc.handle(named("startPairing", None));
        let code = last_arg(&f, "pairing_code");
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert_eq!(error_message(&complete(&f, wrong)), "invalid_code");
        assert!(!f.ui.calls().contains(&"pairing_finished".to_owned()));
        let Response::Paired { secret } = complete(&f, &code) else {
            panic!("not paired");
        };
        assert!(!secret.is_empty());
        assert_eq!(
            f.ui.calls()
                .iter()
                .filter(|c| *c == "pairing_finished")
                .count(),
            1
        );
        // The code is spent.
        assert_eq!(error_message(&complete(&f, &code)), "invalid_code");
    }

    #[test]
    fn complete_pairing_store_failure_is_an_internal_error_without_detail() {
        let f = fx();
        // The fixture holds one browser; fill the store to its limit so `complete` fails.
        never_idle_lock(&f);
        let mut failure = None;
        for _ in 0..80 {
            f.sched.advance(START_MIN_INTERVAL);
            f.svc.handle(named("startPairing", None));
            let code = last_arg(&f, "pairing_code");
            match complete(&f, &code) {
                Response::Paired { .. } => {}
                other => {
                    failure = Some(other);
                    break;
                }
            }
        }
        let r = failure.expect("the paired list never filled up");
        assert_eq!(error_message(&r), "internal error");
    }

    #[test]
    fn pairing_flow_end_to_end_then_get_credentials_authorised() {
        let (f, _) = sample_fx();
        f.sched.advance(START_MIN_INTERVAL);
        f.svc.handle(named("startPairing", None));
        let code = last_arg(&f, "pairing_code");
        let Response::Paired { secret } = complete(&f, &code) else {
            panic!("not paired");
        };
        let get = |pairing: Option<String>| {
            f.svc.handle(Request::GetCredentials {
                token: "tok".into(),
                url: GITHUB.into(),
                pairing,
            })
        };
        assert_eq!(get(None), Response::PairingRequired);
        assert!(matches!(get(Some(secret)), Response::CredentialList { .. }));
    }

    #[test]
    fn revoked_browser_gets_pairing_required_and_loses_diagnostics() {
        let (f, _) = sample_fx();
        let id = browser_id(&f);
        assert_eq!(diag(&f, report()), Response::Acknowledged);
        let deps = f.svc.ctx_for_test().deps;
        deps.pairing.revoke(id).unwrap();
        // The app wiring drops the snapshot of a revoked browser the same way.
        deps.diagnostics.remove(id);
        for r in all_requests(Some(&f.secret)) {
            if r.requires_pairing() {
                assert_eq!(f.svc.handle(r), Response::PairingRequired);
            }
        }
        assert!(snapshot(&f, id).is_none());
        assert!(deps.diagnostics.all().is_empty());
    }
}
