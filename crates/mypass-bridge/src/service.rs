//! The bridge service: every request from the extension passes the gates here, then reaches its
//! handler.
//!
//! Order:
//! 1. quitting: every request is refused, so nothing mutates the vault during the quit flow;
//! 2. pairing: everything but the two handshake requests needs a paired browser;
//! 3. locked: only the requests allowed while locked are answered, the rest get `Locked`;
//! 4. dispatch, an exhaustive `match` over the request enum (a new request must pick a handler);
//! 5. a served request that is a deliberate user gesture resets the idle auto-lock timer.
//!
//! Unpaired requests to vault operations reveal no lock state. The data handlers re-check the
//! lock under the session's own lock (`with_db`).
//!
//! The bridge always searches with `SearchScope::Autofill`: free text the
//! extension has no use for is never matched.

#![deny(clippy::wildcard_enum_match_arm)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use mypass_session::autolock::AutoLock;
use mypass_session::scheduler::Scheduler;
use mypass_session::session::{SessionEvent, SessionState, VaultSession};
use mypass_session::settings::SettingsStore;

use crate::diagnostics::DiagnosticsStore;
use crate::handlers::{browse, create, fill, generator, identifier, session_ops, social};
use crate::pairing::PairingManager;
use crate::passkeys::{PasskeyContext, PasskeyHandler};
use crate::protocol::{Request, Response};
use crate::ui::{BridgeUi, ProposalStore};
use crate::unlock::{UnlockThrottle, VaultUnlocker};
use crate::views::error;

/// Everything the service works with.
pub struct BridgeDeps {
    pub session: VaultSession,
    pub autolock: AutoLock,
    pub settings: SettingsStore,
    pub scheduler: Arc<dyn Scheduler>,
    pub pairing: Arc<PairingManager>,
    pub diagnostics: Arc<DiagnosticsStore>,
    pub proposals: Arc<ProposalStore>,
    pub ui: Arc<dyn BridgeUi>,
    pub passkeys: Arc<dyn PasskeyHandler>,
    pub unlocker: Arc<dyn VaultUnlocker>,
}

/// Built once per request and handed to the handler.
pub(crate) struct Ctx<'a> {
    pub deps: &'a BridgeDeps,
    pub throttle: &'a UnlockThrottle,
    /// The injected wall clock at the start of the request.
    pub now: SystemTime,
    /// Monotonic time of the last `requestUnlock` that was shown to the user (the debounce).
    pub last_unlock_request: &'a Mutex<Option<Duration>>,
}

pub struct BridgeService {
    deps: BridgeDeps,
    throttle: Arc<UnlockThrottle>,
    last_unlock_request: Mutex<Option<Duration>>,
}

impl std::fmt::Debug for BridgeService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeService").finish_non_exhaustive()
    }
}

impl BridgeService {
    /// Builds the service and registers the session observer that drops pending proposals and
    /// tells the passkey handler when the vault stops being unlocked or is switched.
    pub fn new(deps: BridgeDeps) -> Arc<Self> {
        let service = Self::new_unobserved(deps);
        service.register_observer();
        service
    }

    /// Builds the service without the session observer: a caller that can still fail to start
    /// registers it with [`Self::register_observer`] only once it is sure to run, so a failed
    /// start leaves no observer behind.
    pub fn new_unobserved(deps: BridgeDeps) -> Arc<Self> {
        Arc::new(Self {
            throttle: Arc::new(UnlockThrottle::new(Arc::clone(&deps.scheduler))),
            deps,
            last_unlock_request: Mutex::new(None),
        })
    }

