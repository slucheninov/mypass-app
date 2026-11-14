//! Test fixtures shared by the handler tests: one vault session behind a real `BridgeService`, a
//! paired browser, a manual clock and the recorded UI and passkey calls.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test-only support code

use crate::diagnostics::DiagnosticsStore;
use crate::pairing::PairingManager;
use crate::pairing::{PairedBrowserStore, PairingChallenge, StartOutcome};
use crate::passkeys::{NotSupportedPasskeys, PasskeyContext, PasskeyHandler, PasskeyService};
use crate::protocol::{
    BrowserDiagnosticsReport, ExtensionScanState, PermissionState, ProxyState, Request, Response,
};
use crate::service::{BridgeDeps, BridgeService};
use crate::ui::{BridgeUi, ProposalStore};
use mypass_kdbx::{Entry, PlainFileIo, Timestamp};
use mypass_platform::testing::InMemorySecretStore;
use mypass_session::autolock::AutoLock;
use mypass_session::scheduler::ManualScheduler;
use mypass_session::session::{SessionDeps, VaultSession};
use mypass_session::settings::{IdleTimeout, SettingsPatch, SettingsStore};
use mypass_session::testing::fast_database;
use secrecy::SecretString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use uuid::Uuid;

/// Records every call.
#[derive(Default)]
pub(crate) struct RecordingUi {
    calls: Mutex<Vec<String>>,
    details: Mutex<Vec<String>>,
}

