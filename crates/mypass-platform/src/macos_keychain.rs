//! The app-private secret store on the macOS Keychain.
//!
//! Every item is a generic password (service and account as given by the caller) in the user's
//! login keychain, which is the file-based keychain: no `kSecUseDataProtectionKeychain`, because the
//! data-protection keychain needs the `keychain-access-groups` entitlement and a provisioning
//! profile that a non-sandboxed Developer ID app does not have. Consequences: no access group is
//! involved, and no `kSecAttrAccessible` class is set (file-keychain items never sync to iCloud, so
//! "this device only" is inherent, and the attribute is rejected or ignored there).
//!
//! Items are created with the default access (only the creating application is trusted). The
//! store shared with the native host, whose access list also trusts the team-signed host, is
//! `macos_keychain_acl.rs` (stage 5, ruling R4).
//!
//! Everything goes through the safe `security-framework` crate; this module has no `unsafe`. The
//! one exception, the attribute-only no-UI lookup of [`ItemStore::attributes_silently`], lives
//! in the allow-listed `macos_keychain_acl.rs`.
//! No secret ever reaches an error or a `Debug` output: errors carry the numeric `OSStatus` only.

use crate::keychain_device_unlock::{ItemAttributes, ItemStore};
use crate::{PlatformError, PlatformResult, SecretStore};
use core_foundation::data::CFData;
use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords::{delete_generic_password, get_generic_password};

/// `errSecItemNotFound`.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecDuplicateItem`.
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
/// How many times `set` deletes a duplicate and adds again before it reports tampering.
const MAX_ADD_ATTEMPTS: usize = 3;
/// `errSecSuccess`.
const ERR_SEC_SUCCESS: i32 = 0;
/// `errSecInteractionNotAllowed`: the silent query would have needed UI.
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;

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

/// What `set` does after one add attempt.
#[derive(Debug, PartialEq, Eq)]
enum AddStep {
    /// The item is stored.
    Done,
    /// An item already exists: delete it and add again.
    DeleteAndRetry,
    /// Give up with this error.
    Fail(PlatformError),
}

/// The decision after add attempt number `attempt` (1-based) returned `code`. Pure, so the retry
/// rules are tested without the Keychain.
fn next_add_step(code: i32, attempt: usize) -> AddStep {
    if code == ERR_SEC_SUCCESS {
        AddStep::Done
    } else if code != ERR_SEC_DUPLICATE_ITEM {
        AddStep::Fail(failed(code))
    } else if attempt >= MAX_ADD_ATTEMPTS {
        AddStep::Fail(PlatformError::SecretStoreTampered)
    } else {
        AddStep::DeleteAndRetry
    }
}

/// The result of a silent attribute lookup, from its status: "no such item" is `None`, and a
/// lookup that would have needed interaction is an error, never "absent".
fn silent_lookup(
    result: Result<Option<Option<String>>, i32>,
) -> PlatformResult<Option<ItemAttributes>> {
    match result {
        Ok(found) => Ok(found.map(|note| ItemAttributes { note })),
        Err(code) if is_not_found(code) => Ok(None),
        Err(ERR_SEC_INTERACTION_NOT_ALLOWED) => Err(PlatformError::Failed(format!(
            "keychain status {ERR_SEC_INTERACTION_NOT_ALLOWED}: the keychain needs interaction"
        ))),
        Err(code) => Err(failed(code)),
    }
}

/// One add-only `SecItemAdd` of a generic password with the default (creator-only) access and
/// an optional public note (`kSecAttrComment`).
fn add_item_only(service: &str, account: &str, secret: &[u8], note: Option<&str>) -> i32 {
    let mut options = ItemAddOptions::new(ItemAddValue::Data {
        class: ItemClass::generic_password(),
        data: CFData::from_buffer(secret),
    });
    options.set_service(service).set_account_name(account);
    if let Some(note) = note {
        options.set_comment(note);
    }
    match options.add() {
        Ok(()) => ERR_SEC_SUCCESS,
        Err(e) => e.code(),
    }
}

/// Maps a status of an operation whose "not found" counts as success to a result.
fn status_ok_or_absent(code: i32) -> PlatformResult<()> {
    if code == ERR_SEC_SUCCESS || is_not_found(code) {
        Ok(())
    } else {
        Err(failed(code))
    }
}