    /// Registers the session observer (call once).
    pub fn register_observer(&self) {
        let throttle = Arc::clone(&self.throttle);
        let proposals = self.deps.proposals.clone();
        let passkeys = self.deps.passkeys.clone();
        // A direct switch from one unlocked vault to another emits only `StateChanged(Unlocked)`
        // with a new path: a proposal made for the old vault must not be taken into the new one,
        // and a pending passkey ceremony (its candidates are entry ids of the old vault) must not
        // be confirmed or approved against the new one.
        // Seeded with the vault already open at registration (the bridge can start, or restart,
        // while a vault is unlocked), so the first switch after that is seen as a switch too.
        // The seed is taken after the observer is in place and only when no event arrived in
        // between, under the observer's own lock, so no switch can fall between the two.
        // `.1` records that the observer has seen an event.
        let open_path: Arc<Mutex<(Option<String>, bool)>> = Arc::new(Mutex::new((None, false)));
        let observed = Arc::clone(&open_path);
        self.deps
            .session
            .add_observer(Arc::new(move |e: &SessionEvent| {
                if let SessionEvent::StateChanged(status) = e {
                    let mut guard = observed.lock().unwrap_or_else(PoisonError::into_inner);
                    let (open, seen) = &mut *guard;
                    *seen = true;
                    if status.state != SessionState::Unlocked {
                        *open = None;
                        proposals.clear();
                        passkeys.on_session_locked();
                    } else {
                        throttle.reset();
                        if open.is_some() && *open != status.vault_path {
                            proposals.clear();
                            passkeys.on_session_locked();
                        }
                        open.clone_from(&status.vault_path);
                    }
                }
            }));
        let mut guard = open_path.lock().unwrap_or_else(PoisonError::into_inner);
        if !guard.1 {
            let status = self.deps.session.status();
            if status.state == SessionState::Unlocked {
                guard.0 = status.vault_path;
            }
        }
    }

