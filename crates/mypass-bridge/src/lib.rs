//! The browser bridge between the MyPass app and the Chromium extension.
//!
//! Modules are added task by task; the ones that touch the vault sit behind the `server` feature so
//! the native messaging host links none of that code.

/// The native messaging host name registered with every browser.
pub const HOST_NAME: &str = "com.mypass.chromehost";

/// The one extension id allowed to talk to the host.
pub const EXTENSION_ID: &str = "hjijbabodfldconmhdcdphlbgaajkejn";

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
