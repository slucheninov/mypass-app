//! The bridge service: every request from the extension passes the gates here, then reaches its
//! handler.
//!
//! Order (load-bearing, ported from the Swift app):
//! 1. quitting: every request is refused, so nothing mutates the vault during the quit flow;
//! 2. locked: only the requests that touch no vault data are answered, the rest get `Locked`;
//! 3. pairing: everything but the two handshake requests needs a paired browser;
//! 4. dispatch, an exhaustive `match` over the request enum (a new request must pick a handler);
//! 5. a served request that is a deliberate user gesture resets the idle auto-lock timer.
//!
//! Why locked comes before pairing: the extension shows "unlock MyPass" the moment the vault is
//! locked, before it has any reason to pair, and pairing is refused while locked, which gives the
//! "unlock, then connect" flow (the pairing code is shown only to a user who can see the app). The
//! price is a one-bit leak: a peer that holds the endpoint token but is not paired learns whether
//! the vault is locked (`Locked` against `PairingRequired`). That bit is accepted; it matches the
//! Swift app and reveals no vault data. The handlers re-check the lock under the session's own lock
//! (`with_db`), so this gate is an early answer, never the guard of the data.
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
use crate::handlers::{browse, create, fill, generator, identifier, session_ops};
use crate::pairing::PairingManager;
use crate::passkeys::{PasskeyContext, PasskeyHandler};
use crate::protocol::{Request, Response};
use crate::ui::{BridgeUi, ProposalStore};
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
}

/// Built once per request and handed to the handler.
pub(crate) struct Ctx<'a> {
    pub deps: &'a BridgeDeps,
    /// The injected wall clock at the start of the request.
    pub now: SystemTime,
    /// Monotonic time of the last `requestUnlock` that was shown to the user (the debounce).
    pub last_unlock_request: &'a Mutex<Option<Duration>>,
}

pub struct BridgeService {
    deps: BridgeDeps,
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
            deps,
            last_unlock_request: Mutex::new(None),
        })
    }

    /// Registers the session observer (call once).
    pub fn register_observer(&self) {
        let proposals = self.deps.proposals.clone();
        let passkeys = self.deps.passkeys.clone();
        // A direct switch from one unlocked vault to another emits only `StateChanged(Unlocked)`
        // with a new path: a proposal made for the old vault must not be taken into the new one,
        // and a pending passkey ceremony (its candidates are entry ids of the old vault) must not
        // be confirmed or approved against the new one.
        let open_path: Mutex<Option<String>> = Mutex::new(None);
        self.deps
            .session
            .add_observer(Arc::new(move |e: &SessionEvent| {
                if let SessionEvent::StateChanged(status) = e {
                    let mut open = open_path.lock().unwrap_or_else(PoisonError::into_inner);
                    if status.state != SessionState::Unlocked {
                        *open = None;
                        proposals.clear();
                        passkeys.on_session_locked();
                    } else {
                        if open.is_some() && *open != status.vault_path {
                            proposals.clear();
                            passkeys.on_session_locked();
                        }
                        *open = status.vault_path.clone();
                    }
                }
            }));
    }

    /// A context for calling a handler directly, past the gates.
    #[cfg(test)]
    pub(crate) fn ctx_for_test(&self) -> Ctx<'_> {
        Ctx {
            deps: &self.deps,
            now: self.deps.scheduler.wall_now(),
            last_unlock_request: &self.last_unlock_request,
        }
    }

    /// Answers one request. Never panics, never returns vault data to a locked, unpaired or
    /// quitting caller.
    pub fn handle(&self, request: Request) -> Response {
        let deps = &self.deps;
        if deps.session.is_quitting() {
            return error("MyPass is quitting");
        }
        let unlocked = deps.session.is_unlocked();
        if !unlocked && !request.is_answerable_while_locked() {
            return Response::Locked;
        }
        if request.requires_pairing() && !deps.pairing.is_authorized(request.pairing()) {
            return Response::PairingRequired;
        }
        let ctx = Ctx {
            deps,
            now: deps.scheduler.wall_now(),
            last_unlock_request: &self.last_unlock_request,
        };
        let response = Self::dispatch(&ctx, &request, unlocked);
        self.note_activity(&request, &response);
        response
    }

    /// Resets the idle timer for a served, gesture-gated request (see `resets_auto_lock_timer`).
    pub(crate) fn note_activity(&self, request: &Request, response: &Response) {
        if request.resets_auto_lock_timer() && response.was_served() {
            self.deps.autolock.record_activity();
        }
    }

    fn dispatch(ctx: &Ctx<'_>, request: &Request, unlocked: bool) -> Response {
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
            Request::FillCredential { id, url, .. } => fill::fill_credential(ctx, id, url),
            Request::GetTotp { id, url, .. } => fill::get_totp(ctx, id, url),
            Request::ProposeSaveLogin {
                url,
                username,
                password,
                ..
            } => session_ops::propose_save_login(ctx, url, username, password),
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
            Request::LockVault { .. } => session_ops::lock_vault(ctx),
            Request::OpenApp { .. } => session_ops::open_app(ctx),
            Request::OpenBrowserSettings { .. } => session_ops::open_browser_settings(ctx),
            Request::ReportBrowserDiagnostics { report, .. } => {
                session_ops::report_browser_diagnostics(ctx, report, request.pairing())
            }
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
    fn request_table_matches_wire_types() {
        let all = all_requests(None);
        assert_eq!(all.len(), 29);
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
    fn locked_gate_runs_before_pairing_gate() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.svc.handle(named("getCredentials", None)),
            Response::Locked
        );
        assert_eq!(
            f.svc.handle(named("openApp", None)),
            Response::PairingRequired
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
        assert_eq!(n, 54);
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
                enabled: true
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
