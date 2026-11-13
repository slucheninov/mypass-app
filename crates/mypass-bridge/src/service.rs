//! The bridge service: every request from the extension passes the gates here, then reaches its
//! handler.
//!
//! Order (load-bearing, ported from the Swift app):
//! 1. quitting: every request is refused, so nothing mutates the vault during the quit flow;
//! 2. locked: only the requests that touch no vault data are answered;
//! 3. pairing: everything but the two handshake requests needs a paired browser;
//! 4. dispatch, an exhaustive `match` over the request enum (a new request must pick a handler);
//! 5. a served request that is a deliberate user gesture resets the idle auto-lock timer.
//!
//! The bridge always searches with `SearchScope::Autofill` (Task 11 onwards): free text the
//! extension has no use for is never matched.

#![deny(clippy::wildcard_enum_match_arm)]

use std::sync::Arc;
use std::time::SystemTime;

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
    #[allow(dead_code)] // read by the handlers of Tasks 12 to 15
    pub now: SystemTime,
}

pub struct BridgeService {
    deps: BridgeDeps,
}

impl std::fmt::Debug for BridgeService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeService").finish_non_exhaustive()
    }
}

impl BridgeService {
    /// Builds the service and registers the session observer that drops pending proposals and
    /// tells the passkey handler when the vault stops being unlocked.
    pub fn new(deps: BridgeDeps) -> Arc<Self> {
        let proposals = deps.proposals.clone();
        let passkeys = deps.passkeys.clone();
        deps.session.add_observer(Arc::new(move |e: &SessionEvent| {
            if let SessionEvent::StateChanged(status) = e {
                if status.state != SessionState::Unlocked {
                    proposals.clear();
                    passkeys.on_session_locked();
                }
            }
        }));
        Arc::new(Self { deps })
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
    use crate::pairing::{PairedBrowserStore, PairingChallenge, StartOutcome};
    use crate::passkeys::NotSupportedPasskeys;
    use crate::protocol::{
        BrowserDiagnosticsReport, ExtensionScanState, PermissionState, ProxyState, REQUEST_TYPES,
    };
    use crate::ui::Proposal;
    use mypass_kdbx::{Entry, PlainFileIo, Timestamp};
    use mypass_platform::testing::InMemorySecretStore;
    use mypass_session::scheduler::ManualScheduler;
    use mypass_session::session::{LockReason, SessionDeps};
    use mypass_session::settings::{IdleTimeout, SettingsPatch};
    use mypass_session::testing::fast_database;
    use secrecy::SecretString;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, UNIX_EPOCH};
    use uuid::Uuid;
    use zeroize::Zeroizing;

