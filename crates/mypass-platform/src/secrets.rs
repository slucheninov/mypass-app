//! Secret storage.
//!
//! `Platform.secrets` is app-private (device-unlock wrapping material, later app state).
//! `Platform.shared_secrets` holds what the ChromeHost must also read (bridge endpoint and token):
//! the macOS shared Keychain access group (stage 3), DPAPI files under `AppPaths::secrets_shared`
//! on Windows (stage 6).

use crate::PlatformResult;

/// A store of small binary secrets keyed by `(service, account)`.
pub trait SecretStore: Send + Sync {
    /// Creates or replaces the secret.
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()>;
    /// Reads the secret; `Ok(None)` when there is none.
    fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>>;
    /// Deletes the secret; deleting a missing secret is not an error.
    fn delete(&self, service: &str, account: &str) -> PlatformResult<()>;
}
