//! The passkey seam and its stage 4 implementation, [`PasskeyService`].
//!
//! Gate order: the bridge service answers quitting, then locked, then pairing, before a request
//! reaches [`PasskeyHandler::handle`] (only `getPasskeyProxyStatus` passes the locked gate).
//! Inside the handler the app-side toggle is checked first, before any parsing: Chrome's attach
//! state can lag the setting, so the app refuses `passkeyCreate` / `passkeyGet` on its own
//! whenever the feature is off, whatever the extension believes.
//!
//! Idle timer: `passkeyCreate`, `passkeyGet`, the status poll, the toggle and cancel never reset
//! the auto-lock timer (`Request::resets_auto_lock_timer`): a page or the background script can
//! produce them without a user gesture. Only confirm and approve (popup clicks) do, and only when
//! served (`Response::was_served`: a completion that carries an error is not).

#![deny(clippy::wildcard_enum_match_arm)]

pub mod errors;
pub mod pending;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mypass_session::scheduler::Scheduler;
use mypass_session::session::VaultSession;
use mypass_session::settings::{SettingsPatch, SettingsStore};

use crate::protocol::{Request, Response};
use crate::ui::BridgeUi;
use crate::views::error;
use errors::{create_error, get_error, NOT_ALLOWED};
use pending::{Epoch, Pending, PendingCreate, PendingGet};

/// What a passkey handler may use. `unlocked` is the lock state the gates saw for this request.
pub struct PasskeyContext<'a> {
    pub session: &'a VaultSession,
    pub settings: &'a SettingsStore,
    pub unlocked: bool,
}

/// Handles the seven passkey requests, after the quit, lock and pairing gates.
pub trait PasskeyHandler: Send + Sync {
    fn handle(&self, ctx: &PasskeyContext<'_>, request: &Request) -> Response;
    /// Called when the session stops being unlocked and when the vault is switched while
    /// unlocked, so pending ceremonies are dropped.
    fn on_session_locked(&self) {}
}

/// The answer to `passkeyCreate` / `passkeyGet` while the app-side toggle is off.
pub(crate) const TURNED_OFF: &str = "MyPass passkeys are turned off in Settings";

/// The stage 4 passkey handler: the proxy status and toggle, cancel, and the pending table the
/// ceremonies (create, confirm, get, approve) share. See the module docs for the gate order.
pub struct PasskeyService {
    pending: Mutex<Pending>,
    scheduler: Arc<dyn Scheduler>,
    ui: Arc<dyn BridgeUi>,
}

impl std::fmt::Debug for PasskeyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasskeyService").finish_non_exhaustive()
    }
}

impl PasskeyService {
    /// `scheduler` gives the monotonic time of the pending TTL (never a clock of its own);
    /// `ui` hears about toggle changes made from the extension.
    #[must_use]
    pub fn new(scheduler: Arc<dyn Scheduler>, ui: Arc<dyn BridgeUi>) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(Pending::new()),
            scheduler,
            ui,
        })
    }

    /// The table. Never hold the guard across a session call (see `pending` module docs).
    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The epoch to read before the vault read of a phase-one handler.
    pub fn epoch(&self) -> Epoch {
        self.pending().epoch()
    }

    /// Stores a create ceremony; `false` when a lock or vault switch cleared the table since
    /// `epoch` was read (the caller then answers `Locked`).
    pub fn put_create(&self, epoch: Epoch, id: i64, p: PendingCreate) -> bool {
        let now = self.scheduler.monotonic_now();
        self.pending().put_create(epoch, id, p, now)
    }

    /// Takes the create ceremony `id` (single use), `None` when unknown or expired.
    pub fn take_create(&self, id: i64) -> Option<PendingCreate> {
        let now = self.scheduler.monotonic_now();
        self.pending().take_create(id, now)
    }

    /// Stores a get ceremony; see [`Self::put_create`].
    pub fn put_get(&self, epoch: Epoch, id: i64, p: PendingGet) -> bool {
        let now = self.scheduler.monotonic_now();
        self.pending().put_get(epoch, id, p, now)
    }

    /// Takes the get ceremony `id` (single use), `None` when unknown or expired.
    pub fn take_get(&self, id: i64) -> Option<PendingGet> {
        let now = self.scheduler.monotonic_now();
        self.pending().take_get(id, now)
    }

    /// `(creates, gets)` held.
    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> (usize, usize) {
        self.pending().len()
    }

    /// The status poll. `attachAllowed` folds in the app-side opt-in, not just the lock state:
    /// the extension mirrors this flag locally, and without the app's value in the answer,
    /// turning the feature off in Settings would leave an already attached Chrome attached
    /// indefinitely (there is no push channel to tell it otherwise). `enabled` is reported
    /// separately, even while locked, so the extension learns the app-side opt-in on its own
    /// and resyncs its cache and attachment without waiting for a service worker restart: the
    /// app is authoritative, the extension's flag is only a cache.
    fn status(ctx: &PasskeyContext<'_>) -> Response {
        let enabled = ctx.settings.get().passkey_proxy_enabled;
        Response::PasskeyProxyStatus {
            unlocked: ctx.unlocked,
            paired: true,
            attach_allowed: ctx.unlocked && enabled,
            enabled,
        }
    }

    /// The toggle from the extension. The answer is the stored value read back after the
    /// update. `SettingsStore::update` keeps a new value in memory even when persisting it
    /// fails, so that value is in force for this run and is what the answer reports; the next
    /// start reads the file again. The UI hears about a real change only. Turning the feature
    /// off drops every pending ceremony, so nothing started before is confirmed after.
    fn set_enabled(&self, ctx: &PasskeyContext<'_>, enabled: bool) -> Response {
        let before = ctx.settings.get().passkey_proxy_enabled;
        // An `Io` error still applied the value in memory (see above); `InvalidValue` cannot
        // come from a boolean. Either way the answer below reads back what is in force.
        let _applied = ctx.settings.update(SettingsPatch {
            passkey_proxy_enabled: Some(enabled),
            ..SettingsPatch::default()
        });
        let stored = ctx.settings.get().passkey_proxy_enabled;
        if !stored {
            self.pending().clear();
        }
        if stored != before {
            self.ui.passkey_proxy_changed(stored);
        }
        Response::PasskeyProxyEnabled { enabled: stored }
    }
}

