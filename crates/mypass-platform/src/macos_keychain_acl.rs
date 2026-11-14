//! The macOS endpoint store (ruling R4): a login-keychain generic password whose access list
//! trusts the running binary and one peer binary, each by its designated code requirement.
//!
//! `SecTrustedApplicationCreateFromPath` records the designated requirement of the code at a
//! path; for our Developer ID builds that is `identifier ... and anchor apple generic and ... and
//! certificate leaf[subject.OU] = W645B93PPQ`, which survives updates and refuses another binary
//! put at the same path. Before trusting anything the store checks that both the running binary
//! and the peer satisfy their team requirement **and** that both designated requirements are
//! team-based (never a cdhash, which is what an ad-hoc signature would record). The access list
//! is set at creation (`kSecAttrAccess`), and the item is only ever deleted and re-added, never
//! updated, so an item planted with a broader access list is never reused.
//!
//! Reads and deletes go through the safe `security-framework` calls of the app-only store; only
//! the creation with an access list and the designated-requirement text need the legacy C API,
//! which `security-framework` does not wrap. Errors carry status codes only, never item data.
//!
//! Signing questions go through [`MacSignatureProbe`] (safe `security-framework` calls).

// `unsafe` is denied crate-wide and allowed in this module only: the access-list and requirement
// calls are C functions declared here. Each block is minimal and carries a `// SAFETY:` note; the
// guard `unsafe_only_in_allow_listed_modules` lists this file.
#![allow(unsafe_code)]

use crate::endpoint_store::{
    delete_then_add, designated_requirement_is_team_based, requirement_identifier, ItemOps,
    SignatureProbe, StoreSide,
};
use crate::macos_keychain::MacKeychainSecretStore;
use crate::{PlatformError, PlatformResult, SecretStore};
use core_foundation::base::{CFType, CFTypeRef, OSStatus, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;
use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement, SecStaticCode};
use std::ffi::{c_char, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut CFTypeRef) -> OSStatus;
    fn SecAccessCreate(
        descriptor: CFStringRef,
        trusted_list: CFTypeRef,
        access: *mut CFTypeRef,
    ) -> OSStatus;
    fn SecItemAdd(attributes: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecCodeCopyDesignatedRequirement(
        code: CFTypeRef,
        flags: u32,
        requirement: *mut CFTypeRef,
    ) -> OSStatus;
    fn SecRequirementCopyString(
        requirement: CFTypeRef,
        flags: u32,
        text: *mut CFTypeRef,
    ) -> OSStatus;

    static kSecClass: CFStringRef;
    static kSecClassGenericPassword: CFStringRef;
    static kSecAttrService: CFStringRef;
    static kSecAttrAccount: CFStringRef;
    static kSecAttrLabel: CFStringRef;
    static kSecAttrAccess: CFStringRef;
    static kSecValueData: CFStringRef;
}

const ERR_SEC_SUCCESS: OSStatus = 0;

fn failed(what: &str, code: OSStatus) -> PlatformError {
    PlatformError::Failed(format!("{what}: keychain status {code}"))
}

/// Takes ownership of a +1 reference returned by a `Copy`/`Create` call.
fn owned(status: OSStatus, raw: CFTypeRef, what: &str) -> PlatformResult<CFType> {
    if status != ERR_SEC_SUCCESS || raw.is_null() {
        return Err(failed(what, status));
    }
    // SAFETY: the call succeeded and returned a non-null object under the create rule; wrapping
    // it transfers that single reference to `CFType`, which releases it on drop.
    Ok(unsafe { CFType::wrap_under_create_rule(raw) })
}

/// The attribute keys and the class value of an add, retained.
struct AddKeys {
    class: CFString,
    generic_password: CFString,
    service: CFString,
    account: CFString,
    label: CFString,
    access: CFString,
    data: CFString,
}

fn add_keys() -> AddKeys {
    // SAFETY: the Security framework constants are valid, immutable `CFString`s for the life of
    // the process; reading them is a plain load and the get rule retains each one.
    unsafe {
        AddKeys {
            class: CFString::wrap_under_get_rule(kSecClass),
            generic_password: CFString::wrap_under_get_rule(kSecClassGenericPassword),
            service: CFString::wrap_under_get_rule(kSecAttrService),
            account: CFString::wrap_under_get_rule(kSecAttrAccount),
            label: CFString::wrap_under_get_rule(kSecAttrLabel),
            access: CFString::wrap_under_get_rule(kSecAttrAccess),
            data: CFString::wrap_under_get_rule(kSecValueData),
        }
    }
}

