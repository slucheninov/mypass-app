//! The browser bridge between the MyPass app and the Chromium extension.
//!
//! Modules are added task by task; the ones that touch the vault sit behind the `server` feature so
//! the native messaging host links none of that code.

pub mod codec;
pub mod diagnostics;
pub mod endpoint;
pub mod gating;
#[cfg(feature = "server")]
pub(crate) mod handlers;
#[cfg(feature = "server")]
pub mod pairing;
#[cfg(feature = "server")]
pub mod passkeys;
pub mod protocol;
pub mod relay;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
pub mod service;
#[cfg(all(test, feature = "server"))]
mod testing;
#[cfg(feature = "server")]
pub mod ui;
#[cfg(feature = "server")]
pub mod views;

/// The native messaging host name and the one extension id allowed to talk to the host: one copy,
/// owned by the platform crate that writes the manifests.
pub use mypass_platform::{EXTENSION_ID, HOST_NAME};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_constants_are_pinned() {
        assert_eq!(HOST_NAME, "com.mypass.chromehost");
        assert_eq!(EXTENSION_ID, "hjijbabodfldconmhdcdphlbgaajkejn");
        assert_eq!(EXTENSION_ID.len(), 32);
    }
}
