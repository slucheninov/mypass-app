//! The passkey seam: stage 4 implements [`PasskeyHandler`]; stage 3 answers with a placeholder
//! that keeps the extension from ever attaching the passkey proxy.

#![deny(clippy::wildcard_enum_match_arm)]

use mypass_session::session::VaultSession;
use mypass_session::settings::SettingsStore;

use crate::protocol::{Request, Response};
use crate::views::error;

/// What a passkey handler may use. `unlocked` is the lock state the gates saw for this request.
pub struct PasskeyContext<'a> {
    pub session: &'a VaultSession,
    pub settings: &'a SettingsStore,
    pub unlocked: bool,
}

/// Handles the seven passkey requests, after the quit, lock and pairing gates.
pub trait PasskeyHandler: Send + Sync {
    fn handle(&self, ctx: &PasskeyContext<'_>, request: &Request) -> Response;
    /// Called when the session locks, so pending ceremonies can be dropped.
    fn on_session_locked(&self) {}
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
