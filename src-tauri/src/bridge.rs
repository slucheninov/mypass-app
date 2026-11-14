//! The browser bridge inside the app shell: starting it, what it asks of the windows and the
//! webview, the pairing window rules, and the quit rule.
//!
//! Decisions are plain functions ([`unlock_focus_due`], [`pairing_changed`], [`extension_folder`],
//! [`on_quit_outcome`]); the only Tauri type is [`TauriBridgeEvents`], the thin production
//! implementation of the [`BridgeEvents`] seam, so every rule runs in tests with recording fakes.

use crate::commands::browser::PairingChallengeDto;
use crate::events::{
    BridgeChangedEvent, PairingChallengeEvent, ProposalEvent, ProposalKind, SettingsChanged,
    UnlockRequestedEvent,
};
use crate::lifecycle::ExitOutcome;
use crate::shell::WindowOps;
use mypass_bridge::diagnostics::DiagnosticsStore;
use mypass_bridge::endpoint::EndpointStore;
use mypass_bridge::pairing::{
    PairedBrowserStore, PairingChallenge, PairingError, PairingManager, PAIRING_CODE_TTL,
};
use mypass_bridge::passkeys::PasskeyService;
use mypass_bridge::server::{LoopbackServer, ServerConfig, ServerError};
use mypass_bridge::service::{BridgeDeps, BridgeService};
use mypass_bridge::ui::{BridgeUi, ProposalStore};
use mypass_platform::{HostRegistrationReport, NativeHostRegistrar, SecretStore};
use mypass_session::session::SessionEvent;
use mypass_session::{AppCore, SessionState, Settings};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::AppHandle;
use tauri_specta::Event;
use uuid::Uuid;

/// The main window's label.
const MAIN: &str = crate::commands::app::MAIN_WINDOW;

/// A browser that keeps asking to be unlocked (the extension polls every 2 s) raises the window
/// at most this often; the request is still noted and announced every time.
pub const UNLOCK_FOCUS_COOLDOWN: Duration = Duration::from_secs(30);

/// Whether an unlock request may show and focus the main window now: the first one, and then one
/// per [`UNLOCK_FOCUS_COOLDOWN`] (monotonic times).
pub fn unlock_focus_due(last: Option<Duration>, now: Duration) -> bool {
    last.is_none_or(|t| now.saturating_sub(t) >= UNLOCK_FOCUS_COOLDOWN)
}

/// Remembers when the window was last raised for an unlock request.
#[derive(Debug, Default)]
pub struct FocusThrottle(Mutex<Option<Duration>>);

impl FocusThrottle {
    /// True (and remembered) when [`unlock_focus_due`].
    pub fn take(&self, now: Duration) -> bool {
        let mut last = lock(&self.0);
        let due = unlock_focus_due(*last, now);
        if due {
            *last = Some(now);
        }
        due
    }