/// The designated requirement of `code` (a `SecCode` or `SecStaticCode`) as text.
fn designated_requirement_text(code: CFTypeRef) -> PlatformResult<String> {
    let mut requirement: CFTypeRef = ptr::null();
    // SAFETY: `code` is a live code object owned by the caller; the out pointer is valid.
    let status = unsafe { SecCodeCopyDesignatedRequirement(code, 0, &mut requirement) };
    let requirement = owned(status, requirement, "designated requirement")?;
    let mut text: CFTypeRef = ptr::null();
    // SAFETY: `requirement` is a live `SecRequirement`; the out pointer is valid.
    let status = unsafe { SecRequirementCopyString(requirement.as_CFTypeRef(), 0, &mut text) };
    let text = owned(status, text, "requirement text")?;
    text.downcast_into::<CFString>()
        .map(|s| s.to_string())
        .ok_or_else(|| PlatformError::Failed("requirement text is not a string".into()))
}

/// The designated requirement of the binary at `path`.
pub(crate) fn designated_requirement_of(path: &Path) -> PlatformResult<String> {
    let url = CFURL::from_path(path, false)
        .ok_or_else(|| PlatformError::Failed("binary path is not a file URL".into()))?;
    let code =
        SecStaticCode::from_path(&url, Flags::NONE).map_err(|e| failed("static code", e.code()))?;
    designated_requirement_text(code.as_CFTypeRef())
}

/// The designated requirement of the running binary.
fn own_designated_requirement() -> PlatformResult<String> {
    let code = SecCode::for_self(Flags::NONE).map_err(|e| failed("own code", e.code()))?;
    designated_requirement_text(code.as_CFTypeRef())
}

/// A trusted-application entry for the code at `path`, or for the running binary when `None`.
fn trusted_application(path: Option<&Path>) -> PlatformResult<CFType> {
    let c_path = path
        .map(|p| CString::new(p.as_os_str().as_bytes()))
        .transpose()
        .map_err(|_| PlatformError::Failed("binary path contains a NUL byte".into()))?;
    let raw_path = c_path.as_ref().map_or(ptr::null(), |p| p.as_ptr());
    let mut app: CFTypeRef = ptr::null();
    // SAFETY: `raw_path` is null (the running binary) or a NUL-terminated string that outlives
    // the call; the out pointer is valid.
    let status = unsafe { SecTrustedApplicationCreateFromPath(raw_path, &mut app) };
    owned(status, app, "trusted application")
}

/// An access list trusting the running binary and `peer`, labelled `label`.
fn access_for(peer: &Path, label: &str) -> PlatformResult<CFType> {
    let trusted = core_foundation::array::CFArray::from_CFTypes(&[
        trusted_application(None)?,
        trusted_application(Some(peer))?,
    ]);
    let descriptor = CFString::new(label);
    let mut access: CFTypeRef = ptr::null();
    // SAFETY: `descriptor` and `trusted` are live CF objects for the duration of the call; the
    // out pointer is valid.
    let status = unsafe {
        SecAccessCreate(
            descriptor.as_concrete_TypeRef(),
            trusted.as_CFTypeRef(),
            &mut access,
        )
    };
    owned(status, access, "access list")
}

/// The endpoint record's Keychain store; see the module docs.
#[derive(Debug, Clone)]
pub(crate) struct MacEndpointKeychainStore {
    peer: Option<PathBuf>,
    side: StoreSide,
}

/// The label shown in Keychain Access.
const ITEM_LABEL: &str = "MyPass browser bridge endpoint";

impl MacEndpointKeychainStore {
    /// The store of `side`; `peer` is the other binary (the host for the app, the app for the
    /// host), trusted next to the running binary. Without a peer every write fails.
    pub(crate) fn new(peer: Option<PathBuf>, side: StoreSide) -> Self {
        Self { peer, side }
    }

