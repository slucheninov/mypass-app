//! Session, proposal, pairing and diagnostics requests.
//!
//! A save proposal keeps its secret only in the [`ProposalStore`](crate::ui::ProposalStore): the
//! UI call carries the proposal id, the URL and the username, never the password or the TOTP
//! secret. The store clears itself when the vault locks.

use std::time::{Duration, UNIX_EPOCH};

use mypass_core::entry_ext::{EntryExt, EntryType};
use mypass_core::matcher::credential_candidates;
use mypass_session::queries::address_of;
use mypass_session::session::LockReason;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::browse::live_entries;
use super::text::{
    is_clean_line, is_clean_text, MAX_OTP_CHARS, MAX_PASSWORD_CHARS, MAX_URL_CHARS,
    MAX_USERNAME_CHARS,
};
use crate::pairing::StartOutcome;
use crate::protocol::{
    BrowserDiagnosticsReport, ExtraFieldKind, NewEntryKind, ProposalExtraField, Response,
    WirePassword,
};
use crate::service::Ctx;
use crate::ui::{Proposal, ProposalExtra};
use crate::unlock::{PasswordAttempt, UnlockFailure};
use crate::views::{error, pairing_error_response, parse_wire_id};

pub(crate) fn unlock_with_password(ctx: &Ctx<'_>, password: WirePassword) -> Response {
    if ctx.deps.session.is_unlocked() {
        return Response::UnlockNotNeeded;
    }
    if password.char_len() > MAX_PASSWORD_CHARS {
        return error("invalid request");
    }
    match ctx.throttle.attempt(
        || ctx.deps.session.is_unlocked(),
        || {
            ctx.deps
                .unlocker
                .unlock_with_password(password.into_secret())
        },
    ) {
        PasswordAttempt::Unlocked => Response::Unlocked,
        PasswordAttempt::NotNeeded => Response::UnlockNotNeeded,
        PasswordAttempt::WrongPassword => Response::WrongPassword,
        PasswordAttempt::Throttled { retry_after_secs } => {
            Response::UnlockThrottled { retry_after_secs }
        }
        PasswordAttempt::InApp => Response::UnlockInApp,
        PasswordAttempt::Failed => error("unlock_failed"),
    }
}

/// One device prompt for the popup. A request while another bridge prompt is in flight, or within
/// two seconds of the last one starting, is answered `unlockCancelled` without a prompt (a popup
/// reopened during a prompt never starts a second one and never counts a failure). A prompt that
/// did not recognize the user answers `deviceNotRecognized` with the lock spell's count.
pub(crate) fn unlock_with_device(ctx: &Ctx<'_>, automatic: bool) -> Response {
    if ctx.deps.session.is_unlocked() {
        return Response::UnlockNotNeeded;
    }
    ctx.throttle
        .device_attempt(|| {
            if ctx.deps.session.is_unlocked() {
                return Response::UnlockNotNeeded;
            }
            match ctx.deps.unlocker.unlock_with_device(automatic) {
                Ok(()) => {
                    ctx.throttle.reset();
                    Response::Unlocked
                }
                Err(UnlockFailure::Cancelled) => Response::UnlockCancelled,
                Err(UnlockFailure::DeviceUnavailable) => Response::DeviceUnlockUnavailable,
                Err(UnlockFailure::InApp) => Response::UnlockInApp,
                Err(UnlockFailure::NotRecognized) => Response::DeviceNotRecognized {
                    device_failures: ctx.deps.session.device_unlock_attempts().failures,
                },
                Err(UnlockFailure::WrongPassword | UnlockFailure::Failed) => error("unlock_failed"),
            }
        })
        .unwrap_or(Response::UnlockCancelled)
}

pub(crate) fn get_unlock_options(ctx: &Ctx<'_>) -> Response {
    let offer = ctx.deps.unlocker.device_offer();
    let attempts = ctx.deps.session.device_unlock_attempts();
    Response::UnlockOptions {
        device_unlock: offer.offered && !attempts.blocked && !ctx.deps.session.is_unlocked(),
        touch_id: offer.touch_id,
        device_failures: attempts.failures,
        device_blocked: attempts.blocked,
        device_prompt_in_flight: offer.prompt_in_flight,
        retry_after_secs: ctx.throttle.retry_after_secs(),
    }
}

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

