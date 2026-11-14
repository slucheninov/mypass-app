//! Secret storage.
//!
//! `Platform.secrets` is app-private (device-unlock wrapping material, later app state).
//! `Platform.shared_secrets` holds what the ChromeHost must also read (bridge endpoint and token):
//! on macOS and, until DPAPI arrives in stage 6, on Windows it is a `FileSecretStore` (a same-user
//! 0600 file) under `AppPaths::secrets_shared`; it holds only the per-launch endpoint.

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