    /// The peer binary, or the error that stops a write (fail closed).
    fn peer(&self) -> PlatformResult<&Path> {
        self.peer
            .as_deref()
            .ok_or_else(|| PlatformError::Failed("the other MyPass binary was not found".into()))
    }

    /// The running binary and the peer are both team-signed with team-based designated
    /// requirements; otherwise nothing is written.
    fn verify_signatures(&self) -> PlatformResult<()> {
        let peer = self.peer()?;
        let (own_requirement, peer_requirement) = self.side.requirements();
        let probe = MacSignatureProbe;
        if !probe.running_binary_satisfies(own_requirement)
            || !probe.binary_satisfies(peer, peer_requirement)
        {
            return Err(PlatformError::Failed(
                "the app or the host is not signed by the MyPass team".into(),
            ));
        }
        let own_id = requirement_identifier(own_requirement).unwrap_or_default();
        let peer_id = requirement_identifier(peer_requirement).unwrap_or_default();
        if !designated_requirement_is_team_based(&own_designated_requirement()?, own_id)
            || !designated_requirement_is_team_based(&designated_requirement_of(peer)?, peer_id)
        {
            return Err(PlatformError::Failed(
                "a designated requirement is not pinned to the MyPass team".into(),
            ));
        }
        Ok(())
    }
}

/// One write: the item's coordinates, its secret and the access list.
struct AclItem<'a> {
    service: &'a str,
    account: &'a str,
    secret: &'a [u8],
    access: &'a CFType,
}

impl ItemOps for AclItem<'_> {
    fn delete(&self) -> PlatformResult<()> {
        MacKeychainSecretStore::app_only().delete(self.service, self.account)
    }

    fn add(&self) -> i32 {
        let k = add_keys();
        let pairs: [(CFString, CFType); 6] = [
            (k.class, k.generic_password.as_CFType()),
            (k.service, CFString::new(self.service).as_CFType()),
            (k.account, CFString::new(self.account).as_CFType()),
            (k.label, CFString::new(ITEM_LABEL).as_CFType()),
            (k.access, self.access.clone()),
            (k.data, CFData::from_buffer(self.secret).as_CFType()),
        ];
        let attributes = CFDictionary::from_CFType_pairs(&pairs);
        // SAFETY: `attributes` is a live dictionary for the duration of the call; no result is
        // requested (null out pointer is allowed).
        unsafe { SecItemAdd(attributes.as_CFTypeRef(), ptr::null_mut()) }
    }
}

impl SecretStore for MacEndpointKeychainStore {
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
        self.verify_signatures()?;
        let access = access_for(self.peer()?, ITEM_LABEL)?;
        // `SecTrustedApplicationCreateFromPath` records whatever code sits at the path when it
        // runs, so a binary swapped in after the first check would be trusted. Check again now
        // that the access list exists: a swap must then also be undone inside this window, by a
        // process that can already write into the app bundle. Residual risk, recorded in the
        // stage 5 report; the public API offers no way to read the recorded requirement back.
        self.verify_signatures()?;
        delete_then_add(&AclItem {
            service,
            account,
            secret,
            access: &access,
        })
    }

    fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>> {
        MacKeychainSecretStore::app_only().get(service, account)
    }

    fn delete(&self, service: &str, account: &str) -> PlatformResult<()> {
        MacKeychainSecretStore::app_only().delete(service, account)
    }
}

/// [`SignatureProbe`] over the Security framework's code-signing checks.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct MacSignatureProbe;

impl SignatureProbe for MacSignatureProbe {
    fn running_binary_satisfies(&self, requirement: &str) -> bool {
        let Ok(requirement) = requirement.parse::<SecRequirement>() else {
            return false;
        };
        SecCode::for_self(Flags::NONE)
            .and_then(|code| code.check_validity(Flags::NONE, &requirement))
            .is_ok()
    }

