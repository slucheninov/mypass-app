//! Secret stores on the macOS Keychain.
//!
//! Mechanism: every item is a generic password (`kSecClassGenericPassword`, service and account
//! as given by the caller) in the user's login keychain, which is the file-based keychain: no
//! `kSecUseDataProtectionKeychain`, because the data-protection keychain needs the
//! `keychain-access-groups` entitlement and a provisioning profile that a non-sandboxed Developer ID
//! app does not have. Consequences: no access group is involved, and no `kSecAttrAccessible` class
//! is set (file-keychain items never sync to iCloud, so "this device only" is inherent, and the
//! attribute is rejected or ignored there).
//!
//! Two flavours share one body:
//!
//! - [`MacKeychainSecretStore::app_only`] creates items with the default access: only the creating
//!   application is trusted (`Platform.secrets`: the unlock key and other app-private secrets).
//! - [`MacKeychainSecretStore::shared_with`] builds a `SecAccess` that trusts a list of executables
//!   (the running app and `AppPaths::host_exe`) and passes it as `kSecAttrAccess` to `SecItemAdd`
//!   (`Platform.shared_secrets`: what the native messaging host must read). A trusted application
//!   is matched by its designated requirement, so both binaries must carry the same stable team
//!   signature (`W645B93PPQ`, stage 5); an unsigned or ad-hoc build never matches and the host
//!   would get a prompt or a refusal. Until stage 5 the shared ACL is therefore untested against a
//!   real host; the ignored tests only prove the round trip with the test binary trusting itself.
//!
//! The safe `security-framework` crate does lookups, updates and deletes. It cannot attach an
//! access list to a new item, and `-sys` has no binding for `SecAccessCreate` and
//! `SecTrustedApplicationCreateFromPath`, so those few calls are declared here and wrapped in safe
//! functions. Both functions are deprecated by Apple but are the only access-list API of the
//! file-based keychain.
//!
//! No secret ever reaches an error or a `Debug` output: errors carry the numeric `OSStatus` only.
//! The secret is copied into a `CFData` for the add call; that copy is released by the Keychain
//! call chain and cannot be zeroed from here, which is the same exposure as any Security.framework
//! client.

#![allow(unsafe_code)]

use crate::{PlatformError, PlatformResult, SecretStore};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFMutableDictionary;
use core_foundation::string::{CFString, CFStringRef};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use security_framework_sys::item::{
    kSecAttrAccount, kSecAttrService, kSecClass, kSecClassGenericPassword, kSecValueData,
};
use security_framework_sys::keychain_item::SecItemAdd;
use std::ffi::{c_char, c_void, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// `errSecItemNotFound`.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecSuccess`.
const ERR_SEC_SUCCESS: i32 = 0;
/// The label shown for the access list in Keychain Access.
const ACCESS_DESCRIPTOR: &str = "MyPass";

type OsStatus = i32;
type SecAccessRef = *mut c_void;
type SecTrustedApplicationRef = *mut c_void;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustedApplicationCreateFromPath(
        path: *const c_char,
        app: *mut SecTrustedApplicationRef,
    ) -> OsStatus;
    fn SecAccessCreate(
        descriptor: CFStringRef,
        trusted_list: core_foundation::array::CFArrayRef,
        access: *mut SecAccessRef,
    ) -> OsStatus;
    static kSecAttrAccess: CFStringRef;
}

/// A [`SecretStore`] on the login keychain; see the module docs for the two flavours.
#[derive(Debug, Clone)]
pub(crate) struct MacKeychainSecretStore {
    /// `None`: default access (only the creating app). `Some`: the executables the item trusts.
    trusted_paths: Option<Vec<PathBuf>>,
}

impl MacKeychainSecretStore {
    /// Items only the creating application may read.
    pub(crate) fn app_only() -> Self {
        Self {
            trusted_paths: None,
        }
    }

    /// Items that every executable in `trusted` may read without a prompt (the app itself must be
    /// in the list: a list replaces the default access, it does not extend it).
    pub(crate) fn shared_with(trusted: Vec<PathBuf>) -> Self {
        Self {
            trusted_paths: Some(trusted),
        }
    }

    /// The executables an item created by this store trusts; `None` for the default access.
    #[cfg(test)]
    fn trusted_paths(&self) -> Option<&[PathBuf]> {
        self.trusted_paths.as_deref()
    }
}

/// The platform error of a failed Security call: the status code only, never any item data.
fn failed(code: OsStatus) -> PlatformError {
    PlatformError::Failed(format!("keychain status {code}"))
}

/// Whether `code` is "no such item".
fn is_not_found(code: OsStatus) -> bool {
    code == ERR_SEC_ITEM_NOT_FOUND
}

/// Maps a status of an operation whose "not found" counts as success to a result.
fn status_ok_or_absent(code: OsStatus) -> PlatformResult<()> {
    if code == ERR_SEC_SUCCESS || is_not_found(code) {
        Ok(())
    } else {
        Err(failed(code))
    }
}

/// Builds a `SecAccess` that trusts exactly `paths`.
fn make_access(paths: &[PathBuf]) -> PlatformResult<CFType> {
    let mut apps: Vec<CFType> = Vec::with_capacity(paths.len());
    for path in paths {
        apps.push(trusted_application(path)?);
    }
    let list = CFArray::from_CFTypes(&apps);
    let descriptor = CFString::new(ACCESS_DESCRIPTOR);
    let mut access: SecAccessRef = std::ptr::null_mut();
    // SAFETY: `descriptor` and `list` are live CF objects for the whole call, `access` is a valid
    // out-pointer, and on success the callee returns a +1 reference that is adopted below.
    let status = unsafe {
        SecAccessCreate(
            descriptor.as_concrete_TypeRef(),
            list.as_concrete_TypeRef(),
            &mut access,
        )
    };
    if status != ERR_SEC_SUCCESS || access.is_null() {
        return Err(failed(status));
    }
    // SAFETY: `access` is a non-null `SecAccessRef` returned by a "Create" function, so we own one
    // reference and `wrap_under_create_rule` releases it on drop.
    Ok(unsafe { CFType::wrap_under_create_rule(access as CFTypeRef) })
}