    /// Records every call; does not override `passkey_proxy_changed`.
    #[derive(Default)]
    struct RecordingUi {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingUi {
        fn note(&self, s: &str) {
            self.calls.lock().unwrap().push(s.to_owned());
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl BridgeUi for RecordingUi {
        fn pairing_code(&self, _c: &PairingChallenge) {
            self.note("pairing_code");
        }
        fn pairing_finished(&self) {
            self.note("pairing_finished");
        }
        fn save_login_proposal(&self, _id: &str, _url: &str, _username: &str) {
            self.note("save_login_proposal");
        }
        fn save_totp_proposal(&self, _id: &str, _url: &str, _entry_id: Option<Uuid>) {
            self.note("save_totp_proposal");
        }
        fn unlock_requested(&self) {
            self.note("unlock_requested");
        }
        fn open_app_requested(&self) {
            self.note("open_app_requested");
        }
        fn open_browser_settings_requested(&self) {
            self.note("open_browser_settings_requested");
        }
        fn credential_filled(&self, _id: Uuid) {
            self.note("credential_filled");
        }
    }

    #[derive(Default)]
    struct RecordingPasskeys {
        handled: AtomicUsize,
        locked: AtomicUsize,
    }

    impl PasskeyHandler for RecordingPasskeys {
        fn handle(&self, _ctx: &PasskeyContext<'_>, _request: &Request) -> Response {
            self.handled.fetch_add(1, Ordering::SeqCst);
            Response::Acknowledged
        }
        fn on_session_locked(&self) {
            self.locked.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Fx {
        svc: Arc<BridgeService>,
        session: VaultSession,
        sched: Arc<ManualScheduler>,
        ui: Arc<RecordingUi>,
        proposals: Arc<ProposalStore>,
        secret: String,
        _dir: tempfile::TempDir,
    }

    fn fx_with(passkeys: Arc<dyn PasskeyHandler>) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let settings = SettingsStore::in_memory();
        settings
            .update(SettingsPatch {
                auto_lock_idle_timeout: Some(IdleTimeout::OneMinute),
                ..SettingsPatch::default()
            })
            .unwrap();
        let sched = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let session = VaultSession::new(SessionDeps {
            settings: settings.clone(),
            scheduler: sched.clone(),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        let autolock = AutoLock::new(session.clone(), settings.clone(), sched.clone());
        let pairing = Arc::new(
            PairingManager::new(
                PairedBrowserStore::new(InMemorySecretStore::new()),
                sched.clone(),
            )
            .unwrap(),
        );
        let secret = match pairing.start(Some("Chrome")) {
            StartOutcome::Started(c) => pairing.complete(&c.code).unwrap().unwrap(),
            StartOutcome::Throttled => panic!("throttled"),
        };
        let ui = Arc::new(RecordingUi::default());
        let proposals = Arc::new(ProposalStore::new(sched.clone()));
        let mut db = fast_database("Bridge Test");
        let now = Timestamp::now();
        let mut e = Entry::new(now);
        e.set_field("Title", "SecretTitleXyz", false);
        let root = db.root_id();
        db.add_entry(root, e, now).unwrap();
        let svc = BridgeService::new(BridgeDeps {
            session: session.clone(),
            autolock,
            settings,
            scheduler: sched.clone(),
            pairing,
            diagnostics: Arc::new(DiagnosticsStore::new()),
            proposals: proposals.clone(),
            ui: ui.clone(),
            passkeys,
        });
        session
            .create_vault_with(
                &dir.path().join("v.kdbx"),
                SecretString::from("pw".to_owned()),
                db,
            )
            .unwrap();
        Fx {
            svc,
            session,
            sched,
            ui,
            proposals,
            secret,
            _dir: dir,
        }
    }

    fn fx() -> Fx {
        fx_with(Arc::new(NotSupportedPasskeys))
    }

    fn report() -> BrowserDiagnosticsReport {
        BrowserDiagnosticsReport {
            proxy_state: ProxyState::Disabled,
            permission_state: PermissionState::NotGranted,
            password_saving_enabled: None,
            extension_scan_state: ExtensionScanState::NotChecked,
            enabled_managers: vec![],
        }
    }

    /// One of every request, in `REQUEST_TYPES` order.
    fn all_requests(pairing: Option<&str>) -> Vec<Request> {
        let t = || "tok".to_owned();
        let p = || pairing.map(str::to_owned);
        let s = || "x".to_owned();
        vec![
            Request::GetCredentials {
                token: t(),
                url: "https://a.example".into(),
                pairing: p(),
            },
            Request::GetFavorites {
                token: t(),
                pairing: p(),
            },
            Request::ListEntries {
                token: t(),
                category: "all".into(),
                pairing: p(),
            },
            Request::SearchEntries {
                token: t(),
                query: s(),
                pairing: p(),
            },
            Request::CreateEntry {
                token: t(),
                entry_type: "Login".into(),
                title: s(),
                username: s(),
                password: s(),
                url: s(),
                card_number: s(),
                cardholder_name: s(),
                expiry: s(),
                cvv: s(),
                notes: s(),
                favorite: false,
                r#match: s(),
                otp: s(),
                pairing: p(),
            },
            Request::FillAddress {
                token: t(),
                id: s(),
                url: s(),
                pairing: p(),
            },
            Request::FillCredential {
                token: t(),
                id: s(),
                url: s(),
                pairing: p(),
            },
            Request::GetTotp {
                token: t(),
                id: s(),
                url: s(),
                pairing: p(),
            },
            Request::ProposeSaveLogin {
                token: t(),
                url: s(),
                username: s(),
                password: s(),
                pairing: p(),
            },
            Request::AddEntryIdentifier {
                token: t(),
                id: s(),
                url: s(),
                value: s(),
                field_key: s(),
                pairing: p(),
            },
            Request::ProposeSaveTotp {
                token: t(),
                url: s(),
                otpauth_or_secret: s(),
                entry_id: None,
                pairing: p(),
            },
            Request::StartPairing {
                token: t(),
                browser: None,
            },
            Request::CompletePairing {
                token: t(),
                code: "000000".into(),
            },
            Request::RequestUnlock {
                token: t(),
                pairing: p(),
            },
            Request::LockVault {
                token: t(),
                pairing: p(),
            },
            Request::OpenApp {
                token: t(),
                pairing: p(),
            },
            Request::OpenBrowserSettings {
                token: t(),
                pairing: p(),
            },
            Request::ReportBrowserDiagnostics {
                token: t(),
                report: report(),
                pairing: p(),
            },
            Request::GetPasskeyProxyStatus {
                token: t(),
                pairing: p(),
            },
            Request::SetPasskeyProxyEnabled {
                token: t(),
                enabled: true,
                pairing: p(),
            },
            Request::PasskeyCreate {
                token: t(),
                request_id: 7,
                request_details_json: "{}".into(),
                origin_url: s(),
                pairing: p(),
            },
            Request::PasskeyCreateConfirm {
                token: t(),
                request_id: 7,
                entry_id: None,
                replace: false,
                pairing: p(),
            },
            Request::PasskeyGet {
                token: t(),
                request_id: 7,
                request_details_json: "{}".into(),
                origin_url: s(),
                pairing: p(),
            },
            Request::PasskeyGetApprove {
                token: t(),
                request_id: 7,
                entry_id: s(),
                pairing: p(),
            },
            Request::PasskeyCancel {
                token: t(),
                request_id: -9,
                pairing: p(),
            },
            Request::GeneratePassword {
                token: t(),
                domain: s(),
                pairing: p(),
            },
            Request::RecordGeneratedPassword {
                token: t(),
                password: s(),
                url: s(),
                created_at: None,
                pairing: p(),
            },
            Request::GetGeneratedPasswordHistory {
                token: t(),
                pairing: p(),
            },
            Request::DeleteGeneratedPassword {
                token: t(),
                id: s(),
                pairing: p(),
            },
        ]
    }

    fn named(name: &str, pairing: Option<&str>) -> Request {
        all_requests(pairing)
            .into_iter()
            .find(|r| r.name() == name)
            .unwrap()
    }

    fn error_message(r: &Response) -> &str {
        let Response::Error { message } = r else {
            panic!("expected Error, got {r:?}");
        };
        message
    }

    /// Whether `act` reset the idle timer: 50 s idle, act, 50 s more; a one minute timeout still
    /// has the session unlocked only when the timer was reset in between.
    fn resets_timer(f: &Fx, act: impl FnOnce()) -> bool {
        f.sched.advance(Duration::from_secs(50));
        act();
        f.sched.advance(Duration::from_secs(50));
        f.session.is_unlocked()
    }

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
        let f = fx();
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
    fn bridge_ui_default_passkey_proxy_changed_is_noop() {
        let f = fx();
        f.ui.passkey_proxy_changed(true);
        let r = f
            .svc
            .handle(named("setPasskeyProxyEnabled", Some(&f.secret)));
        assert_eq!(r, Response::PasskeyProxyEnabled { enabled: false });
        assert!(f.ui.calls().is_empty());
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
                        assert!(!text.contains("SecretTitleXyz"));
                        if after_lock {
                            assert_eq!(r, Response::Locked);
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
    fn passkey_status_placeholder_reports_disabled_and_stays_locked_safe() {
        let f = fx();
        f.session.lock(LockReason::Manual);
        assert_eq!(
            f.svc
                .handle(named("getPasskeyProxyStatus", Some(&f.secret))),
            Response::PasskeyProxyStatus {
                unlocked: false,
                paired: true,
                attach_allowed: false,
                enabled: false
            }
        );
    }

    #[test]
    fn set_passkey_proxy_enabled_placeholder_answers_disabled() {
        let f = fx();
        assert_eq!(
            f.svc
                .handle(named("setPasskeyProxyEnabled", Some(&f.secret))),
            Response::PasskeyProxyEnabled { enabled: false }
        );
    }

    #[test]
    fn passkey_placeholder_cases_return_not_supported_error() {
        let f = fx();
        for name in [
            "passkeyCreate",
            "passkeyCreateConfirm",
            "passkeyGet",
            "passkeyGetApprove",
        ] {
            let r = f.svc.handle(named(name, Some(&f.secret)));
            assert_eq!(
                error_message(&r),
                "passkeys are not supported by this version of MyPass yet"
            );
        }
    }

    #[test]
    fn passkey_cancel_placeholder_answers_cancelled_with_same_id() {
        let f = fx();
        assert_eq!(
            f.svc.handle(named("passkeyCancel", Some(&f.secret))),
            Response::PasskeyCancelled { request_id: -9 }
        );
    }

    #[test]
    fn passkey_confirm_never_resets_timer_when_error() {
        let f = fx();
        let confirm = named("passkeyCreateConfirm", Some(&f.secret));
        assert!(!resets_timer(&f, || {
            assert!(matches!(f.svc.handle(confirm), Response::Error { .. }));
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
    fn dispatch_match_has_no_wildcard_arm() {
        let src = include_str!("service.rs");
        let body = src.split("fn dispatch(").nth(1).unwrap();
        let body = body.split("#[cfg(test)]").next().unwrap();
        assert!(!body.contains("_ =>"));
        assert!(src.contains("#![deny(clippy::wildcard_enum_match_arm)]"));
    }
}