/// Most extra values one registration form may send.
pub(crate) const MAX_EXTRA_FIELDS: usize = 24;
/// Longest readable label of an extra value, in characters.
pub(crate) const MAX_EXTRA_LABEL_CHARS: usize = 64;
/// Longest extra value, in characters (a street address joined into one line fits).
pub(crate) const MAX_EXTRA_VALUE_CHARS: usize = 512;

/// Label prefixes of fields the app or KeePass clients own. An `Other` value is stored under its
/// label, so a page must never be able to aim it at one of those.
const RESERVED_LABEL_PREFIXES: [&str; 3] = ["mypass.", "kpex_", "kp2a_"];

/// Checks every extra value against the limits. One bad value refuses the whole proposal, so a page
/// never gets a partial save it did not ask for.
fn checked_extras(fields: &[ProposalExtraField]) -> Option<Vec<ProposalExtra>> {
    if fields.len() > MAX_EXTRA_FIELDS {
        return None;
    }
    fields
        .iter()
        .map(|field| {
            // The raw text is checked before it is trimmed, exactly like the extension's background
            // (prompt-record.js `checkFields`): a control character or an over-long value that
            // trimming would hide still refuses the proposal.
            if !is_clean_line(&field.label, MAX_EXTRA_LABEL_CHARS)
                || !is_clean_line(&field.value, MAX_EXTRA_VALUE_CHARS)
            {
                return None;
            }
            let label = field.label.trim();
            let value = field.value.trim();
            let lowered = label.to_lowercase();
            let label_ok = (field.kind != ExtraFieldKind::Other || !label.is_empty())
                && !RESERVED_LABEL_PREFIXES
                    .iter()
                    .any(|p| lowered.starts_with(p));
            (label_ok && !value.is_empty()).then(|| ProposalExtra {
                kind: field.kind,
                label: label.to_owned(),
                value: Zeroizing::new(value.to_owned()),
            })
        })
        .collect()
}