/// Creates the `SecTrustedApplication` of the executable at `path`.
fn trusted_application(path: &Path) -> PlatformResult<CFType> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| PlatformError::Failed("keychain: path contains a NUL byte".to_owned()))?;
    let mut app: SecTrustedApplicationRef = std::ptr::null_mut();
    // SAFETY: `c_path` is a NUL-terminated string that outlives the call and `app` is a valid
    // out-pointer; on success the callee returns a +1 reference that is adopted below.
    let status = unsafe { SecTrustedApplicationCreateFromPath(c_path.as_ptr(), &mut app) };
    if status != ERR_SEC_SUCCESS || app.is_null() {
        return Err(failed(status));
    }
    // SAFETY: `app` is a non-null `SecTrustedApplicationRef` from a "Create" function (we own one
    // reference, released by the wrapper on drop).
    Ok(unsafe { CFType::wrap_under_create_rule(app as CFTypeRef) })
}

/// Adds a generic password carrying `access` (`SecItemAdd`).
fn add_with_access(
    service: &str,
    account: &str,
    secret: &[u8],
    access: &CFType,
) -> PlatformResult<()> {
    let service = CFString::new(service);
    let account = CFString::new(account);
    let data = CFData::from_buffer(secret);
    let mut query = CFMutableDictionary::new();
    // SAFETY: these are constant `CFStringRef` globals exported by Security.framework, valid for
    // the life of the process; `wrap_under_get_rule` retains them.
    let (class_key, class_value, service_key, account_key, data_key, access_key) = unsafe {
        (
            CFString::wrap_under_get_rule(kSecClass),
            CFString::wrap_under_get_rule(kSecClassGenericPassword),
            CFString::wrap_under_get_rule(kSecAttrService),
            CFString::wrap_under_get_rule(kSecAttrAccount),
            CFString::wrap_under_get_rule(kSecValueData),
            CFString::wrap_under_get_rule(kSecAttrAccess),
        )
    };
    query.add(&class_key.as_CFTypeRef(), &class_value.as_CFTypeRef());
    query.add(&service_key.as_CFTypeRef(), &service.as_CFTypeRef());
    query.add(&account_key.as_CFTypeRef(), &account.as_CFTypeRef());
    query.add(&data_key.as_CFTypeRef(), &data.as_CFTypeRef());
    query.add(&access_key.as_CFTypeRef(), &access.as_CFTypeRef());
    let attributes = query.to_immutable();
    // SAFETY: `attributes` is a live CFDictionary for the whole call, and a null result pointer
    // is allowed (the added item is not returned).
    let status = unsafe { SecItemAdd(attributes.as_concrete_TypeRef(), std::ptr::null_mut()) };
    if status == ERR_SEC_SUCCESS {
        Ok(())
    } else {
        Err(failed(status))
    }
}

impl SecretStore for MacKeychainSecretStore {
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
        match &self.trusted_paths {
            // Updates in place (or adds), so a failed write never loses the previous secret.
            None => set_generic_password(service, account, secret).map_err(|e| failed(e.code())),
            // An existing item keeps the access list it was created with, which would go stale
            // after a re-sign, so the item is deleted and added again with a fresh list.
            Some(paths) => {
                let access = make_access(paths)?;
                self.delete(service, account)?;
                add_with_access(service, account, secret, &access)
            }
        }
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

    #[test]
    fn shared_store_trusts_exactly_the_paths_it_was_given() {
        let paths = vec![
            PathBuf::from("/app/MyPass"),
            PathBuf::from("/host/mypass-host"),
        ];
        let store = MacKeychainSecretStore::shared_with(paths.clone());
        assert_eq!(store.trusted_paths(), Some(paths.as_slice()));
        assert_eq!(MacKeychainSecretStore::app_only().trusted_paths(), None);
    }

    #[test]
    fn a_path_with_a_nul_byte_is_refused_before_any_ffi_call() {
        let bad = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/a\0b"));
        assert!(trusted_application(&bad).is_err());
    }

    // Real-Keychain tests: never run by default (`AGENTS.md`: no non-ignored test touches the real
    // Keychain). Run with `cargo test -p mypass-platform -- --ignored keychain_`. Each uses a unique
    // service name and removes its item, so it never touches an item of the real app.

    fn unique_service(tag: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("com.mypass.tests.{tag}.{}.{nanos}", std::process::id())
    }

    fn round_trip(store: &MacKeychainSecretStore, tag: &str) {
        let service = unique_service(tag);
        let account = "test-account";
        // Clean up even when an assertion fails halfway.
        struct Cleanup<'a>(&'a MacKeychainSecretStore, &'a str, &'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.delete(self.1, self.2);
            }
        }
        let _cleanup = Cleanup(store, &service, account);

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

    #[test]
    #[ignore = "uses the real login keychain"]
    fn keychain_app_only_round_trip() {
        round_trip(&MacKeychainSecretStore::app_only(), "app");
    }

    #[test]
    #[ignore = "uses the real login keychain"]
    fn keychain_shared_round_trip_with_the_test_binary_trusted() {
        let exe = std::env::current_exe().unwrap();
        let store = MacKeychainSecretStore::shared_with(vec![exe.clone(), exe]);
        round_trip(&store, "shared");
    }
}