impl RecordingUi {
    fn note(&self, s: &str) {
        self.calls.lock().unwrap().push(s.to_owned());
        self.details.lock().unwrap().push(s.to_owned());
    }
    /// A call with its arguments, space separated, as `details()` shows it.
    fn note_with(&self, name: &str, args: &str) {
        self.calls.lock().unwrap().push(name.to_owned());
        self.details.lock().unwrap().push(format!("{name} {args}"));
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    /// Every call as `name args...` (the pairing code and proposal ids included).
    pub(crate) fn details(&self) -> Vec<String> {
        self.details.lock().unwrap().clone()
    }
}

impl BridgeUi for RecordingUi {
    fn pairing_code(&self, c: &PairingChallenge) {
        self.note_with("pairing_code", &c.code);
    }
    fn pairing_finished(&self) {
        self.note("pairing_finished");
    }
    fn save_login_proposal(&self, id: &str, url: &str, username: &str) {
        self.note_with("save_login_proposal", &format!("{id} {url} {username}"));
    }
    fn save_totp_proposal(&self, id: &str, url: &str, entry_id: Option<Uuid>) {
        self.note_with("save_totp_proposal", &format!("{id} {url} {entry_id:?}"));
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
    fn passkey_proxy_changed(&self, enabled: bool) {
        self.note_with("passkey_proxy_changed", &enabled.to_string());
    }
}

#[derive(Default)]
pub(crate) struct RecordingPasskeys {
    pub(crate) handled: AtomicUsize,
    pub(crate) locked: AtomicUsize,
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

pub(crate) struct Fx {
    pub(crate) svc: Arc<BridgeService>,
    pub(crate) session: VaultSession,
    pub(crate) sched: Arc<ManualScheduler>,
    pub(crate) ui: Arc<RecordingUi>,
    pub(crate) proposals: Arc<ProposalStore>,
    pub(crate) settings: SettingsStore,
    pub(crate) secret: String,
    pub(crate) _dir: tempfile::TempDir,
}

pub(crate) fn fx_with(passkeys: Arc<dyn PasskeyHandler>) -> Fx {
    let mut db = fast_database("Bridge Test");
    let now = Timestamp::now();
    let mut e = Entry::new(now);
    e.set_field("Title", "SecretTitleXyz", false);
    let root = db.root_id();
    db.add_entry(root, e, now).unwrap();
    fx_with_db(passkeys, db)
}

pub(crate) fn fx_with_db(passkeys: Arc<dyn PasskeyHandler>, db: mypass_kdbx::Database) -> Fx {
    fx_build(db, |_, _| passkeys)
}

/// An `Fx` whose passkey handler is a real [`PasskeyService`] on the fixture's clock and UI.
pub(crate) fn passkey_fx() -> (Fx, Arc<PasskeyService>) {
    let mut db = fast_database("Passkey Test");
    let now = Timestamp::now();
    let mut e = Entry::new(now);
    e.set_field("Title", "SecretTitleXyz", false);
    let root = db.root_id();
    db.add_entry(root, e, now).unwrap();
    let mut service = None;
    let f = fx_build(db, |sched, ui| {
        let s = PasskeyService::new(sched, ui);
        service = Some(s.clone());
        s
    });
    (f, service.unwrap())
}

/// Builds the fixture; `make_passkeys` gets the fixture's scheduler and UI.
fn fx_build(
    db: mypass_kdbx::Database,
    make_passkeys: impl FnOnce(Arc<ManualScheduler>, Arc<RecordingUi>) -> Arc<dyn PasskeyHandler>,
) -> Fx {
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
    let passkeys = make_passkeys(sched.clone(), ui.clone());
    let proposals = Arc::new(ProposalStore::new(sched.clone()));
    let svc = BridgeService::new(BridgeDeps {
        session: session.clone(),
        autolock,
        settings: settings.clone(),
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
        settings,
        secret,
        _dir: dir,
    }
}

pub(crate) fn fx() -> Fx {
    fx_with(Arc::new(NotSupportedPasskeys))
}

pub(crate) fn report() -> BrowserDiagnosticsReport {
    BrowserDiagnosticsReport {
        proxy_state: ProxyState::Disabled,
        permission_state: PermissionState::NotGranted,
        password_saving_enabled: None,
        extension_scan_state: ExtensionScanState::NotChecked,
        enabled_managers: vec![],
    }
}

/// One of every request, in `REQUEST_TYPES` order.
pub(crate) fn all_requests(pairing: Option<&str>) -> Vec<Request> {
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

pub(crate) fn named(name: &str, pairing: Option<&str>) -> Request {
    all_requests(pairing)
        .into_iter()
        .find(|r| r.name() == name)
        .unwrap()
}

pub(crate) fn error_message(r: &Response) -> &str {
    let Response::Error { message } = r else {
        panic!("expected Error, got {r:?}");
    };
    message
}

/// Whether `act` reset the idle timer: 50 s idle, act, 50 s more; a one minute timeout still
/// has the session unlocked only when the timer was reset in between.
pub(crate) fn resets_timer(f: &Fx, act: impl FnOnce()) -> bool {
    f.sched.advance(Duration::from_secs(50));
    act();
    f.sched.advance(Duration::from_secs(50));
    f.session.is_unlocked()
}

/// The ids of the entries of [`sample_database`].
#[allow(dead_code)] // `e` is not read by any test yet; it documents the fixture
pub(crate) struct Ids {
    /// GitHub login: favorite, TOTP, passkey, public `Phone`, protected `Recovery`, extra URL.
    pub a: Uuid,
    /// Favorite login `alpha` (lowercase title: a case-sensitive sort would put it last), no TOTP.
    pub b: Uuid,
    /// Address `Home Address`.
    pub c: Uuid,
    /// Card.
    pub d: Uuid,
    /// Notes `Journal`, with a body no search may match.
    pub e: Uuid,
    /// Files `Documents`.
    pub f: Uuid,
    /// A GitHub login in the recycle bin.
    pub g: Uuid,
    /// A GitHub login in the generator history group.
    pub h: Uuid,
    /// A favorite GitHub login in `never` mode.
    pub never: Uuid,
}

pub(crate) const A_PASSWORD: &str = "a-pass-w0rd!";
pub(crate) const A_OTP_SECRET: &str = "JBSWY3DPEHPK3PXP";

fn field_entry(title: &str, fields: &[(&str, &str, bool)]) -> Entry {
    let mut e = Entry::new(Timestamp::now());
    e.set_field("Title", title, false);
    for (k, v, protected) in fields {
        e.set_field(k, v, *protected);
    }
    e
}

/// The shared bridge fixture, built on `fast_database`.
pub(crate) fn sample_database() -> (mypass_kdbx::Database, Ids) {
    use mypass_kdbx::Group;
    let now = Timestamp::now();
    let mut db = fast_database("Bridge Sample");
    let root = db.root_id();
    let mut history = Group::new("Password Generator History", now);
    history.uuid = mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID;
    db.add_group(root, history, now).unwrap();

    let mut a = field_entry(
        "GitHub",
        &[
            ("UserName", "alice", false),
            ("Password", A_PASSWORD, true),
            ("URL", "https://github.com", false),
            ("Favorite", "1", false),
            ("otp", A_OTP_SECRET, true),
            ("Phone", " 555-0100 ", false),
            ("Recovery", "recovery-code-xyz", true),
            ("KPEX_PASSKEY_CREDENTIAL_ID", "cred", true),
            ("KPEX_PASSKEY_PRIVATE_KEY_PEM", "pem", true),
        ],
    );
    a.string_arrays.push(mypass_kdbx::StringArray {
        key: "KP2A_URL".into(),
        values: vec!["https://gist.github.com".into()],
    });
    let b = field_entry(
        "alpha",
        &[
            ("Favorite", "1", false),
            ("UserName", "bob", false),
            ("Password", "b-secret", true),
            ("URL", "https://example.org", false),
        ],
    );
    let c = field_entry(
        "Home Address",
        &[
            ("MyPass.EntryType", "Address", false),
            ("MyPass.Address.label", "Home", true),
            ("MyPass.Address.fullName", "Alice Example", true),
            ("MyPass.Address.city", "Kyiv", true),
            ("URL", "https://github.com", false),
        ],
    );
    let d = field_entry(
        "Visa Card",
        &[
            ("MyPass.EntryType", "Card", false),
            ("MyPass.CardNumber", "4111111111111111", true),
        ],
    );
    let e = field_entry(
        "Journal",
        &[
            ("MyPass.EntryType", "Notes", false),
            ("Notes", "zebra-in-notes", false),
        ],
    );
    let f = field_entry(
        "Documents",
        &[
            ("MyPass.EntryType", "Files", false),
            ("Favorite", "1", false),
            ("URL", "https://github.com", false),
        ],
    );
    let g = field_entry(
        "Binned GitHub",
        &[
            ("Favorite", "1", false),
            ("UserName", "binned", false),
            ("Password", "bin-pass", true),
            ("URL", "https://github.com", false),
        ],
    );
    let h = field_entry(
        "Generated GitHub",
        &[
            ("Favorite", "1", false),
            ("Password", "gen-pass", true),
            ("URL", "https://github.com", false),
        ],
    );
    let never = field_entry(
        "GitHub Never",
        &[
            ("UserName", "nobody", false),
            ("Password", "never-pass", true),
            ("URL", "https://github.com", false),
            ("Favorite", "1", false),
            ("MyPass.URIMatchMode", "never", false),
        ],
    );
    let ids = Ids {
        a: a.uuid,
        b: b.uuid,
        c: c.uuid,
        d: d.uuid,
        e: e.uuid,
        f: f.uuid,
        g: g.uuid,
        h: h.uuid,
        never: never.uuid,
    };
    for entry in [a, b, c, d, e, f, g, never] {
        db.add_entry(root, entry, now).unwrap();
    }
    db.add_entry(mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID, h, now)
        .unwrap();
    db.delete_entry(ids.g, now).unwrap();
    (db, ids)
}

/// An `Fx` over [`sample_database`].
pub(crate) fn sample_fx() -> (Fx, Ids) {
    let (db, ids) = sample_database();
    (fx_with_db(Arc::new(NotSupportedPasskeys), db), ids)
}

/// Dispatches a request built by `make` with the fixture's pairing secret.
pub(crate) fn send(f: &Fx, make: impl FnOnce(String, Option<String>) -> Request) -> Response {
    f.svc.handle(make("tok".to_owned(), Some(f.secret.clone())))
}

pub(crate) fn credential_rows(r: &Response) -> &[crate::protocol::ExtensionCredentialSummary] {
    let Response::CredentialList { credentials } = r else {
        panic!("expected CredentialList, got {r:?}");
    };
    credentials
}