    fn binary_satisfies(&self, path: &Path, requirement: &str) -> bool {
        let Ok(requirement) = requirement.parse::<SecRequirement>() else {
            return false;
        };
        let Some(url) = CFURL::from_path(path, false) else {
            return false;
        };
        SecStaticCode::from_path(&url, Flags::NONE)
            .and_then(|code| code.check_validity(Flags::NONE, &requirement))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint_store::{APP_REQUIREMENT, HOST_REQUIREMENT, TEAM_ID};

    /// An Apple-signed system binary: valid input for every signing call, never team-signed.
    const SYSTEM_BINARY: &str = "/bin/ls";

    #[test]
    fn probe_checks_requirements_for_real() {
        let probe = MacSignatureProbe;
        assert!(probe.binary_satisfies(Path::new(SYSTEM_BINARY), "anchor apple"));
        assert!(!probe.binary_satisfies(Path::new(SYSTEM_BINARY), HOST_REQUIREMENT));
        assert!(!probe.binary_satisfies(Path::new("/no/such/binary"), "anchor apple"));
        assert!(!probe.binary_satisfies(Path::new(SYSTEM_BINARY), "not a requirement ("));
        // The test binary is never signed by the team.
        assert!(!probe.running_binary_satisfies(APP_REQUIREMENT));
        assert!(!probe.running_binary_satisfies(HOST_REQUIREMENT));
    }

    #[test]
    fn designated_requirement_text_of_a_system_binary() {
        let text = designated_requirement_of(Path::new(SYSTEM_BINARY)).unwrap();
        assert!(text.contains("identifier \"com.apple.ls\""), "{text}");
        assert!(text.contains("anchor apple"), "{text}");
        assert!(!designated_requirement_is_team_based(&text, "com.apple.ls"));
    }

    /// Refused before any Keychain call: the test binary and `/bin/ls` are not team-signed.
    #[test]
    fn unsigned_binaries_are_refused_before_any_keychain_call() {
        let store =
            MacEndpointKeychainStore::new(Some(PathBuf::from(SYSTEM_BINARY)), StoreSide::App);
        let err = store.verify_signatures().unwrap_err();
        assert!(matches!(err, PlatformError::Failed(ref m) if m.contains("not signed")));
    }

    #[test]
    fn a_missing_peer_fails_closed() {
        let store = MacEndpointKeychainStore::new(None, StoreSide::App);
        let err = store
            .set("com.mypass.tests.never", "loopback", b"x")
            .unwrap_err();
        assert!(matches!(err, PlatformError::Failed(ref m) if m.contains("not found")));
    }

    /// The real access list: needs `MYPASS_KEYCHAIN_TESTS=1`, a team-signed test binary (run
    /// through the signed app's test harness) and `MYPASS_TEST_PEER_BINARY` naming the signed
    /// host. Checks the round trip and that `security dump-keychain -a` lists the team in the
    /// item's access list, never a cdhash.
    #[test]
    #[ignore = "touches the real Keychain; needs MYPASS_KEYCHAIN_TESTS=1 and team-signed binaries"]
    fn item_acl_trusts_team_requirement() {
        if std::env::var("MYPASS_KEYCHAIN_TESTS").as_deref() != Ok("1") {
            println!("skipped: MYPASS_KEYCHAIN_TESTS is not 1");
            return;
        }
        let Ok(peer) = std::env::var("MYPASS_TEST_PEER_BINARY") else {
            println!("skipped: MYPASS_TEST_PEER_BINARY is not set");
            return;
        };
        if !MacSignatureProbe.running_binary_satisfies(APP_REQUIREMENT) {
            println!("skipped: the test binary is not signed as {APP_REQUIREMENT}");
            return;
        }
        let service = format!("com.mypass.tests.endpoint.{}", std::process::id());
        let store = MacEndpointKeychainStore::new(Some(PathBuf::from(peer)), StoreSide::App);
        store.set(&service, "loopback", b"{\"port\":1}").unwrap();
        let read = store.get(&service, "loopback");
        let dump = std::process::Command::new("security")
            .args(["dump-keychain", "-a"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        store.delete(&service, "loopback").unwrap();
        assert_eq!(read.unwrap(), Some(b"{\"port\":1}".to_vec()));
        let dump = dump.unwrap();
        let section = dump
            .split("keychain: ")
            .find(|s| s.contains(&service))
            .expect("the item is in the dump");
        assert!(section.contains(TEAM_ID), "{section}");
        assert!(!section.contains("cdhash"), "{section}");
    }
}
