//! The Tauri commands: thin wrappers over `mypass_session::AppCore`, one file per domain.
//!
//! Rules every command follows:
//!
//! - It takes `tauri::State`, calls the facade and returns its result; no rule lives here.
//! - A password arriving as `String` is wrapped by [`into_secret`] on the command's first line.
//! - Nothing is logged or printed (pinned by `command_wrappers_do_not_log_arguments`).
//! - Work that walks the disk or derives a key runs on the blocking pool ([`blocking`]).
//! - The webview never supplies a path for attachments or CSV files (see `crate::files`).
//!
//! Logic that needs more than a delegation sits in a plain function next to its command, so the
//! tests drive it with fakes and no window server.

pub mod app;
pub mod attachment;
pub mod backup;
pub mod draft;
pub mod entry;
pub mod generator;
pub mod group;
pub mod import;
pub mod platform;
pub mod settings;
pub mod sidebar;
pub mod vault;

use mypass_session::{ApiError, ErrorCode};
use secrecy::SecretString;
use zeroize::Zeroize;

/// Wraps a password received from the webview and zeroizes the `String` it arrived in.
///
/// The secret is built from a copy (`SecretString::from(&str)`), not by converting the `String`
/// in place: converting may reallocate and leave the old buffer behind, whereas the copy leaves
/// exactly one buffer to wipe.
pub fn into_secret(password: &mut String) -> SecretString {
    let secret = SecretString::from(password.as_str());
    password.zeroize();
    secret
}

/// The error for a failure that is not the caller's fault (a panicked worker).
pub fn internal(message: impl Into<String>) -> ApiError {
    ApiError {
        code: ErrorCode::Internal,
        message: message.into(),
    }
}

/// Runs `work` on the blocking pool; a panicked or cancelled worker is `Internal`.
pub async fn blocking<T, F>(work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| internal(e.to_string()))?
}

/// `Locked` / `NoVault` unless the vault is open: checked before a dialog opens or a grant is
/// consumed, so a locked app never asks the user for a file.
pub fn ensure_unlocked(core: &mypass_session::AppCore) -> Result<(), ApiError> {
    core.session.with_db(|_| ()).map_err(ApiError::from)
}