    /// A context for calling a handler directly, past the gates.
    #[cfg(test)]
    pub(crate) fn ctx_for_test(&self) -> Ctx<'_> {
        Ctx {
            deps: &self.deps,
            now: self.deps.scheduler.wall_now(),
            last_unlock_request: &self.last_unlock_request,
            throttle: &self.throttle,
        }
    }

    /// Answers one request. Never panics, never returns vault data to a locked, unpaired or
    /// quitting caller.
    pub fn handle(&self, mut request: Request) -> Response {
        let deps = &self.deps;
        if deps.session.is_quitting() {
            return error("MyPass is quitting");
        }
        if request.requires_pairing() && !deps.pairing.is_authorized(request.pairing()) {
            return Response::PairingRequired;
        }
        let unlocked = deps.session.is_unlocked();
        if !unlocked && !request.is_answerable_while_locked() {
            return Response::Locked;
        }
        let ctx = Ctx {
            deps,
            now: deps.scheduler.wall_now(),
            last_unlock_request: &self.last_unlock_request,
            throttle: &self.throttle,
        };
        let response = Self::dispatch(&ctx, &mut request, unlocked);
        self.note_activity(&request, &response);
        response
    }

    /// Resets the idle timer for a served, gesture-gated request (see `resets_auto_lock_timer`).
    pub(crate) fn note_activity(&self, request: &Request, response: &Response) {
        if request.resets_auto_lock_timer() && response.was_served() {
            self.deps.autolock.record_activity();
        }
    }

    fn dispatch(ctx: &Ctx<'_>, request: &mut Request, unlocked: bool) -> Response {
        match request {
            Request::GetCredentials { url, .. } => browse::get_credentials(ctx, url),
            Request::GetFavorites { .. } => browse::get_favorites(ctx),
            Request::ListEntries { category, .. } => browse::list_entries(ctx, category),
            Request::SearchEntries { query, .. } => browse::search_entries(ctx, query),
            Request::CreateEntry {
                entry_type,
                title,
                username,
                password,
                url,
                card_number,
                cardholder_name,
                expiry,
                cvv,
                notes,
                favorite,
                r#match,
                otp,
                ..
            } => create::create_entry(
                ctx,
                &create::CreateEntryArgs {
                    entry_type,
                    title,
                    username,
                    password,
                    url,
                    card_number,
                    cardholder_name,
                    expiry,
                    cvv,
                    notes,
                    favorite: *favorite,
                    match_mode: r#match,
                    otp,
                },
            ),
            Request::FillAddress { id, url, .. } => fill::fill_address(ctx, id, url),
            Request::FillPerson {
                id,
                url,
                address_id,
                ..
            } => fill::fill_person(ctx, id, url, address_id.as_deref()),
            Request::FillCredential { id, url, .. } => fill::fill_credential(ctx, id, url),
            Request::GetTotp { id, url, .. } => fill::get_totp(ctx, id, url),
            Request::ProposeSaveLogin {
                url,
                username,
                password,
                extra_fields,
                username_is_email,
                ..
            } => session_ops::propose_save_login(
                ctx,
                url,
                username,
                password,
                extra_fields,
                *username_is_email,
            ),
            Request::AddEntryIdentifier {
                id,
                url,
                value,
                field_key,
                ..
            } => identifier::add_entry_identifier(ctx, id, url, value, field_key),
            Request::ProposeSaveTotp {
                url,
                otpauth_or_secret,
                entry_id,
                ..
            } => session_ops::propose_save_totp(ctx, url, otpauth_or_secret, entry_id.as_deref()),
            Request::StartPairing { browser, .. } => {
                session_ops::start_pairing(ctx, browser.as_deref())
            }
            Request::CompletePairing { code, .. } => session_ops::complete_pairing(ctx, code),
            Request::RequestUnlock { .. } => session_ops::request_unlock(ctx),
            Request::UnlockWithPassword { password, .. } => {
                session_ops::unlock_with_password(ctx, std::mem::take(password))
            }
            Request::UnlockWithDevice { automatic, .. } => {
                session_ops::unlock_with_device(ctx, *automatic)
            }
            Request::GetUnlockOptions { .. } => session_ops::get_unlock_options(ctx),
            Request::LockVault { .. } => session_ops::lock_vault(ctx),
            Request::OpenApp { new_entry, .. } => session_ops::open_app(ctx, *new_entry),
            Request::OpenBrowserSettings { .. } => session_ops::open_browser_settings(ctx),
            Request::ReportBrowserDiagnostics {
                report, pairing, ..
            } => session_ops::report_browser_diagnostics(ctx, report, pairing.as_deref()),
            Request::GetPasskeyProxyStatus { .. }
            | Request::SetPasskeyProxyEnabled { .. }
            | Request::PasskeyCreate { .. }
            | Request::PasskeyCreateConfirm { .. }
            | Request::PasskeyGet { .. }
            | Request::PasskeyGetApprove { .. }
            | Request::PasskeyCancel { .. } => ctx.deps.passkeys.handle(
                &PasskeyContext {
                    session: &ctx.deps.session,
                    settings: &ctx.deps.settings,
                    unlocked,
                },
                request,
            ),
            Request::GeneratePassword { domain, .. } => generator::generate_password(ctx, domain),
            Request::RecordGeneratedPassword {
                password,
                url,
                created_at,
                ..
            } => generator::record_generated_password(ctx, password, url, *created_at),
            Request::GetGeneratedPasswordHistory { .. } => {
                generator::get_generated_password_history(ctx)
            }
            Request::DeleteGeneratedPassword { id, .. } => {
                generator::delete_generated_password(ctx, id)
            }
            Request::RecordSocialSignIn {
                page_url,
                provider,
                source,
                ..
            } => social::record_social_sign_in(ctx, page_url, provider, *source),
            Request::GetSocialSignIn { url, .. } => social::get_social_sign_in(ctx, url),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::protocol::REQUEST_TYPES;
    use crate::testing::*;
    use crate::ui::Proposal;
    use mypass_session::session::LockReason;
    use std::sync::atomic::Ordering;
    use zeroize::Zeroizing;

    #[test]
    fn observer_registered_while_unlocked_sees_the_first_vault_switch() {
        let f = fx();
        assert!(f.session.is_unlocked());
        // A second service registered after the vault was opened (a bridge restart).
        let recording = Arc::new(RecordingPasskeys::default());
        let proposals = Arc::new(ProposalStore::new(f.sched.clone()));
        let _late = BridgeService::new(BridgeDeps {
            session: f.session.clone(),
            autolock: mypass_session::autolock::AutoLock::new(
                f.session.clone(),
                f.settings.clone(),
                f.sched.clone(),
            ),
            settings: f.settings.clone(),
            scheduler: f.sched.clone(),
            pairing: Arc::new(
                PairingManager::new(
                    crate::pairing::PairedBrowserStore::new(
                        mypass_platform::testing::InMemorySecretStore::new(),
                    ),
                    f.sched.clone(),
                )
                .unwrap(),
            ),
            diagnostics: Arc::new(DiagnosticsStore::new()),
            proposals,
            ui: f.ui.clone(),
            passkeys: recording.clone(),
            unlocker: f.unlocker.clone(),
        });
        // A direct switch to another vault: its first `StateChanged` is already `Unlocked`.
        let dir = tempfile::tempdir().unwrap();
        f.session
            .create_vault_with(
                &dir.path().join("other.kdbx"),
                secrecy::SecretString::from("pw".to_owned()),
                mypass_session::testing::fast_database("Other"),
            )
            .unwrap();
        assert!(f.session.is_unlocked());
        assert_eq!(recording.locked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_table_matches_wire_types() {
        let all = all_requests(None);
        assert_eq!(all.len(), 35);
        for (r, name) in all.iter().zip(REQUEST_TYPES) {
            assert_eq!(r.name(), name);
        }
    }

    #[test]
    fn quit_in_progress_rejects_every_request_with_exact_message() {
        let f = fx();
        f.session.begin_quit();
        for pairing in [Some(f.secret.clone()), None] {
            for r in all_requests(pairing.as_deref()) {
                assert_eq!(error_message(&f.svc.handle(r)), "MyPass is quitting");
            }
        }
        f.session.lock(LockReason::Manual);
        assert!(!f.session.is_unlocked());
        for r in all_requests(None) {
            assert_eq!(error_message(&f.svc.handle(r)), "MyPass is quitting");
        }
    }

    #[test]
    fn cancel_quit_restores_service_answers() {
        let (f, _svc) = passkey_fx();
        let r = || named("getPasskeyProxyStatus", Some(&f.secret));
        f.session.begin_quit();
        assert!(matches!(f.svc.handle(r()), Response::Error { .. }));
        f.session.cancel_quit();
        assert!(matches!(
            f.svc.handle(r()),
            Response::PasskeyProxyStatus { .. }
        ));
    }

    #[test]
    fn gate_order_is_quitting_then_pairing_then_locked() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.svc.handle(named("getCredentials", None)),
            Response::PairingRequired
        );
        assert_eq!(
            f.svc.handle(named("getCredentials", Some(&f.secret))),
            Response::Locked
        );
        f.session.begin_quit();
        assert_eq!(
            error_message(&f.svc.handle(named("getCredentials", None))),
            "MyPass is quitting"
        );
    }

    fn password_attempt(f: &Fx, password: &str, proof: Option<&str>) -> Response {
        f.svc.handle(Request::UnlockWithPassword {
            token: "tok".into(),
            password: crate::protocol::WirePassword::new(password.into()),
            pairing: proof.map(str::to_owned),
        })
    }

    fn wrong_password(f: &Fx) -> Response {
        f.unlocker
            .script_password(Err(crate::unlock::UnlockFailure::WrongPassword));
        password_attempt(f, "wrong", Some(&f.secret))
    }

    fn reach_backoff(f: &Fx) {
        for _ in 0..4 {
            assert_eq!(wrong_password(f), Response::WrongPassword);
        }
        assert_eq!(
            wrong_password(f),
            Response::UnlockThrottled {
                retry_after_secs: 30
            }
        );
    }

    #[test]
    fn unpaired_unlock_with_password_is_pairing_required_and_never_reaches_the_unlocker() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(password_attempt(&f, "pw", None), Response::PairingRequired);
        assert_eq!(f.unlocker.password_calls(), 0);
    }

    #[test]
    fn quitting_refuses_unlock_requests() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.session.begin_quit();
        for name in ["unlockWithPassword", "unlockWithDevice", "getUnlockOptions"] {
            assert_eq!(
                error_message(&f.svc.handle(named(name, Some(&f.secret)))),
                "MyPass is quitting"
            );
        }
        assert_eq!(f.unlocker.password_calls(), 0);
        assert_eq!(f.unlocker.device_calls(), 0);
    }

    #[test]
    fn paired_password_unlock_while_locked_answers_unlocked() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.unlocker.script_password(Ok(()));
        assert_eq!(
            password_attempt(&f, "pw", Some(&f.secret)),
            Response::Unlocked
        );
        assert_eq!(f.unlocker.password_calls(), 1);
    }

    #[test]
    fn wrong_password_then_fifth_is_throttled_over_the_bridge() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        reach_backoff(&f);
        assert_eq!(
            password_attempt(&f, "pw", Some(&f.secret)),
            Response::UnlockThrottled {
                retry_after_secs: 30
            }
        );
        assert_eq!(f.unlocker.password_calls(), 5);
    }

    #[test]
    fn unlock_when_already_unlocked_is_not_needed_and_skips_the_unlocker() {
        let f = fx();
        for name in ["unlockWithPassword", "unlockWithDevice"] {
            assert_eq!(
                f.svc.handle(named(name, Some(&f.secret))),
                Response::UnlockNotNeeded
            );
        }
        assert_eq!(f.unlocker.password_calls(), 0);
        assert_eq!(f.unlocker.device_calls(), 0);
    }

    #[test]
    fn overlong_password_is_invalid_request_and_not_counted() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        let long = "x".repeat(mypass_core::text::MAX_PASSWORD_CHARS + 1);
        assert_eq!(
            error_message(&password_attempt(&f, &long, Some(&f.secret))),
            "invalid request"
        );
        assert_eq!(f.unlocker.password_calls(), 0);
        reach_backoff(&f);
    }

    #[test]
    fn in_app_failure_maps_to_unlock_in_app() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.unlocker
            .script_password(Err(crate::unlock::UnlockFailure::InApp));
        f.unlocker
            .script_device(Err(crate::unlock::UnlockFailure::InApp));
        for name in ["unlockWithPassword", "unlockWithDevice"] {
            assert_eq!(
                f.svc.handle(named(name, Some(&f.secret))),
                Response::UnlockInApp
            );
        }
    }

    #[test]
    fn device_unlock_maps_cancelled_and_unavailable() {
        use crate::unlock::UnlockFailure;
        let f = fx();
        f.session.lock(LockReason::Manual);
        let request = || named("unlockWithDevice", Some(&f.secret));
        for (failure, response) in [
            (UnlockFailure::Cancelled, Response::UnlockCancelled),
            (
                UnlockFailure::DeviceUnavailable,
                Response::DeviceUnlockUnavailable,
            ),
            (UnlockFailure::Failed, error("unlock_failed")),
        ] {
            f.unlocker.script_device(Err(failure));
            assert_eq!(f.svc.handle(request()), response);
            f.sched.advance(Duration::from_secs(2));
        }
        reach_backoff(&f);
        f.unlocker.script_device(Ok(()));
        assert_eq!(f.svc.handle(request()), Response::Unlocked);
        assert_eq!(wrong_password(&f), Response::WrongPassword);
    }

    #[test]
    fn unpaired_unlock_options_are_pairing_required_while_locked() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.svc.handle(named("getUnlockOptions", None)),
            Response::PairingRequired,
        );
    }

    #[test]
    fn repeated_device_requests_within_two_seconds_do_not_prompt_again() {
        use crate::unlock::UnlockFailure;
        let f = fx();
        f.session.lock(LockReason::Manual);
        let request = || named("unlockWithDevice", Some(&f.secret));
        f.unlocker.script_device(Err(UnlockFailure::Cancelled));
        f.unlocker
            .script_device(Err(UnlockFailure::DeviceUnavailable));
        assert_eq!(f.svc.handle(request()), Response::UnlockCancelled);
        f.sched.advance(Duration::from_millis(1999));
        assert_eq!(f.svc.handle(request()), Response::UnlockCancelled);
        assert_eq!(f.unlocker.device_calls(), 1);
        f.sched.advance(Duration::from_millis(1));
        assert_eq!(f.svc.handle(request()), Response::DeviceUnlockUnavailable);
        assert_eq!(f.unlocker.device_calls(), 2);
    }

    #[test]
    fn unlock_options_reports_offer_and_running_backoff() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.unlocker.set_offer(crate::unlock::DeviceOffer {
            offered: true,
            touch_id: true,
        });
        let request = || named("getUnlockOptions", Some(&f.secret));
        assert_eq!(
            f.svc.handle(request()),
            Response::UnlockOptions {
                device_unlock: true,
                touch_id: true,
                device_failures: 0,
                retry_after_secs: None,
            }
        );
        reach_backoff(&f);
        assert_eq!(
            f.svc.handle(request()),
            Response::UnlockOptions {
                device_unlock: true,
                touch_id: true,
                device_failures: 0,
                retry_after_secs: Some(30),
            }
        );
    }

    fn device_request(f: &Fx, automatic: bool) -> Request {
        Request::UnlockWithDevice {
            token: "tok".to_owned(),
            automatic,
            pairing: Some(f.secret.clone()),
        }
    }

    fn device_failures(f: &Fx) -> u32 {
        let response = f.svc.handle(named("getUnlockOptions", Some(&f.secret)));
        let Response::UnlockOptions {
            device_failures, ..
        } = response
        else {
            panic!("expected UnlockOptions, got {response:?}");
        };
        device_failures
    }

    #[test]
    fn not_recognized_device_prompts_are_counted_for_the_popup() {
        use crate::unlock::UnlockFailure;
        let f = fx();
        f.session.lock(LockReason::Manual);
        for count in 1..=3 {
            f.unlocker.script_device(Err(UnlockFailure::NotRecognized));
            assert_eq!(
                f.svc.handle(device_request(&f, false)),
                Response::DeviceNotRecognized {
                    device_failures: count
                }
            );
            assert_eq!(device_failures(&f), count);
            f.sched.advance(Duration::from_secs(2));
        }
        // Not recognized never feeds the password backoff.
        assert_eq!(wrong_password(&f), Response::WrongPassword);
    }

    #[test]
    fn cancelled_and_dropped_device_requests_never_count_a_failure() {
        use crate::unlock::UnlockFailure;
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.unlocker.script_device(Err(UnlockFailure::Cancelled));
        assert_eq!(
            f.svc.handle(device_request(&f, true)),
            Response::UnlockCancelled
        );
        // A popup reopened within the debounce of the last prompt start prompts nothing.
        assert_eq!(
            f.svc.handle(device_request(&f, true)),
            Response::UnlockCancelled
        );
        assert_eq!(f.unlocker.device_calls(), 1);
        assert_eq!(device_failures(&f), 0);
    }

    #[test]
    fn the_automatic_flag_reaches_the_unlocker() {
        use crate::unlock::UnlockFailure;
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.unlocker.script_device(Err(UnlockFailure::Cancelled));
        f.unlocker.script_device(Err(UnlockFailure::Cancelled));
        f.svc.handle(device_request(&f, false));
        assert_eq!(f.unlocker.automatic_device_calls(), 0);
        f.sched.advance(Duration::from_secs(2));
        f.svc.handle(device_request(&f, true));
        assert_eq!(f.unlocker.device_calls(), 2);
        assert_eq!(f.unlocker.automatic_device_calls(), 1);
    }

    #[test]
    fn an_unlock_clears_the_device_failure_count_for_the_next_lock_spell() {
        use crate::unlock::UnlockFailure;
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.unlocker.script_device(Err(UnlockFailure::NotRecognized));
        f.svc.handle(device_request(&f, false));
        assert_eq!(device_failures(&f), 1);
        f.session.unlock(secrecy::SecretString::from("pw")).unwrap();
        f.session.lock(LockReason::Manual);
        assert_eq!(device_failures(&f), 0);
    }

    #[test]
    fn session_unlock_resets_the_unlock_throttle() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        reach_backoff(&f);
        f.session.unlock(secrecy::SecretString::from("pw")).unwrap();
        f.session.lock(LockReason::Manual);
        assert_eq!(wrong_password(&f), Response::WrongPassword);
    }

    #[test]
    fn pairing_requests_are_answerable_while_locked() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        f.sched.advance(crate::pairing::START_MIN_INTERVAL);
        assert_eq!(
            f.svc.handle(named("startPairing", None)),
            Response::PairingStarted
        );
        let displayed = f.ui.details().pop().unwrap();
        let wrong = if displayed.ends_with("000000") {
            "000001"
        } else {
            "000000"
        };
        assert_eq!(
            error_message(&f.svc.handle(Request::CompletePairing {
                token: "tok".into(),
                code: wrong.into()
            })),
            "invalid_code"
        );
    }

    #[test]
    fn unpaired_request_gets_pairing_required_when_unlocked() {
        let f = fx();
        let mut n = 0;
        for wrong in [None, Some("not-a-secret")] {
            for r in all_requests(wrong) {
                if r.requires_pairing() {
                    n += 1;
                    assert_eq!(f.svc.handle(r), Response::PairingRequired);
                }
            }
        }
        assert_eq!(n, 66);
    }

    #[test]
    fn handshake_requests_work_unpaired() {
        let f = fx();
        for name in ["startPairing", "completePairing"] {
            let r = f.svc.handle(named(name, None));
            assert!(
                !matches!(r, Response::PairingRequired | Response::Locked),
                "{name}"
            );
        }
    }

    #[test]
    fn activity_recorded_only_when_served() {
        let f = fx();
        let fill = named("fillCredential", Some(&f.secret));
        let served = Response::Credential {
            username: String::new(),
            password: String::new(),
            totp: None,
            urls: vec![],
        };
        let refused = error("entry not found or domain mismatch");
        assert!(resets_timer(&f, || f.svc.note_activity(&fill, &served)));
        let f = fx();
        assert!(!resets_timer(&f, || f.svc.note_activity(&fill, &refused)));
        let f = fx();
        assert!(!resets_timer(&f, || f
            .svc
            .note_activity(&fill, &Response::Locked)));
        let f = fx();
        let get = named("getCredentials", Some(&f.secret));
        let list = Response::CredentialList {
            credentials: vec![],
        };
        assert!(!resets_timer(&f, || f.svc.note_activity(&get, &list)));
        // End to end: an unpaired FillCredential is refused and does not reset the timer.
        let f = fx();
        assert!(!resets_timer(&f, || {
            f.svc.handle(named("fillCredential", None));
        }));
    }

    #[test]
    fn lock_vault_never_resets_timer() {
        let f = fx();
        let lock = named("lockVault", Some(&f.secret));
        assert!(!resets_timer(&f, || f
            .svc
            .note_activity(&lock, &Response::Acknowledged)));
    }

    #[test]
    fn lock_racing_requests_never_return_data_after_lock() {
        // Sanity: while unlocked the request really returns the title, so the checks below bite.
        let f = fx();
        let r = f.svc.handle(named("listEntries", Some(&f.secret)));
        assert!(serde_json::to_string(&r)
            .unwrap()
            .contains("SecretTitleXyz"));
        for _ in 0..200 {
            let f = fx();
            let locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reader = {
                let svc = f.svc.clone();
                let locked = locked.clone();
                let secret = f.secret.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        let after_lock = locked.load(Ordering::SeqCst);
                        let r = svc.handle(named("listEntries", Some(&secret)));
                        let text = serde_json::to_string(&r).unwrap();
                        // Either the list was read before the lock, or the vault answered Locked.
                        if let Response::CredentialList { .. } = &r {
                            assert!(!after_lock, "data after the lock: {text}");
                            assert!(text.contains("SecretTitleXyz"));
                        } else {
                            assert_eq!(r, Response::Locked, "{text}");
                            assert!(!text.contains("SecretTitleXyz"));
                        }
                    }
                })
            };
            f.session.lock(LockReason::Manual);
            locked.store(true, Ordering::SeqCst);
            reader.join().unwrap();
        }
    }

    #[test]
    fn passkey_status_answers_while_locked_from_the_real_service() {
        let (f, _svc) = passkey_fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.svc
                .handle(named("getPasskeyProxyStatus", Some(&f.secret))),
            Response::PasskeyProxyStatus {
                unlocked: false,
                paired: true,
                attach_allowed: false,
                enabled: true,
                remember_social_sign_in: true,
            }
        );
    }

    #[test]
    fn passkey_requests_but_the_status_answer_locked_while_locked() {
        // R4: the lock gate answers first, whatever the toggle says.
        for toggle in [true, false] {
            let (f, svc) = passkey_fx();
            f.settings
                .update(mypass_session::settings::SettingsPatch {
                    passkey_proxy_enabled: Some(toggle),
                    ..mypass_session::settings::SettingsPatch::default()
                })
                .unwrap();
            f.session.lock(LockReason::Manual);
            for name in [
                "setPasskeyProxyEnabled",
                "passkeyCreate",
                "passkeyCreateConfirm",
                "passkeyGet",
                "passkeyGetApprove",
                "passkeyCancel",
            ] {
                assert_eq!(
                    f.svc.handle(named(name, Some(&f.secret))),
                    Response::Locked,
                    "{name}"
                );
            }
            assert_eq!(svc.pending_len(), (0, 0));
        }
    }

    #[test]
    fn passkey_cancel_answers_cancelled_with_same_id() {
        let (f, _svc) = passkey_fx();
        assert_eq!(
            f.svc.handle(named("passkeyCancel", Some(&f.secret))),
            Response::PasskeyCancelled { request_id: -9 }
        );
    }

    #[test]
    fn passkey_confirm_never_resets_timer_when_error() {
        let (f, _svc) = passkey_fx();
        let confirm = named("passkeyCreateConfirm", Some(&f.secret));
        assert!(!resets_timer(&f, || {
            assert!(matches!(
                f.svc.handle(confirm),
                Response::PasskeyCreateComplete {
                    error: Some(_),
                    response_json: None,
                    ..
                }
            ));
        }));
    }

    #[test]
    fn lock_clears_proposals_and_notifies_passkey_handler() {
        let rec = Arc::new(RecordingPasskeys::default());
        let f = fx_with(rec.clone());
        let id = f.proposals.put(Proposal::Login {
            url: "https://a.example".into(),
            username: "u".into(),
            password: Zeroizing::new("pw".into()),
            extras: Vec::new(),
            offer_address: false,
            username_is_email: false,
        });
        f.svc
            .handle(named("getPasskeyProxyStatus", Some(&f.secret)));
        assert_eq!(rec.handled.load(Ordering::SeqCst), 1);
        assert_eq!(rec.locked.load(Ordering::SeqCst), 0);
        f.session.lock(LockReason::Manual);
        assert!(f.proposals.take(&id).is_none());
        assert_eq!(rec.locked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn passkey_dispatch_arm_matches_is_passkey() {
        // Every request reaches the passkey handler exactly when `Request::is_passkey` says so.
        let n = all_requests(None).len();
        for i in 0..n {
            let rec = Arc::new(RecordingPasskeys::default());
            let f = fx_with(rec.clone());
            let request = all_requests(Some(&f.secret)).swap_remove(i);
            let name = request.name();
            let is_passkey = request.is_passkey();
            f.svc.handle(request);
            assert_eq!(
                rec.handled.load(Ordering::SeqCst),
                usize::from(is_passkey),
                "{name}"
            );
        }
    }

    #[test]
    fn dispatch_match_has_no_wildcard_arm() {
        let src = include_str!("service.rs");
        let body = src.split("fn dispatch(").nth(1).unwrap();
        let body = body.split("#[cfg(test)]").next().unwrap();
        assert!(!body.contains("_ =>"));
        assert!(src.contains("#![deny(clippy::wildcard_enum_match_arm)]"));
    }
}
