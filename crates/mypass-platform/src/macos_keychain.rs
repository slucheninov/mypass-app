//! The app-private secret store on the macOS Keychain.
//!
//! Every item is a generic password (service and account as given by the caller) in the user's
//! login keychain, which is the file-based keychain: no `kSecUseDataProtectionKeychain`, because the
//! data-protection keychain needs the `keychain-access-groups` entitlement and a provisioning
//! profile that a non-sandboxed Developer ID app does not have. Consequences: no access group is
//! involved, and no `kSecAttrAccessible` class is set (file-keychain items never sync to iCloud, so
//! "this device only" is inherent, and the attribute is rejected or ignored there).
//!
//! Items are created with the default access (only the creating application is trusted). There is
//! no store shared with the native host yet: trusting a host binary needs the signed app bundle
//! (ROADMAP, Stage 3).
//!
//! Everything goes through the safe `security-framework` crate; this module has no `unsafe`.
//! No secret ever reaches an error or a `Debug` output: errors carry the numeric `OSStatus` only.

use crate::{PlatformError, PlatformResult, SecretStore};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

/// `errSecItemNotFound`.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecSuccess`.
const ERR_SEC_SUCCESS: i32 = 0;

/// A [`SecretStore`] on the login keychain.
#[derive(Debug, Clone, Default)]
pub(crate) struct MacKeychainSecretStore;

impl MacKeychainSecretStore {
    /// Items only the creating application may read.
    pub(crate) fn app_only() -> Self {
        Self
    }
}

/// The platform error of a failed Security call: the status code only, never any item data.
fn failed(code: i32) -> PlatformError {
    PlatformError::Failed(format!("keychain status {code}"))
}

/// Whether `code` is "no such item".
fn is_not_found(code: i32) -> bool {
    code == ERR_SEC_ITEM_NOT_FOUND
}

/// Maps a status of an operation whose "not found" counts as success to a result.
fn status_ok_or_absent(code: i32) -> PlatformResult<()> {
    if code == ERR_SEC_SUCCESS || is_not_found(code) {
        Ok(())
    } else {
        Err(failed(code))
    }
}

impl SecretStore for MacKeychainSecretStore {
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
        // Delete first, then add a fresh item, instead of updating in place: an item another
        // process pre-created under the same service and account may carry a broad access list, and
        // an in-place update would hand our secret to it. A fresh item is creator-only. A crash
        // between the two calls loses the stored secret; for the unlock key the user then falls
        // back to the master password. A planted item can only yield a wrong unlock key, which
        // fails to decrypt.
        self.delete(service, account)?;
        set_generic_password(service, account, secret).map_err(|e| failed(e.code()))
    }

    fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>> {
        match get_generic_password(service, account) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if is_not_found(e.code()) => Ok(None),
            Err(e) => Err(failed(e.code())),
        }
    }

    fn delete(&self, service: &str, account: &str) -> PlatformResult<()> {
        match delete_generic_password(service, account) {
            Ok(()) => Ok(()),
            Err(e) => status_ok_or_absent(e.code()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_is_recognised() {
        assert!(is_not_found(-25300));
        assert!(!is_not_found(-25293));
        assert!(!is_not_found(0));
    }

    #[test]
    fn delete_status_maps_success_and_absent_to_ok() {
        assert!(status_ok_or_absent(0).is_ok());
        assert!(status_ok_or_absent(-25300).is_ok());
        assert!(status_ok_or_absent(-25293).is_err());
    }

    #[test]
    fn other_status_maps_to_failed_with_the_code_only() {
        assert_eq!(
            failed(-25293),
            PlatformError::Failed("keychain status -25293".to_owned())
        );
    }

    // Real-Keychain test: ignored by default (`AGENTS.md`: no non-ignored test touches the real
    // Keychain) and additionally gated on `MYPASS_KEYCHAIN_TESTS=1`. Run with
    // `MYPASS_KEYCHAIN_TESTS=1 cargo test -p mypass-platform -- --ignored keychain_`. It uses a
    // unique service name and removes its item, so it never touches an item of the real app.

    fn unique_service() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("com.mypass.tests.app.{}.{nanos}", std::process::id())
    }

    #[test]
    #[ignore = "uses the real login keychain; set MYPASS_KEYCHAIN_TESTS=1"]
    fn keychain_app_only_round_trip() {
        if std::env::var("MYPASS_KEYCHAIN_TESTS").as_deref() != Ok("1") {
            println!("skipped: set MYPASS_KEYCHAIN_TESTS=1 to use the real keychain");
            return;
        }
        let store = MacKeychainSecretStore::app_only();
        let service = unique_service();
        let account = "test-account";
        // Clean up even when an assertion fails halfway.
        struct Cleanup<'a>(&'a MacKeychainSecretStore, &'a str, &'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.delete(self.1, self.2);
            }
        }
        let _cleanup = Cleanup(&store, &service, account);

        assert_eq!(store.get(&service, account), Ok(None));
        store.set(&service, account, b"first").unwrap();
        assert_eq!(store.get(&service, account), Ok(Some(b"first".to_vec())));
        store.set(&service, account, b"second \x00\xff").unwrap();
        assert_eq!(
            store.get(&service, account),
            Ok(Some(b"second \x00\xff".to_vec()))
        );
        store.delete(&service, account).unwrap();
        assert_eq!(store.get(&service, account), Ok(None));
        store.delete(&service, account).unwrap();
    }
}