impl PasskeyHandler for PasskeyService {
    fn handle(&self, ctx: &PasskeyContext<'_>, request: &Request) -> Response {
        let toggle_on = || ctx.settings.get().passkey_proxy_enabled;
        match request {
            Request::GetPasskeyProxyStatus { .. } => Self::status(ctx),
            Request::SetPasskeyProxyEnabled { enabled, .. } => self.set_enabled(ctx, *enabled),
            Request::PasskeyCancel { request_id, .. } => {
                self.pending().remove_both(*request_id);
                Response::PasskeyCancelled {
                    request_id: *request_id,
                }
            }
            // The toggle first, before any parsing (see the module docs).
            Request::PasskeyCreate { request_id, .. } => {
                if !toggle_on() {
                    return create_error(*request_id, NOT_ALLOWED, TURNED_OFF);
                }
                create_error(*request_id, NOT_ALLOWED, NOT_SUPPORTED)
            }
            Request::PasskeyGet { request_id, .. } => {
                if !toggle_on() {
                    return get_error(*request_id, NOT_ALLOWED, TURNED_OFF);
                }
                get_error(*request_id, NOT_ALLOWED, NOT_SUPPORTED)
            }
            Request::PasskeyCreateConfirm { request_id, .. } => {
                create_error(*request_id, NOT_ALLOWED, NOT_SUPPORTED)
            }
            Request::PasskeyGetApprove { request_id, .. } => {
                get_error(*request_id, NOT_ALLOWED, NOT_SUPPORTED)
            }
            // Not passkey requests: the service never routes them here.
            Request::GetCredentials { .. }
            | Request::GetFavorites { .. }
            | Request::ListEntries { .. }
            | Request::SearchEntries { .. }
            | Request::CreateEntry { .. }
            | Request::FillAddress { .. }
            | Request::FillCredential { .. }
            | Request::GetTotp { .. }
            | Request::ProposeSaveLogin { .. }
            | Request::AddEntryIdentifier { .. }
            | Request::ProposeSaveTotp { .. }
            | Request::StartPairing { .. }
            | Request::CompletePairing { .. }
            | Request::RequestUnlock { .. }
            | Request::LockVault { .. }
            | Request::OpenApp { .. }
            | Request::OpenBrowserSettings { .. }
            | Request::ReportBrowserDiagnostics { .. }
            | Request::GeneratePassword { .. }
            | Request::RecordGeneratedPassword { .. }
            | Request::GetGeneratedPasswordHistory { .. }
            | Request::DeleteGeneratedPassword { .. } => error("unsupported request"),
        }
    }

    fn on_session_locked(&self) {
        self.pending().clear();
    }
}

/// The stage 3 placeholder: the proxy reports itself disabled and not attachable, creates and
/// gets fail with a fixed message.
pub struct NotSupportedPasskeys;

const NOT_SUPPORTED: &str = "passkeys are not supported by this version of MyPass yet";

impl PasskeyHandler for NotSupportedPasskeys {
    fn handle(&self, ctx: &PasskeyContext<'_>, request: &Request) -> Response {
        match request {
            Request::GetPasskeyProxyStatus { .. } => Response::PasskeyProxyStatus {
                unlocked: ctx.unlocked,
                paired: true,
                attach_allowed: false,
                enabled: false,
            },
            Request::SetPasskeyProxyEnabled { .. } => {
                Response::PasskeyProxyEnabled { enabled: false }
            }
            Request::PasskeyCancel { request_id, .. } => Response::PasskeyCancelled {
                request_id: *request_id,
            },
            Request::PasskeyCreate { .. }
            | Request::PasskeyCreateConfirm { .. }
            | Request::PasskeyGet { .. }
            | Request::PasskeyGetApprove { .. } => error(NOT_SUPPORTED),
            // Not passkey requests: the service never routes them here.
            Request::GetCredentials { .. }
            | Request::GetFavorites { .. }
            | Request::ListEntries { .. }
            | Request::SearchEntries { .. }
            | Request::CreateEntry { .. }
            | Request::FillAddress { .. }
            | Request::FillCredential { .. }
            | Request::GetTotp { .. }
            | Request::ProposeSaveLogin { .. }
            | Request::AddEntryIdentifier { .. }
            | Request::ProposeSaveTotp { .. }
            | Request::StartPairing { .. }
            | Request::CompletePairing { .. }
            | Request::RequestUnlock { .. }
            | Request::LockVault { .. }
            | Request::OpenApp { .. }
            | Request::OpenBrowserSettings { .. }
            | Request::ReportBrowserDiagnostics { .. }
            | Request::GeneratePassword { .. }
            | Request::RecordGeneratedPassword { .. }
            | Request::GetGeneratedPasswordHistory { .. }
            | Request::DeleteGeneratedPassword { .. } => error("unsupported request"),
        }
    }
}

#[cfg(test)]
mod tests;