    /// An unlock happened: the next request is a new one.
    pub fn reset(&self) {
        *lock(&self.0) = None;
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the bridge tells the webview, and the one window it can open. A seam so tests record it.
pub trait BridgeEvents: Send + Sync {
    /// The pairing challenge (`None`: gone). Main window only.
    fn pairing_challenge(&self, challenge: Option<PairingChallengeDto>);
    /// Paired browsers, diagnostics or registration changed. May broadcast.
    fn bridge_changed(&self);
    /// A save proposal arrived (no secret in it). Main window only.
    fn proposal(&self, event: ProposalEvent);
    /// A browser asked for an unlock.
    fn unlock_requested(&self);
    /// Opens the settings window, at `pane` when given.
    fn open_settings(&self, pane: Option<&str>);
    /// The settings changed outside the settings commands (the extension's passkey toggle).
    /// Broadcast to every window, as the `settings_update` command does.
    fn settings_changed(&self, settings: Settings);
}

/// [`BridgeEvents`] over the real app.
pub struct TauriBridgeEvents(pub AppHandle);

impl BridgeEvents for TauriBridgeEvents {
    fn pairing_challenge(&self, challenge: Option<PairingChallengeDto>) {
        // A window that is gone is not an error of the bridge.
        let _ = PairingChallengeEvent(challenge).emit_to(&self.0, MAIN);
    }
    fn bridge_changed(&self) {
        let _ = BridgeChangedEvent.emit(&self.0);
    }
    fn proposal(&self, event: ProposalEvent) {
        let _ = event.emit_to(&self.0, MAIN);
    }
    fn unlock_requested(&self) {
        let _ = UnlockRequestedEvent.emit(&self.0);
    }
    fn open_settings(&self, pane: Option<&str>) {
        let _ = crate::commands::app::open_settings(&self.0, pane);
    }
    fn settings_changed(&self, settings: Settings) {
        let _ = SettingsChanged(settings).emit(&self.0);
    }
}

/// The main window while a pairing code is shown: raised and always on top, and lowered again
/// through the ONE [`PairingWindow::lower`] on every exit (finished, cancelled, locked, expired),
/// so no window stays stuck on top. Lowering is a no-op unless it was raised.
pub struct PairingWindow {
    windows: Arc<dyn WindowOps>,
    raised: AtomicBool,
}

impl PairingWindow {
    pub fn new(windows: Arc<dyn WindowOps>) -> Self {
        Self {
            windows,
            raised: AtomicBool::new(false),
        }
    }

    pub fn raise(&self) {
        let _ = self.windows.show(MAIN);
        let _ = self.windows.focus(MAIN);
        let _ = self.windows.set_always_on_top(MAIN, true);
        self.raised.store(true, Ordering::SeqCst);
    }

    pub fn lower(&self) {
        if self.raised.swap(false, Ordering::SeqCst) {
            let _ = self.windows.set_always_on_top(MAIN, false);
        }
    }
}

/// The pairing manager's change hook: tells the webview the current challenge, lowers the window
/// once there is none, and announces the change.
pub fn pairing_changed(
    pairing: &PairingManager,
    events: &dyn BridgeEvents,
    window: &PairingWindow,
) {
    let active = pairing.active();
    events.pairing_challenge(active.as_ref().map(Into::into));
    if active.is_none() {
        window.lower();
    }
    events.bridge_changed();
}

/// What the bridge asks of the app's UI.
pub struct TauriBridgeUi {
    core: Arc<AppCore>,
    events: Arc<dyn BridgeEvents>,
    windows: Arc<dyn WindowOps>,
    pairing: Arc<PairingManager>,
    pairing_window: Arc<PairingWindow>,
    focus: Arc<FocusThrottle>,
}

impl TauriBridgeUi {
    fn show_main(&self) {
        let _ = crate::shell::show_main(self.windows.as_ref());
    }
}

impl BridgeUi for TauriBridgeUi {
    fn pairing_code(&self, _c: &PairingChallenge) {
        // The challenge itself reached the webview through the change hook.
        self.pairing_window.raise();
        // A lock between the code's creation and this raise already ran its observer: do not leave
        // the window pinned on top with no code to show.
        if self.core.session.status().state != SessionState::Unlocked {
            self.pairing.cancel();
        }
        // Nothing notifies when a code simply runs out, so look again right after its deadline.
        let pairing = Arc::downgrade(&self.pairing);
        let events = Arc::clone(&self.events);
        let window = Arc::clone(&self.pairing_window);
        self.core.scheduler().schedule(
            PAIRING_CODE_TTL.saturating_add(Duration::from_secs(1)),
            Box::new(move || {
                if let Some(pairing) = pairing.upgrade() {
                    pairing_changed(&pairing, events.as_ref(), &window);
                }
            }),
        );
    }

    fn pairing_finished(&self) {
        self.pairing_window.lower();
    }

    fn save_login_proposal(&self, id: &str, url: &str, username: &str) {
        self.events.proposal(ProposalEvent {
            id: id.to_owned(),
            kind: ProposalKind::Login,
            url: url.to_owned(),
            username: Some(username.to_owned()),
            entry_id: None,
        });
    }

    fn save_totp_proposal(&self, id: &str, url: &str, entry_id: Option<Uuid>) {
        self.events.proposal(ProposalEvent {
            id: id.to_owned(),
            kind: ProposalKind::Totp,
            url: url.to_owned(),
            username: None,
            entry_id: entry_id.map(mypass_bridge::views::wire_id),
        });
    }

    fn unlock_requested(&self) {
        self.core.unlock_requests.note();
        self.events.unlock_requested();
        if self.focus.take(self.core.scheduler().monotonic_now()) {
            self.show_main();
        }
    }

    fn open_app_requested(&self) {
        self.show_main();
    }

    fn open_browser_settings_requested(&self) {
        self.events.open_settings(Some("browser"));
    }

    fn credential_filled(&self, _id: Uuid) {
        // The fill handler already marked the entry accessed (recents, usage count); touching it
        // here too counted every fill twice. The hook stays for a future UI refresh.
    }

    fn passkey_proxy_changed(&self, _enabled: bool) {
        // The whole stored snapshot, not just the flag: the settings store of every window
        // replaces its copy with what `SettingsChanged` carries (one source of truth, no event
        // of its own for this one field).
        self.events.settings_changed(self.core.settings.get());
    }
}

/// Why the bridge did not start. The messages are for the developer; the webview only learns
/// `listening: false` and `pairingStoreFailed`.
#[derive(Debug, thiserror::Error)]
pub enum BridgeStartError {
    #[error("the paired browser list is unreadable: {0}")]
    PairingStore(PairingError),
    #[error("the loopback server did not start: {0}")]
    Server(ServerError),
    #[error("a worker thread could not be started: {0}")]
    Thread(String),
}

/// A running bridge.
pub struct BridgeHandle {
    pub server: LoopbackServer,
    pub service: Arc<BridgeService>,
    pub pairing: Arc<PairingManager>,
    pub diagnostics: Arc<DiagnosticsStore>,
    pub proposals: Arc<ProposalStore>,
    /// The last host registration report (`None` until the startup registration ran, or when it
    /// failed).
    pub registration: Arc<Mutex<Option<HostRegistrationReport>>>,
    registration_thread: Mutex<Option<JoinHandle<()>>>,
}

impl BridgeHandle {
    /// Waits for the startup registration to finish (tests, and nothing else).
    pub fn join_registration(&self) {
        let thread = lock(&self.registration_thread).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

/// Registers the host with the browsers. A failure is reported as `None`, never fatal.
pub fn register_hosts(
    registrar: &dyn NativeHostRegistrar,
    bundled_host: Option<&Path>,
) -> Option<HostRegistrationReport> {
    registrar.register(bundled_host?).ok()
}

/// Starts the bridge: pairing over the app-only secrets, the endpoint over the host-shared ones,
/// the loopback server, and the host registration on a worker thread. A locked vault is fine: the
/// service answers `locked`.
pub fn start_bridge(
    core: &Arc<AppCore>,
    windows: Arc<dyn WindowOps>,
    events: Arc<dyn BridgeEvents>,
    bundled_host: Option<PathBuf>,
) -> Result<BridgeHandle, BridgeStartError> {
    let platform = core.platform();
    let scheduler = core.scheduler();
    let pairing = Arc::new(
        PairingManager::new(
            PairedBrowserStore::new(Arc::clone(&platform.secrets)),
            Arc::clone(&scheduler),
        )
        .map_err(BridgeStartError::PairingStore)?,
    );
    let diagnostics = Arc::new(DiagnosticsStore::new());
    let proposals = Arc::new(ProposalStore::new(Arc::clone(&scheduler)));
    let pairing_window = Arc::new(PairingWindow::new(Arc::clone(&windows)));
    let focus = Arc::new(FocusThrottle::default());

    {
        let weak = Arc::downgrade(&pairing);
        let events = Arc::clone(&events);
        let window = Arc::clone(&pairing_window);
        pairing.set_on_change(Arc::new(move || {
            if let Some(pairing) = weak.upgrade() {
                pairing_changed(&pairing, events.as_ref(), &window);
            }
        }));
    }
    let observer_window = Arc::clone(&pairing_window);
    let observer_focus = Arc::clone(&focus);
    let ui: Arc<dyn BridgeUi> = Arc::new(TauriBridgeUi {
        core: Arc::clone(core),
        events: Arc::clone(&events),
        windows,
        pairing: Arc::clone(&pairing),
        pairing_window,
        focus,
    });
    let service = BridgeService::new_unobserved(BridgeDeps {
        session: core.session.clone(),
        autolock: core.autolock.clone(),
        settings: core.settings.clone(),
        scheduler: Arc::clone(&scheduler),
        pairing: Arc::clone(&pairing),
        diagnostics: Arc::clone(&diagnostics),
        proposals: Arc::clone(&proposals),
        ui: Arc::clone(&ui),
        // The pending table is dropped on lock and on a vault switch by the observer that
        // `register_observer` adds below, once the server is up.
        passkeys: PasskeyService::new(Arc::clone(&scheduler), ui),
    });
    let handler_service = Arc::clone(&service);
    let server = LoopbackServer::start(
        ServerConfig::default(),
        Arc::new(EndpointStore::new(Arc::clone(&platform.shared_secrets))),
        Arc::new(move |request| handler_service.handle(request)),
    )
    .map_err(BridgeStartError::Server)?;

    let registration = Arc::new(Mutex::new(None));
    let thread = {
        let registrar = Arc::clone(&platform.host_registrar);
        let slot = Arc::clone(&registration);
        std::thread::Builder::new()
            .name("mypass-host-register".into())
            .spawn(move || {
                *lock(&slot) = register_hosts(registrar.as_ref(), bundled_host.as_deref());
                events.bridge_changed();
            })
            .map_err(|e| {
                server.shutdown();
                BridgeStartError::Thread(e.to_string())
            })?
    };
    service.register_observer();
    // Registered only now that the server is up: a failed start leaves no observer behind (a
    // retry after `browser_reset_pairings` builds a fresh one, and the old one would leak).
    {
        let weak = Arc::downgrade(&pairing);
        let window = observer_window;
        let focus = observer_focus;
        core.session
            .add_observer(Arc::new(move |event: &SessionEvent| {
                let SessionEvent::StateChanged(status) = event else {
                    return;
                };
                if status.state == SessionState::Unlocked {
                    focus.reset();
                    return;
                }
                // A lock ends the pairing (the code cannot be shown) and the unlock throttle.
                if let Some(pairing) = weak.upgrade() {
                    pairing.cancel();
                }
                window.lower();
            }));
    }
    Ok(BridgeHandle {
        server,
        service,
        pairing,
        diagnostics,
        proposals,
        registration,
        registration_thread: Mutex::new(Some(thread)),
    })
}

type Starter = dyn Fn() -> Result<BridgeHandle, BridgeStartError> + Send + Sync;

struct StateInner {
    slot: Mutex<Option<Arc<BridgeHandle>>>,
    store_failed: AtomicBool,
    /// Set by the final quit: nothing starts after it, so a start that was still queued cannot
    /// publish an endpoint record the exiting process never clears.
    quit: AtomicBool,
    starter: Box<Starter>,
    secrets: Arc<dyn SecretStore>,
    bundled_host: Option<PathBuf>,
    events: Arc<dyn BridgeEvents>,
}

/// The managed state of the bridge: the running handle (when it started) and how to start it
/// again after the pairing list was reset.
#[derive(Clone)]
pub struct BridgeState(Arc<StateInner>);

impl BridgeState {
    pub fn new(
        starter: Box<Starter>,
        secrets: Arc<dyn SecretStore>,
        bundled_host: Option<PathBuf>,
        events: Arc<dyn BridgeEvents>,
    ) -> Self {
        Self(Arc::new(StateInner {
            slot: Mutex::new(None),
            store_failed: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            starter,
            secrets,
            bundled_host,
            events,
        }))
    }

    /// Starts the bridge unless it runs already.
    pub fn start(&self) -> Result<(), BridgeStartError> {
        let mut slot = lock(&self.0.slot);
        self.start_locked(&mut slot)
    }

    fn start_locked(&self, slot: &mut Option<Arc<BridgeHandle>>) -> Result<(), BridgeStartError> {
        if slot.is_some() || self.0.quit.load(Ordering::SeqCst) {
            return Ok(());
        }
        match (self.0.starter)() {
            Ok(handle) => {
                self.0.store_failed.store(false, Ordering::SeqCst);
                *slot = Some(Arc::new(handle));
                Ok(())
            }
            Err(e) => {
                let failed = matches!(e, BridgeStartError::PairingStore(_));
                self.0.store_failed.store(failed, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    pub fn handle(&self) -> Option<Arc<BridgeHandle>> {
        lock(&self.0.slot).clone()
    }

    pub fn pairing_store_failed(&self) -> bool {
        self.0.store_failed.load(Ordering::SeqCst)
    }

    pub fn bundled_host(&self) -> Option<PathBuf> {
        self.0.bundled_host.clone()
    }

    /// Keeps the report of a registration the user asked for and tells the webview.
    pub fn record_registration(&self, report: HostRegistrationReport) {
        if let Some(handle) = self.handle() {
            *lock(&handle.registration) = Some(report);
        }
        self.0.events.bridge_changed();
    }

    /// Forgets every paired browser. Works on an unreadable list too (the F5 case): the empty
    /// list is written over it and the bridge starts. A start failure after the reset is not a
    /// failure of the reset.
    pub fn reset_pairings(&self) -> Result<(), PairingError> {
        let mut slot = lock(&self.0.slot);
        if let Some(handle) = slot.as_ref() {
            for browser in handle.pairing.list() {
                handle.pairing.revoke(browser.id)?;
                handle.diagnostics.remove(browser.id);
            }
            return Ok(());
        }
        PairedBrowserStore::new(Arc::clone(&self.0.secrets)).save(&[])?;
        let _ = self.start_locked(&mut slot);
        drop(slot);
        self.0.events.bridge_changed();
        Ok(())
    }

    /// Stops the server (idempotent) and refuses every later start. Taking the slot lock first
    /// waits for a start that is in flight, so its endpoint record is cleared too.
    pub fn shutdown(&self) {
        let handle = {
            let slot = lock(&self.0.slot);
            self.0.quit.store(true, Ordering::SeqCst);
            slot.clone()
        };
        if let Some(handle) = handle {
            handle.server.shutdown();
        }
    }
}

/// The quit rule: the server stops only once the quit decision is final. A cancelled quit
/// (`Stay`) leaves it running and serving.
pub fn on_quit_outcome(state: &BridgeState, outcome: ExitOutcome) {
    if outcome == ExitOutcome::Exit {
        state.shutdown();
    }
}

/// The folder with the unpacked extension: the first that exists of `<resource_dir>/extension`
/// (macOS bundle Resources), `<exe_dir>/extension` (Windows zip, beside `MyPass.exe`) and
/// `<dev_repo_root>/extension` (dev builds).
pub fn extension_folder(
    resource_dir: Option<&Path>,
    exe_dir: Option<&Path>,
    dev_repo_root: Option<&Path>,
) -> Option<PathBuf> {
    [resource_dir, exe_dir, dev_repo_root]
        .into_iter()
        .flatten()
        .map(|dir| dir.join("extension"))
        .find(|candidate| candidate.is_dir())
}

/// The production [`BridgeState`]: starts over `core` with the real windows and events.
pub fn build_state(
    core: &Arc<AppCore>,
    windows: Arc<dyn WindowOps>,
    events: Arc<dyn BridgeEvents>,
) -> BridgeState {
    let bundled_host = std::env::current_exe()
        .ok()
        .and_then(|exe| mypass_platform::locate_bundled_host(&exe));
    let starter = {
        let core = Arc::clone(core);
        let events = Arc::clone(&events);
        let host = bundled_host.clone();
        Box::new(move || {
            start_bridge(
                &core,
                Arc::clone(&windows),
                Arc::clone(&events),
                host.clone(),
            )
        })
    };
    BridgeState::new(
        starter,
        Arc::clone(&core.platform().secrets),
        bundled_host,
        events,
    )
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Test-only support code: a bridge over the fake platform with recording events and windows.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::shell::FakeWindowOps;
    use crate::testing::{shared_world, SharedWorld};

    #[derive(Default)]
    pub struct RecordingEvents {
        pub log: Mutex<Vec<String>>,
        pub challenges: Mutex<Vec<Option<PairingChallengeDto>>>,
        pub proposals: Mutex<Vec<ProposalEvent>>,
        pub panes: Mutex<Vec<Option<String>>>,
        pub settings: Mutex<Vec<Settings>>,
    }

    impl RecordingEvents {
        pub fn count(&self, name: &str) -> usize {
            lock(&self.log).iter().filter(|l| *l == name).count()
        }
    }

    impl BridgeEvents for RecordingEvents {
        fn pairing_challenge(&self, challenge: Option<PairingChallengeDto>) {
            lock(&self.log).push("pairing_challenge".into());
            lock(&self.challenges).push(challenge);
        }
        fn bridge_changed(&self) {
            lock(&self.log).push("bridge_changed".into());
        }
        fn proposal(&self, event: ProposalEvent) {
            lock(&self.log).push("proposal".into());
            lock(&self.proposals).push(event);
        }
        fn unlock_requested(&self) {
            lock(&self.log).push("unlock_requested".into());
        }
        fn open_settings(&self, pane: Option<&str>) {
            lock(&self.log).push("open_settings".into());
            lock(&self.panes).push(pane.map(str::to_owned));
        }
        fn settings_changed(&self, settings: Settings) {
            lock(&self.log).push("settings_changed".into());
            lock(&self.settings).push(settings);
        }
    }

    pub struct Fixture {
        pub w: SharedWorld,
        pub events: Arc<RecordingEvents>,
        pub windows: Arc<FakeWindowOps>,
        pub state: BridgeState,
    }

    impl Fixture {
        /// The running bridge (the fixture starts it, so a failure to start panics the test).
        pub fn handle(&self) -> Arc<BridgeHandle> {
            self.state.handle().unwrap()
        }

        /// The `set_always_on_top` flags of the main window, in order.
        pub fn on_top(&self) -> Vec<bool> {
            self.windows
                .calls()
                .into_iter()
                .filter(|(op, label, _)| op == "set_always_on_top" && label == "main")
                .filter_map(|(_, _, flag)| flag)
                .collect()
        }
    }

    /// A fixture whose bridge is NOT started yet.
    pub fn unstarted(open: bool, bundled_host: Option<PathBuf>) -> Fixture {
        let w = shared_world(open);
        let events = Arc::new(RecordingEvents::default());
        let windows = Arc::new(FakeWindowOps::default());
        let starter = {
            let core = Arc::clone(&w.core);
            let windows: Arc<dyn WindowOps> = windows.clone();
            let events: Arc<dyn BridgeEvents> = events.clone();
            let host = bundled_host.clone();
            Box::new(move || {
                start_bridge(
                    &core,
                    Arc::clone(&windows),
                    Arc::clone(&events),
                    host.clone(),
                )
            })
        };
        let state = BridgeState::new(
            starter,
            Arc::clone(&w.core.platform().secrets),
            bundled_host,
            events.clone(),
        );
        Fixture {
            w,
            events,
            windows,
            state,
        }
    }

    pub fn fixture(open: bool) -> Fixture {
        let f = unstarted(open, None);
        f.state.start().unwrap();
        f
    }

    /// Pairs a browser directly through the manager and returns its secret.
    pub fn pair(f: &Fixture) -> String {
        let handle = f.handle();
        // Past the throttle between two starts.
        f.w.clock
            .advance(mypass_bridge::pairing::START_MIN_INTERVAL);
        handle.pairing.start(Some("Chrome"));
        let code = handle.pairing.active().unwrap().code;
        handle.pairing.complete(&code).unwrap().unwrap()
    }

    /// One framed request over a real loopback connection, after the hello exchange.
    pub fn ask(handle: &BridgeHandle, request: &serde_json::Value) -> serde_json::Value {
        use mypass_bridge::codec::{encode_frame, read_frame};
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect(handle.server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let hello = serde_json::json!({"type": "hello", "nonce": "AAAAAAAAAAAAAAAAAAAAAA=="});
        stream
            .write_all(&encode_frame(hello.to_string().as_bytes()).unwrap())
            .unwrap();
        let ack = read_frame(&mut stream, 1 << 20).unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&ack).unwrap()["type"],
            "helloAck"
        );
        let mut request = request.clone();
        request["token"] = handle.server.endpoint().token.clone().into();
        stream
            .write_all(&encode_frame(request.to_string().as_bytes()).unwrap())
            .unwrap();
        let reply = read_frame(&mut stream, 1 << 20).unwrap().unwrap();
        serde_json::from_slice(&reply).unwrap()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::test_support::{ask, fixture, pair, unstarted};
    use super::*;
    use mypass_bridge::endpoint::{ENDPOINT_ACCOUNT, ENDPOINT_SERVICE};
    use mypass_bridge::pairing::{PAIRING_ACCOUNT, PAIRING_SERVICE};
    use mypass_bridge::protocol::Request;
    use mypass_platform::PlatformError;

    fn start_pairing(f: &super::test_support::Fixture) {
        let r = f.handle().service.handle(Request::StartPairing {
            token: "t".into(),
            browser: Some("Chrome".into()),
        });
        assert_eq!(r, mypass_bridge::protocol::Response::PairingStarted);
    }

    #[test]
    fn bridge_wiring_gives_pairing_store_the_app_only_secrets() {
        let f = fixture(true);
        let handle = f.handle();
        handle.pairing.start(Some("Chrome"));
        let code = handle.pairing.active().unwrap().code;
        assert!(handle.pairing.complete(&code).unwrap().is_some());
        let app_only = &f.w.fake.secrets;
        let shared = &f.w.fake.shared_secrets;
        assert!(app_only
            .get(PAIRING_SERVICE, PAIRING_ACCOUNT)
            .unwrap()
            .is_some());
        assert!(shared
            .get(PAIRING_SERVICE, PAIRING_ACCOUNT)
            .unwrap()
            .is_none());
        assert!(shared
            .get(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT)
            .unwrap()
            .is_some());
        assert!(app_only
            .get(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT)
            .unwrap()
            .is_none());
    }

    #[test]
    fn bridge_starts_while_locked_and_answers_locked() {
        let f = unstarted(true, None);
        f.w.core.vault_lock().unwrap();
        f.state.start().unwrap();
        let reply = ask(
            &f.handle(),
            &serde_json::json!({"type": "getCredentials", "url": "https://a.example"}),
        );
        assert_eq!(reply["type"], "locked");
    }

    #[test]
    fn host_registration_failure_does_not_stop_the_server() {
        let f = unstarted(true, Some(PathBuf::from("/nonexistent/mypass-host")));
        f.w.fake
            .host_registrar
            .fail_next(PlatformError::Failed("boom".into()));
        f.state.start().unwrap();
        let handle = f.handle();
        handle.join_registration();
        assert_eq!(f.w.fake.host_registrar.register_calls(), 1);
        assert!(lock(&handle.registration).is_none());
        f.w.core.vault_lock().unwrap();
        let reply = ask(&handle, &serde_json::json!({"type": "getFavorites"}));
        assert_eq!(reply["type"], "locked");
    }

    #[test]
    fn host_registration_success_keeps_the_report() {
        let f = unstarted(true, Some(PathBuf::from("/h/mypass-host")));
        f.state.start().unwrap();
        let handle = f.handle();
        handle.join_registration();
        assert!(lock(&handle.registration).is_some());
        assert_eq!(f.events.count("bridge_changed"), 1);
    }

    #[test]
    fn unlock_requested_notes_state_emits_event_and_shows_window() {
        let f = fixture(true);
        let secret = pair(&f);
        f.w.core.vault_lock().unwrap();
        f.handle().service.handle(Request::RequestUnlock {
            token: "t".into(),
            pairing: Some(secret.clone()),
        });
        assert_eq!(f.events.count("unlock_requested"), 1);
        let ops: Vec<_> = f.windows.calls().into_iter().map(|(op, _, _)| op).collect();
        assert_eq!(ops, ["show", "focus"]);
        assert!(f.w.core.unlock_requests.consume(), "a note is pending");
    }

    #[test]
    fn unlock_focus_is_due_first_then_once_per_cooldown() {
        let s = Duration::from_secs;
        assert!(unlock_focus_due(None, s(5)));
        assert!(!unlock_focus_due(Some(s(5)), s(7)));
        assert!(!unlock_focus_due(Some(s(5)), s(34)));
        assert!(unlock_focus_due(Some(s(5)), s(35)));
        // A monotonic clock never goes back, but a saturating difference must not panic.
        assert!(!unlock_focus_due(Some(s(50)), s(10)));
    }

    #[test]
    fn a_two_second_unlock_poll_raises_the_window_once() {
        let f = fixture(true);
        let secret = pair(&f);
        f.w.core.vault_lock().unwrap();
        let handle = f.handle();
        for _ in 0..8 {
            handle.service.handle(Request::RequestUnlock {
                token: "t".into(),
                pairing: Some(secret.clone()),
            });
            f.w.clock.advance(Duration::from_secs(2));
        }
        let shows = |f: &super::test_support::Fixture| {
            f.windows
                .calls()
                .iter()
                .filter(|(op, _, _)| op == "show")
                .count()
        };
        assert_eq!(shows(&f), 1, "16 s of polling raises the window once");
        assert_eq!(
            f.events.count("unlock_requested"),
            8,
            "every request is announced"
        );
        f.w.clock.advance(UNLOCK_FOCUS_COOLDOWN);
        handle.service.handle(Request::RequestUnlock {
            token: "t".into(),
            pairing: Some(secret.clone()),
        });
        assert_eq!(shows(&f), 2);
    }

    #[test]
    fn an_unlock_resets_the_focus_throttle() {
        let f = fixture(true);
        let secret = pair(&f);
        f.w.core.vault_lock().unwrap();
        let handle = f.handle();
        let ask_unlock = || {
            handle.service.handle(Request::RequestUnlock {
                token: "t".into(),
                pairing: Some(secret.clone()),
            })
        };
        ask_unlock();
        f.w.core.vault_unlock(crate::testing::pw("pw")).unwrap();
        f.w.core.vault_lock().unwrap();
        f.w.clock.advance(Duration::from_secs(3));
        ask_unlock();
        let shows = f
            .windows
            .calls()
            .iter()
            .filter(|(op, _, _)| op == "show")
            .count();
        assert_eq!(shows, 2);
    }

    #[test]
    fn pairing_code_raises_and_pairing_finished_lowers_window() {
        let f = fixture(true);
        start_pairing(&f);
        let ops: Vec<_> = f
            .windows
            .calls()
            .into_iter()
            .map(|(op, _, flag)| (op, flag))
            .collect();
        assert_eq!(
            ops,
            [
                ("show".to_owned(), None),
                ("focus".to_owned(), None),
                ("set_always_on_top".to_owned(), Some(true)),
            ]
        );
        let code = f.handle().pairing.active().unwrap().code;
        f.handle().service.handle(Request::CompletePairing {
            token: "t".into(),
            code,
        });
        assert_eq!(f.on_top(), [true, false]);
        let challenges = lock(&f.events.challenges);
        assert!(
            challenges.first().unwrap().is_some(),
            "the code reached the webview"
        );
        assert_eq!(challenges.last().unwrap(), &None, "and its end did too");
    }

    #[test]
    fn pairing_window_lowers_on_cancel_lock_and_timeout() {
        // Cancel.
        let f = fixture(true);
        start_pairing(&f);
        f.handle().pairing.cancel();
        assert_eq!(f.on_top(), [true, false]);
        // Lock.
        let f = fixture(true);
        start_pairing(&f);
        f.w.core.vault_lock().unwrap();
        assert_eq!(f.on_top(), [true, false]);
        assert!(f.handle().pairing.active().is_none());
        // Timeout: nothing notifies, the scheduled look right after the deadline does.
        let f = fixture(true);
        start_pairing(&f);
        f.w.clock.advance(PAIRING_CODE_TTL);
        assert_eq!(
            f.on_top(),
            [true],
            "not yet: the code is valid until its deadline"
        );
        f.w.clock.advance(Duration::from_secs(2));
        assert_eq!(f.on_top(), [true, false]);
        assert_eq!(lock(&f.events.challenges).last().unwrap(), &None);
    }

    #[test]
    fn open_app_shows_main_window_without_always_on_top() {
        let f = fixture(true);
        let secret = pair(&f);
        let before = f.windows.calls().len();
        let r = f.handle().service.handle(Request::OpenApp {
            token: "t".into(),
            pairing: Some(secret),
        });
        assert_eq!(r, mypass_bridge::protocol::Response::Acknowledged);
        let calls: Vec<_> = f.windows.calls().into_iter().skip(before).collect();
        let ops: Vec<_> = calls.iter().map(|(op, _, _)| op.as_str()).collect();
        assert_eq!(ops, ["show", "focus"]);
    }

    #[test]
    fn extension_folder_prefers_resources_then_exe_dir_then_dev_repo_then_none() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            let root = dir.path().join(name);
            std::fs::create_dir_all(root.join("extension")).unwrap();
            root
        };
        let (res, exe, dev) = (make("res"), make("exe"), make("dev"));
        assert_eq!(
            extension_folder(Some(&res), Some(&exe), Some(&dev)),
            Some(res.join("extension"))
        );
        assert_eq!(
            extension_folder(Some(&dir.path().join("none")), Some(&exe), Some(&dev)),
            Some(exe.join("extension"))
        );
        assert_eq!(
            extension_folder(None, Some(&dir.path().join("none")), Some(&dev)),
            Some(dev.join("extension"))
        );
        assert_eq!(extension_folder(None, None, None), None);
        assert_eq!(
            extension_folder(Some(&dir.path().join("none")), None, None),
            None
        );
    }

    #[test]
    fn extension_folder_beside_the_exe_when_there_is_no_resources_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ext = dir.path().join("extension");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(ext.join("manifest.json"), b"{}").unwrap();
        assert_eq!(extension_folder(None, Some(dir.path()), None), Some(ext));
    }

    #[test]
    fn no_bridge_start_after_the_final_quit() {
        let f = unstarted(false, None);
        on_quit_outcome(&f.state, ExitOutcome::Exit);
        f.state.start().unwrap();
        assert!(
            f.state.handle().is_none(),
            "a start after the quit is a no-op"
        );
        assert!(f
            .w
            .fake
            .shared_secrets
            .get(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT)
            .unwrap()
            .is_none());
    }

    #[test]
    fn bridge_keeps_serving_after_cancel_quit_and_stops_after_final_quit() {
        let f = fixture(true);
        f.w.core.vault_lock().unwrap();
        let handle = f.handle();
        let locked = || ask(&handle, &serde_json::json!({"type": "getFavorites"}));
        // A quit that is cancelled: the bridge answers again once the session re-opens.
        f.w.core.begin_quit();
        on_quit_outcome(&f.state, ExitOutcome::Stay);
        f.w.core.cancel_quit();
        assert_eq!(locked()["type"], "locked");
        // The final decision stops the server.
        f.w.core.begin_quit();
        on_quit_outcome(&f.state, ExitOutcome::Exit);
        assert!(
            std::net::TcpStream::connect_timeout(
                &handle.server.local_addr(),
                Duration::from_secs(1)
            )
            .is_err(),
            "the listener is gone"
        );
        assert!(f
            .w
            .fake
            .shared_secrets
            .get(ENDPOINT_SERVICE, ENDPOINT_ACCOUNT)
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_corrupt_pairing_list_does_not_start_the_bridge_and_a_reset_repairs_it() {
        let f = unstarted(true, None);
        f.w.fake
            .secrets
            .set(PAIRING_SERVICE, PAIRING_ACCOUNT, b"not json")
            .unwrap();
        assert!(matches!(
            f.state.start(),
            Err(BridgeStartError::PairingStore(PairingError::Corrupt))
        ));
        assert!(f.state.handle().is_none());
        assert!(f.state.pairing_store_failed());
        f.state.reset_pairings().unwrap();
        assert!(
            f.state.handle().is_some(),
            "the bridge started after the reset"
        );
        assert!(!f.state.pairing_store_failed());
        assert!(f.handle().pairing.list().is_empty());
    }

    #[test]
    fn proposal_events_carry_no_secret_and_name_their_kind() {
        let f = fixture(true);
        let secret = pair(&f);
        let reply = f.handle().service.handle(Request::ProposeSaveLogin {
            token: "t".into(),
            url: "https://a.example/login".into(),
            username: "me".into(),
            password: "hunter2-secret".into(),
            pairing: Some(secret),
        });
        assert_eq!(
            reply,
            mypass_bridge::protocol::Response::SaveProposalAccepted
        );
        let events = lock(&f.events.proposals);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ProposalKind::Login);
        assert_eq!(events[0].username.as_deref(), Some("me"));
        assert!(!format!("{events:?}").contains("hunter2-secret"));
    }

    #[test]
    fn open_browser_settings_goes_to_the_browser_pane() {
        let f = fixture(true);
        let secret = pair(&f);
        f.handle().service.handle(Request::OpenBrowserSettings {
            token: "t".into(),
            pairing: Some(secret),
        });
        assert_eq!(*lock(&f.events.panes), [Some("browser".to_owned())]);
    }

    #[test]
    fn bridge_deps_use_passkey_service() {
        let f = fixture(true);
        let secret = pair(&f);
        let reply = ask(
            &f.handle(),
            &serde_json::json!({"type": "getPasskeyProxyStatus", "pairing": secret}),
        );
        assert_eq!(
            reply,
            serde_json::json!({
                "type": "passkeyProxyStatus", "unlocked": true, "paired": true,
                "attachAllowed": true, "enabled": true
            })
        );
    }

    #[test]
    fn passkey_proxy_changed_event_emitted_once_when_extension_toggles() {
        let f = fixture(true);
        let secret = pair(&f);
        let toggle = |enabled: bool| {
            ask(
                &f.handle(),
                &serde_json::json!({
                    "type": "setPasskeyProxyEnabled", "enabled": enabled, "pairing": secret
                }),
            )
        };
        // Already on: no change, no event.
        assert_eq!(toggle(true)["enabled"], true);
        assert_eq!(f.events.count("settings_changed"), 0);
        assert_eq!(toggle(false)["enabled"], false);
        assert_eq!(toggle(false)["enabled"], false);
        assert_eq!(f.events.count("settings_changed"), 1);
        {
            // The full stored snapshot, the same one the settings commands emit.
            let sent = lock(&f.events.settings);
            assert!(!sent[0].passkey_proxy_enabled);
            assert_eq!(sent[0], f.w.core.settings.get());
        }
        assert_eq!(toggle(true)["enabled"], true);
        assert_eq!(f.events.count("settings_changed"), 2);
        assert!(lock(&f.events.settings)[1].passkey_proxy_enabled);
    }

    /// A create request as Chrome hands it over (origin in the proxy override, ES256 offered).
    fn passkey_create(secret: &str, request_id: i64) -> Request {
        let details = serde_json::json!({
            "challenge": "MDEyMzQ1Njc4OWFiY2RlZi1jaGFsbGVuZ2U",
            "rp": {"id": "example.com", "name": "Example"},
            "user": {"id": "dXNlci0x", "name": "alice", "displayName": "Alice"},
            "pubKeyCredParams": [{"type": "public-key", "alg": -7}],
            "extensions": {"remoteDesktopClientOverride":
                {"origin": "https://example.com", "sameOriginWithAncestors": true}},
        });
        Request::PasskeyCreate {
            token: "t".into(),
            request_id,
            request_details_json: details.to_string(),
            origin_url: "https://example.com".into(),
            pairing: Some(secret.to_owned()),
        }
    }

    fn passkey_confirm(secret: &str, request_id: i64) -> Request {
        Request::PasskeyCreateConfirm {
            token: "t".into(),
            request_id,
            entry_id: None,
            replace: false,
            pairing: Some(secret.to_owned()),
        }
    }

    #[test]
    fn passkey_pending_ceremony_is_dropped_by_a_lock_of_the_started_bridge() {
        use mypass_bridge::protocol::Response;
        let f = fixture(true);
        let secret = pair(&f);
        let service = f.handle().service.clone();
        assert!(matches!(
            service.handle(passkey_create(&secret, 1)),
            Response::PasskeyCreatePending { .. }
        ));
        f.w.core.vault_lock().unwrap();
        f.w.core.vault_unlock(crate::testing::pw("pw")).unwrap();
        let Response::PasskeyCreateComplete {
            response_json: None,
            error: Some(e),
            ..
        } = service.handle(passkey_confirm(&secret, 1))
        else {
            panic!("the ceremony survived the lock");
        };
        assert_eq!(
            (e.name.as_str(), e.message.as_str()),
            ("NotAllowedError", "No pending request")
        );
        // Sanity: without a lock in between the same steps succeed.
        assert!(matches!(
            service.handle(passkey_create(&secret, 2)),
            Response::PasskeyCreatePending { .. }
        ));
        assert!(matches!(
            service.handle(passkey_confirm(&secret, 2)),
            Response::PasskeyCreateComplete {
                response_json: Some(_),
                error: None,
                ..
            }
        ));
    }
}