impl MacKeychainSecretStore {
    /// `set`, optionally with a public note.
    fn add_replacing(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
        note: Option<&str>,
    ) -> PlatformResult<()> {
        // The item is only ever added, never updated: an item another process pre-created under
        // the same service and account may carry a broad access list, and an update (also one that
        // slips in between a delete and an add) would hand our secret to it. `SecItemAdd` refuses
        // to overwrite, so a planted or re-planted item shows up as a duplicate: delete it and add
        // again, at most `MAX_ADD_ATTEMPTS` times, then report tampering or a concurrent writer. A
        // crash between a delete and the add loses the stored secret; for the unlock key the user
        // then falls back to the master password. A planted item can only yield a wrong unlock
        // key, which fails to decrypt.
        let mut attempt = 0;
        loop {
            attempt += 1;
            match next_add_step(add_item_only(service, account, secret, note), attempt) {
                AddStep::Done => return Ok(()),
                AddStep::Fail(e) => return Err(e),
                AddStep::DeleteAndRetry => self.delete(service, account)?,
            }
        }
    }
}

impl SecretStore for MacKeychainSecretStore {
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
        self.add_replacing(service, account, secret, None)
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

/// The `kSecAttrAccount` key of a search-result attribute dictionary.
const ACCOUNT_ATTRIBUTE: &str = "acct";

impl ItemStore for MacKeychainSecretStore {
    /// Lists the accounts under `service` from the item attributes only (`kSecReturnAttributes`,
    /// no data), which never shows an access prompt, also for items of other applications.
    fn accounts(&self, service: &str) -> PlatformResult<Vec<String>> {
        let results = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(service)
            .load_attributes(true)
            .limit(Limit::All)
            .search();
        match results {
            Ok(results) => Ok(results
                .iter()
                .filter_map(|r| r.simplify_dict())
                .filter_map(|mut attributes| attributes.remove(ACCOUNT_ATTRIBUTE))
                .collect()),
            Err(e) if is_not_found(e.code()) => Ok(Vec::new()),
            Err(e) => Err(failed(e.code())),
        }
    }

    fn attributes_silently(
        &self,
        service: &str,
        account: &str,
    ) -> PlatformResult<Option<ItemAttributes>> {
        silent_lookup(crate::macos_keychain_acl::silent_item_attributes(
            service, account,
        ))
    }

    fn set_with_note(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
        note: &str,
    ) -> PlatformResult<()> {
        self.add_replacing(service, account, secret, Some(note))
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
    fn silent_lookup_never_turns_interaction_into_absence() {
        assert_eq!(silent_lookup(Ok(None)), Ok(None));
        assert_eq!(silent_lookup(Err(-25300)), Ok(None));
        assert_eq!(
            silent_lookup(Ok(Some(Some("ab".into())))),
            Ok(Some(ItemAttributes {
                note: Some("ab".into())
            }))
        );
        assert_eq!(
            silent_lookup(Ok(Some(None))),
            Ok(Some(ItemAttributes { note: None }))
        );
        assert!(matches!(
            silent_lookup(Err(-25308)),
            Err(PlatformError::Failed(ref m)) if m.contains("-25308")
        ));
        assert_eq!(silent_lookup(Err(-25293)), Err(failed(-25293)));
    }

    #[test]
    fn add_decisions() {
        assert_eq!(next_add_step(0, 1), AddStep::Done);
        assert_eq!(next_add_step(0, 3), AddStep::Done);
        assert_eq!(next_add_step(-25299, 1), AddStep::DeleteAndRetry);
        assert_eq!(next_add_step(-25299, 2), AddStep::DeleteAndRetry);
        assert_eq!(
            next_add_step(-25299, 3),
            AddStep::Fail(PlatformError::SecretStoreTampered)
        );
        assert_eq!(next_add_step(-25293, 1), AddStep::Fail(failed(-25293)));
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
        assert_eq!(store.accounts(&service), Ok(vec![account.to_owned()]));
        assert_eq!(
            store.attributes_silently(&service, account),
            Ok(Some(ItemAttributes { note: None }))
        );
        store
            .set_with_note(&service, account, b"third", "00ff")
            .unwrap();
        assert_eq!(
            store.attributes_silently(&service, account),
            Ok(Some(ItemAttributes {
                note: Some("00ff".to_owned())
            }))
        );
        store.delete(&service, account).unwrap();
        assert_eq!(store.attributes_silently(&service, account), Ok(None));
        assert_eq!(store.get(&service, account), Ok(None));
        assert_eq!(store.accounts(&service), Ok(Vec::new()));
        store.delete(&service, account).unwrap();
    }
}