/// Case and whitespace folded, so "12  Main St" and "12 main st" are the same address line.
fn folded(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Whether `address` holds every captured address part (a country may match the code or the name).
fn holds_address(address: &mypass_session::dto::AddressDto, extras: &[ProposalExtra]) -> bool {
    extras.iter().filter(|x| x.kind.is_address_part()).all(|x| {
        let wanted = folded(&x.value);
        let stored: &[&str] = match x.kind {
            ExtraFieldKind::AddressLine1 => &[&address.address_line1],
            ExtraFieldKind::AddressLine2 => &[&address.address_line2],
            ExtraFieldKind::City => &[&address.city],
            ExtraFieldKind::Region => &[&address.region],
            ExtraFieldKind::PostalCode => &[&address.postal_code],
            ExtraFieldKind::Country => &[&address.country_code, &address.country_name],
            ExtraFieldKind::Name
            | ExtraFieldKind::GivenName
            | ExtraFieldKind::FamilyName
            | ExtraFieldKind::Email
            | ExtraFieldKind::Phone
            | ExtraFieldKind::Birthday
            | ExtraFieldKind::Username
            | ExtraFieldKind::Organization
            | ExtraFieldKind::Other => &[],
        };
        stored.iter().any(|s| folded(s) == wanted)
    })
}

pub(crate) fn propose_save_login(
    ctx: &Ctx<'_>,
    url: &str,
    username: &str,
    password: &str,
    extra_fields: &[ProposalExtraField],
    username_is_email: bool,
) -> Response {
    if !is_clean_line(url, MAX_URL_CHARS)
        || !is_clean_line(username, MAX_USERNAME_CHARS)
        || !is_clean_text(password, MAX_PASSWORD_CHARS, false)
    {
        return error(INVALID);
    }
    let Some(extras) = checked_extras(extra_fields) else {
        return error(INVALID);
    };
    // An Address entry is offered only for a form with address parts that no live Address entry
    // already holds.
    let offer_address = if extras.iter().any(|x| x.kind.is_address_part()) {
        match ctx.deps.session.with_db(|db| {
            !live_entries(db).into_iter().any(|(e, _)| {
                e.entry_type() == EntryType::Address && holds_address(&address_of(e), &extras)
            })
        }) {
            Ok(offer) => offer,
            Err(_) => return Response::Locked,
        }
    } else {
        false
    };
    let id = ctx.deps.proposals.put(Proposal::Login {
        url: url.to_owned(),
        username: username.to_owned(),
        password: Zeroizing::new(password.to_owned()),
        extras,
        offer_address,
        // Only meaningful with a username to copy.
        username_is_email: username_is_email && !username.trim().is_empty(),
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
        // The store failed: a fixed message, no detail (a path, a keychain status).
        Err(e) => pairing_error_response(&e),
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

pub(crate) fn open_app(ctx: &Ctx<'_>, new_entry: Option<NewEntryKind>) -> Response {
    match new_entry {
        None => ctx.deps.ui.open_app_requested(),
        Some(kind) => ctx.deps.ui.new_entry_requested(kind),
    }
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
    use crate::protocol::{ExtraFieldKind, ProposalExtraField};
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
            extra_fields: Vec::new(),
            username_is_email: false,
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
            extras,
            offer_address,
            username_is_email,
        }) = f.proposals.take(&id)
        else {
            panic!("no login proposal");
        };
        assert_eq!(
            (url.as_str(), username.as_str(), password.as_str()),
            ("https://a.example/login", "alice", pw)
        );
        assert!(extras.is_empty());
        assert!(!offer_address);
        assert!(!username_is_email);
        assert!(f.proposals.take(&id).is_none());
    }

    #[test]
    fn a_3000_char_url_is_accepted_for_login_proposals() {
        let f = fx();
        let url = format!("https://a.example/?q={}", "x".repeat(2980));
        let r = send(&f, |token, pairing| Request::ProposeSaveLogin {
            token,
            url,
            username: "u".into(),
            password: "pw".into(),
            extra_fields: Vec::new(),
            username_is_email: false,
            pairing,
        });
        assert_eq!(r, Response::SaveProposalAccepted);
        let r = send(&f, |token, pairing| Request::ProposeSaveLogin {
            token,
            url: "x".repeat(MAX_URL_CHARS + 1),
            username: "u".into(),
            password: "pw".into(),
            extra_fields: Vec::new(),
            username_is_email: false,
            pairing,
        });
        assert_eq!(error_message(&r), "invalid request");
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

    fn extra(kind: ExtraFieldKind, label: &str, value: &str) -> ProposalExtraField {
        ProposalExtraField {
            kind,
            label: label.into(),
            value: value.into(),
        }
    }

    fn propose_with(
        fields: Vec<ProposalExtraField>,
    ) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::ProposeSaveLogin {
            token,
            url: "https://shop.example/register".into(),
            username: "ann@example.com".into(),
            password: "N3w-Passw0rd".into(),
            extra_fields: fields,
            username_is_email: false,
            pairing,
        }
    }

    #[test]
    fn propose_save_login_keeps_that_the_username_is_the_email() {
        let f = fx();
        let flagged = |username: &'static str| {
            move |token, pairing| Request::ProposeSaveLogin {
                token,
                url: "https://shop.example/register".into(),
                username: username.into(),
                password: "N3w-Passw0rd".into(),
                extra_fields: vec![extra(ExtraFieldKind::City, "City", "Lviv")],
                username_is_email: true,
                pairing,
            }
        };
        let flag = |f: &Fx| {
            let id = last_arg(f, "save_login_proposal");
            match f.proposals.take(&id) {
                Some(Proposal::Login {
                    username_is_email, ..
                }) => username_is_email,
                _ => panic!("no login proposal"),
            }
        };
        assert_eq!(
            send(&f, flagged("ann@example.com")),
            Response::SaveProposalAccepted
        );
        assert!(flag(&f));
        assert_eq!(send(&f, flagged("")), Response::SaveProposalAccepted);
        assert!(!flag(&f), "no username, nothing to copy");
    }

    /// The stored extras (kind, label, value) and whether an Address entry is offered.
    fn stored_extras(f: &Fx) -> (Vec<(ExtraFieldKind, String, String)>, bool) {
        let id = last_arg(f, "save_login_proposal");
        let Some(Proposal::Login {
            extras,
            offer_address,
            ..
        }) = f.proposals.take(&id)
        else {
            panic!("no login proposal");
        };
        let rows = extras
            .iter()
            .map(|x| (x.kind, x.label.clone(), String::clone(&x.value)))
            .collect();
        (rows, offer_address)
    }

    #[test]
    fn propose_save_login_keeps_the_checked_extra_fields_trimmed() {
        let (f, _) = sample_fx();
        let r = send(
            &f,
            propose_with(vec![
                extra(ExtraFieldKind::GivenName, " First name ", " Ann "),
                extra(ExtraFieldKind::Phone, "Phone", "+380501234567"),
                extra(ExtraFieldKind::Other, "Nickname", "annie"),
            ]),
        );
        assert_eq!(r, Response::SaveProposalAccepted);
        // The UI call still carries only the id, the URL and the username.
        assert!(f.ui.details().iter().all(|d| !d.contains("annie")));
        let (rows, offer_address) = stored_extras(&f);
        assert_eq!(
            rows,
            [
                (ExtraFieldKind::GivenName, "First name".into(), "Ann".into()),
                (
                    ExtraFieldKind::Phone,
                    "Phone".into(),
                    "+380501234567".into()
                ),
                (ExtraFieldKind::Other, "Nickname".into(), "annie".into()),
            ]
        );
        assert!(!offer_address, "no address part, no Address entry offered");
    }

    #[test]
    fn an_address_is_offered_only_when_no_address_entry_holds_it() {
        let (f, _) = sample_fx();
        // The sample vault's "Home Address" is in Kyiv: case and spacing do not matter.
        send(
            &f,
            propose_with(vec![extra(ExtraFieldKind::City, "City", "  KYIV ")]),
        );
        assert_eq!(
            stored_extras(&f),
            (
                vec![(ExtraFieldKind::City, "City".into(), "KYIV".into())],
                false
            )
        );
        send(
            &f,
            propose_with(vec![extra(ExtraFieldKind::City, "City", "Lviv")]),
        );
        assert!(stored_extras(&f).1, "a new city is a new address");
        // Every captured part must be held: Kyiv plus a postal code the entry lacks is new.
        send(
            &f,
            propose_with(vec![
                extra(ExtraFieldKind::City, "City", "Kyiv"),
                extra(ExtraFieldKind::PostalCode, "ZIP", "01001"),
            ]),
        );
        assert!(stored_extras(&f).1);
    }

    #[test]
    fn extra_fields_at_the_limits_are_accepted() {
        let f = fx();
        let fields = (0..MAX_EXTRA_FIELDS)
            .map(|i| {
                extra(
                    ExtraFieldKind::Other,
                    &format!("{i:0>width$}", width = MAX_EXTRA_LABEL_CHARS),
                    &"v".repeat(MAX_EXTRA_VALUE_CHARS),
                )
            })
            .collect();
        assert_eq!(
            send(&f, propose_with(fields)),
            Response::SaveProposalAccepted
        );
        assert_eq!(stored_extras(&f).0.len(), MAX_EXTRA_FIELDS);
    }

    #[test]
    fn one_bad_extra_field_refuses_the_whole_proposal() {
        let f = fx();
        let too_many = (0..=MAX_EXTRA_FIELDS)
            .map(|_| extra(ExtraFieldKind::Other, "Field", "v"))
            .collect::<Vec<_>>();
        let long_label = "l".repeat(MAX_EXTRA_LABEL_CHARS + 1);
        let long_value = "v".repeat(MAX_EXTRA_VALUE_CHARS + 1);
        let cases = vec![
            too_many,
            vec![extra(ExtraFieldKind::City, &long_label, "Kyiv")],
            vec![extra(ExtraFieldKind::City, "City", &long_value)],
            vec![extra(ExtraFieldKind::City, "City", "   ")],
            vec![extra(ExtraFieldKind::Other, "  ", "value")],
            vec![extra(ExtraFieldKind::Other, "MyPass.EntryType", "Card")],
            vec![extra(
                ExtraFieldKind::Other,
                "KPEX_PASSKEY_PRIVATE_KEY_PEM",
                "x",
            )],
            vec![extra(
                ExtraFieldKind::Other,
                "kp2a_url",
                "https://evil.example",
            )],
            vec![extra(ExtraFieldKind::Phone, "Phone", "1\u{0007}2")],
            vec![extra(ExtraFieldKind::Other, "Line\nbreak", "v")],
            vec![
                extra(ExtraFieldKind::GivenName, "First name", "Ann"),
                extra(ExtraFieldKind::Phone, "Phone", "1\u{0000}"),
            ],
        ];
        for fields in cases {
            let r = send(&f, propose_with(fields));
            assert_eq!(error_message(&r), "invalid request");
        }
        assert!(f.ui.calls().is_empty(), "nothing was proposed");
    }

    /// The raw label and value are checked before they are trimmed, like the extension's background
    /// does: trimming must not turn a refused field into an accepted one.
    #[test]
    fn extra_fields_are_checked_before_they_are_trimmed() {
        let f = fx();
        let padded_value = format!(" {} ", "v".repeat(MAX_EXTRA_VALUE_CHARS));
        let padded_label = format!(" {} ", "l".repeat(MAX_EXTRA_LABEL_CHARS));
        let cases = vec![
            vec![extra(ExtraFieldKind::GivenName, "First name", "\tAnn")],
            vec![extra(ExtraFieldKind::GivenName, "First name", "Ann\n")],
            vec![extra(
                ExtraFieldKind::GivenName,
                "First name\u{0085}",
                "Ann",
            )],
            vec![extra(ExtraFieldKind::City, "City", &padded_value)],
            vec![extra(ExtraFieldKind::City, &padded_label, "Kyiv")],
        ];
        for fields in cases {
            let r = send(&f, propose_with(fields));
            assert_eq!(error_message(&r), "invalid request");
        }
        assert!(f.ui.calls().is_empty(), "nothing was proposed");
    }

    #[test]
    fn a_proposal_with_extra_fields_is_gated_like_any_other() {
        let f = fx();
        let fields = || vec![extra(ExtraFieldKind::GivenName, "First name", "Ann")];
        // No pairing secret: refused before the handler.
        let r = f.svc.handle(Request::ProposeSaveLogin {
            token: "tok".into(),
            url: "https://shop.example/register".into(),
            username: "u".into(),
            password: "pw".into(),
            extra_fields: fields(),
            username_is_email: false,
            pairing: None,
        });
        assert_eq!(r, Response::PairingRequired);
        f.session.lock(LockReason::Manual);
        assert_eq!(send(&f, propose_with(fields())), Response::Locked);
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
            propose_save_login(&ctx, "https://a.example", "u", "pw", &[], false),
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
    fn open_app_with_new_entry_asks_the_ui_for_that_entry_kind_even_while_locked() {
        let open = |kind, pairing: Option<String>| Request::OpenApp {
            token: String::new(),
            new_entry: Some(kind),
            pairing,
        };
        let f = fx();
        f.session.lock(LockReason::Manual);
        // Unpaired: refused before the UI hears of it.
        assert_eq!(
            f.svc.handle(open(NewEntryKind::Address, None)),
            Response::PairingRequired
        );
        assert!(f.ui.calls().is_empty());
        // Paired and locked: answered, the window comes forward with the request.
        assert_eq!(
            f.svc
                .handle(open(NewEntryKind::Address, Some(f.secret.clone()))),
            Response::Acknowledged
        );
        assert_eq!(
            f.svc
                .handle(open(NewEntryKind::Files, Some(f.secret.clone()))),
            Response::Acknowledged
        );
        assert_eq!(
            f.ui.details(),
            ["new_entry_requested Address", "new_entry_requested Files"]
        );
        // Without `newEntry` only the window is shown.
        assert_eq!(ask(&f, "openApp"), Response::Acknowledged);
        assert_eq!(f.ui.calls().last().unwrap(), "open_app_requested");
        // Unlocked: the same.
        let f = fx();
        assert_eq!(
            f.svc
                .handle(open(NewEntryKind::Files, Some(f.secret.clone()))),
            Response::Acknowledged
        );
        assert_eq!(f.ui.details(), ["new_entry_requested Files"]);
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
    fn complete_pairing_store_limit_tells_the_user_to_revoke_a_browser() {
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
        assert_eq!(
            error_message(&r),
            "too many paired browsers, revoke one in MyPass settings"
        );
    }

    #[test]
    fn wrong_codes_lead_to_pairing_throttled_from_start_pairing() {
        let f = fx();
        never_idle_lock(&f);
        let mut throttled = false;
        // Every start is spaced out, so only the failure budget can refuse it.
        for _ in 0..40 {
            f.sched.advance(START_MIN_INTERVAL);
            let r = f.svc.handle(named("startPairing", None));
            if r != Response::PairingStarted {
                assert_eq!(error_message(&r), "pairing_throttled");
                throttled = true;
                break;
            }
            let code = last_arg(&f, "pairing_code");
            let wrong = if code == "000000" { "000001" } else { "000000" };
            for _ in 0..3 {
                assert_eq!(error_message(&complete(&f, wrong)), "invalid_code");
            }
        }
        assert!(throttled, "the failure budget never throttled a start");
        // The throttle is not a way in: the active code was dropped with the budget.
        assert_eq!(error_message(&complete(&f, "123456")), "invalid_code");
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
