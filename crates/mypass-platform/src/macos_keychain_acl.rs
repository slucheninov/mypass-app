//! The macOS endpoint store (ruling R4): a login-keychain generic password whose access list
//! trusts the running binary and one peer binary, each by its designated code requirement.
//!
//! `SecTrustedApplicationCreateFromPath` records the designated requirement of the code at a
//! path; our builds are signed with the explicit R27 requirement `identifier "<id>" and anchor
//! apple generic and certificate leaf[subject.OU] = "W645B93PPQ"`, which survives updates and
//! certificate renewals and refuses another binary put at the same path. Before trusting anything
//! the store checks that both the running binary and the peer satisfy their team requirement
//! **and** that the designated requirement each carries is exactly that expression, compared in
//! the Security framework's canonical form (never a substring test: an extra `or` branch, a
//! `subject.CN` pin or a cdhash, which an ad-hoc signature records, are all refused). The access list
//! is set at creation (`kSecAttrAccess`), and the item is only ever deleted and re-added, never
//! updated, so an item planted with a broader access list is never reused.
//!
//! Reads and deletes go through the safe `security-framework` calls of the app-only store; only
//! the creation with an access list and the designated-requirement text need the legacy C API,
//! which `security-framework` does not wrap. Errors carry status codes only, never item data.
//!
//! Signing questions go through [`MacSignatureProbe`] (safe `security-framework` calls).
//!
//! This module also holds the one other Keychain query `security-framework` cannot express: the
//! attribute-only, no-UI lookup of an item ([`silent_item_attributes`]) that the device-unlock
//! enrolment checks use (`kSecUseAuthenticationUI = kSecUseAuthenticationUIFail`; the crate only
//! offers `Skip`, which would hide an item that needs interaction instead of failing).

// `unsafe` is denied crate-wide and allowed in this module only: the access-list and requirement
// calls are C functions declared here. Each block is minimal and carries a `// SAFETY:` note; the
// guard `unsafe_only_in_allow_listed_modules` lists this file.
#![allow(unsafe_code)]

use crate::endpoint_store::{
    delete_then_add, designated_requirement_is_exact, ItemOps, SignatureProbe, StoreSide,
};
use crate::macos_keychain::MacKeychainSecretStore;
use crate::{PlatformError, PlatformResult, SecretStore};
use core_foundation::base::{CFType, CFTypeRef, OSStatus, TCFType};
use core_foundation::boolean::CFBoolean;
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
    fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> OSStatus;
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
    static kSecAttrComment: CFStringRef;
    static kSecReturnAttributes: CFStringRef;
    static kSecReturnData: CFStringRef;
    static kSecMatchLimit: CFStringRef;
    static kSecMatchLimitOne: CFStringRef;
    static kSecUseAuthenticationUI: CFStringRef;
    static kSecUseAuthenticationUIFail: CFStringRef;
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

/// The keys and values of the silent attribute query, retained.
struct SilentKeys {
    class: CFString,
    generic_password: CFString,
    service: CFString,
    account: CFString,
    comment: CFString,
    return_attributes: CFString,
    /// Only the test reads it: the query must never contain it.
    #[cfg_attr(not(test), allow(dead_code))]
    return_data: CFString,
    match_limit: CFString,
    match_limit_one: CFString,
    authentication_ui: CFString,
    authentication_ui_fail: CFString,
}

fn silent_keys() -> SilentKeys {
    // SAFETY: the Security framework constants are valid, immutable `CFString`s for the life of
    // the process; reading them is a plain load and the get rule retains each one.
    unsafe {
        SilentKeys {
            class: CFString::wrap_under_get_rule(kSecClass),
            generic_password: CFString::wrap_under_get_rule(kSecClassGenericPassword),
            service: CFString::wrap_under_get_rule(kSecAttrService),
            account: CFString::wrap_under_get_rule(kSecAttrAccount),
            comment: CFString::wrap_under_get_rule(kSecAttrComment),
            return_attributes: CFString::wrap_under_get_rule(kSecReturnAttributes),
            return_data: CFString::wrap_under_get_rule(kSecReturnData),
            match_limit: CFString::wrap_under_get_rule(kSecMatchLimit),
            match_limit_one: CFString::wrap_under_get_rule(kSecMatchLimitOne),
            authentication_ui: CFString::wrap_under_get_rule(kSecUseAuthenticationUI),
            authentication_ui_fail: CFString::wrap_under_get_rule(kSecUseAuthenticationUIFail),
        }
    }
}

/// The silent lookup of one generic password: attributes only (never `kSecReturnData`), one
/// match, and no UI (`kSecUseAuthenticationUIFail`: an item that would need interaction fails
/// with `errSecInteractionNotAllowed` instead of prompting).
fn silent_attributes_query(service: &str, account: &str) -> CFDictionary<CFString, CFType> {
    let k = silent_keys();
    CFDictionary::from_CFType_pairs(&[
        (k.class, k.generic_password.as_CFType()),
        (k.service, CFString::new(service).as_CFType()),
        (k.account, CFString::new(account).as_CFType()),
        (k.return_attributes, CFBoolean::true_value().as_CFType()),
        (k.match_limit, k.match_limit_one.as_CFType()),
        (k.authentication_ui, k.authentication_ui_fail.as_CFType()),
    ])
}

/// Runs [`silent_attributes_query`]: `Ok(None)` when there is no such item, otherwise the item's
/// comment (`Ok(Some(None))` without one); `Err` carries the `OSStatus` (including
/// `errSecInteractionNotAllowed`), which the caller maps.
pub(crate) fn silent_item_attributes(
    service: &str,
    account: &str,
) -> Result<Option<Option<String>>, OSStatus> {
    const ERR_SEC_ITEM_NOT_FOUND: OSStatus = -25300;
    let query = silent_attributes_query(service, account);
    let mut result: CFTypeRef = ptr::null();
    // SAFETY: `query` is a live dictionary for the duration of the call; the out pointer is
    // valid and receives a +1 reference on success.
    let status = unsafe { SecItemCopyMatching(query.as_CFTypeRef(), &mut result) };
    if status == ERR_SEC_ITEM_NOT_FOUND {
        return Ok(None);
    }
    let result = owned(status, result, "silent attributes").map_err(|_| status)?;
    let attributes = result
        .downcast_into::<CFDictionary>()
        .ok_or(ERR_SEC_ITEM_NOT_FOUND)?;
    let comment_key = silent_keys().comment;
    let comment = attributes
        .find(comment_key.as_CFTypeRef())
        .map(|value| {
            // SAFETY: the dictionary holds a live CF object at this key; the get rule retains it
            // for the wrapper's lifetime.
            unsafe { CFType::wrap_under_get_rule(*value) }
        })
        .and_then(|value| value.downcast_into::<CFString>())
        .map(|text| text.to_string());
    Ok(Some(comment))
}

/// The designated requirement of `code` (a `SecCode` or `SecStaticCode`) as canonical text.
fn designated_requirement_text(code: CFTypeRef) -> PlatformResult<String> {
    let mut requirement: CFTypeRef = ptr::null();
    // SAFETY: `code` is a live code object owned by the caller; the out pointer is valid.
    let status = unsafe { SecCodeCopyDesignatedRequirement(code, 0, &mut requirement) };
    let requirement = owned(status, requirement, "designated requirement")?;
    requirement_text(requirement.as_CFTypeRef())
}

/// The canonical text of a compiled requirement (`SecRequirementCopyString`).
fn requirement_text(requirement: CFTypeRef) -> PlatformResult<String> {
    let mut text: CFTypeRef = ptr::null();
    // SAFETY: `requirement` is a live `SecRequirement` owned by the caller; the out pointer is
    // valid.
    let status = unsafe { SecRequirementCopyString(requirement, 0, &mut text) };
    let text = owned(status, text, "requirement text")?;
    text.downcast_into::<CFString>()
        .map(|s| s.to_string())
        .ok_or_else(|| PlatformError::Failed("requirement text is not a string".into()))
}

/// The canonical text of `requirement`: compiled by the Security framework
/// (`SecRequirementCreateWithString`) and printed back, so it compares exactly with the text of a
/// binary's designated requirement.
pub(crate) fn canonical_requirement(requirement: &str) -> PlatformResult<String> {
    let compiled = requirement
        .parse::<SecRequirement>()
        .map_err(|e| failed("requirement", e.code()))?;
    requirement_text(compiled.as_CFTypeRef())
}

/// Whether `actual` (a designated requirement as printed by the Security framework) is exactly
/// the requirement constant `expected`, compared in canonical form.
fn designated_requirement_is(actual: &str, expected: &str) -> PlatformResult<bool> {
    Ok(designated_requirement_is_exact(
        actual,
        &canonical_requirement(expected)?,
    ))
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

    /// The running binary and the peer both satisfy their team requirement, and the designated
    /// requirement each one carries (the one the access list records) is exactly that
    /// requirement (R27); otherwise nothing is written.
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
        if !designated_requirement_is(&own_designated_requirement()?, own_requirement)?
            || !designated_requirement_is(&designated_requirement_of(peer)?, peer_requirement)?
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

    #[test]
    fn silent_query_reads_attributes_only_and_never_shows_ui() {
        let query = silent_attributes_query("svc", "acct");
        let k = silent_keys();
        let value = |key: &CFString| query.find(key).map(|v| v.clone());
        assert_eq!(
            value(&k.return_attributes).and_then(|v| v.downcast::<CFBoolean>()),
            Some(CFBoolean::true_value())
        );
        assert!(
            value(&k.return_data).is_none(),
            "the query must not ask for data"
        );
        assert_eq!(
            value(&k.authentication_ui).and_then(|v| v.downcast::<CFString>()),
            Some(k.authentication_ui_fail.clone())
        );
        assert_eq!(
            value(&k.match_limit).and_then(|v| v.downcast::<CFString>()),
            Some(k.match_limit_one.clone())
        );
        assert_eq!(
            value(&k.service).and_then(|v| v.downcast::<CFString>()),
            Some(CFString::new("svc"))
        );
        assert_eq!(
            value(&k.account).and_then(|v| v.downcast::<CFString>()),
            Some(CFString::new("acct"))
        );
        assert_eq!(query.len(), 6);
    }

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
        assert!(!designated_requirement_is(&text, APP_REQUIREMENT).unwrap());
        assert!(!designated_requirement_is(&text, HOST_REQUIREMENT).unwrap());
        // Its own designated requirement does compare equal with itself in canonical form.
        assert!(designated_requirement_is(&text, &text).unwrap());
    }

    #[test]
    fn the_required_expressions_compare_equal_in_canonical_form() {
        for (expected, id) in [
            (APP_REQUIREMENT, "com.mypass.app"),
            (HOST_REQUIREMENT, "com.mypass.chromehost"),
        ] {
            let canonical = canonical_requirement(expected).unwrap();
            assert!(designated_requirement_is(&canonical, expected).unwrap());
            // Spacing and quoting are the framework's business: the same expression written
            // differently has the same canonical text.
            let respelled = format!(
                "identifier {id}   and anchor apple generic and certificate \
                 leaf[subject.OU] = {TEAM_ID}"
            );
            let respelled = canonical_requirement(&respelled).unwrap();
            assert!(designated_requirement_is(&respelled, expected).unwrap());
        }
    }

    #[test]
    fn near_miss_designated_requirements_are_refused() {
        let app = APP_REQUIREMENT;
        let near_misses = [
            // A broadened requirement: trusts any Apple-signed code.
            format!("{app} or anchor apple"),
            format!("({app}) or identifier \"com.evil.app\""),
            // A certificate-name pin (the default of an Apple Development certificate).
            "identifier \"com.mypass.app\" and anchor apple generic and certificate \
             leaf[subject.CN] = \"Apple Development: Example (W645B93PPQ)\""
                .to_owned(),
            format!("{app} and certificate leaf[subject.CN] = \"Apple Development: X\""),
            // The Developer ID default: not the explicit R27 requirement.
            "identifier \"com.mypass.app\" and anchor apple generic and certificate \
             1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate \
             leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate \
             leaf[subject.OU] = W645B93PPQ"
                .to_owned(),
            app.replace("com.mypass.app", "com.mypass.chromehost"),
            app.replace("com.mypass.app", "com.mypass.app2"),
            app.replace(TEAM_ID, "ABCDE12345"),
            app.replace("anchor apple generic and ", ""),
            app.replace("anchor apple generic", "anchor trusted"),
            format!("{app} and !anchor apple"),
            "cdhash H\"0123456789abcdef0123456789abcdef01234567\"".to_owned(),
        ];
        for near_miss in &near_misses {
            let canonical = canonical_requirement(near_miss)
                .unwrap_or_else(|e| panic!("{near_miss} does not compile: {e}"));
            assert!(
                !designated_requirement_is(&canonical, APP_REQUIREMENT).unwrap(),
                "{canonical}"
            );
        }
        assert!(canonical_requirement("not a requirement (").is_err());
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
        let record = item_record(&dump, &service, "loopback").expect("the item is in the dump");
        assert_acl_trusts_exactly_app_and_host(record);
    }

    /// The record of the one item with exactly this service and account in the output of
    /// `security dump-keychain -a` (every item starts with a `keychain: ` line), so neighbouring
    /// items can never answer for it.
    fn item_record<'a>(dump: &'a str, service: &str, account: &str) -> Option<&'a str> {
        let service_line = format!("\"svce\"<blob>=\"{service}\"");
        let account_line = format!("\"acct\"<blob>=\"{account}\"");
        let mut starts: Vec<usize> = dump
            .match_indices("keychain: ")
            .filter(|(i, _)| *i == 0 || dump.get(i - 1..*i) == Some("\n"))
            .map(|(i, _)| i)
            .collect();
        starts.push(dump.len());
        let mut matching = starts
            .windows(2)
            .filter_map(|w| dump.get(*w.first()?..*w.get(1)?))
            .filter(|record| {
                let has = |wanted: &str| record.lines().any(|l| l.trim() == wanted);
                has(&service_line) && has(&account_line)
            });
        let record = matching.next()?;
        matching.next().is_none().then_some(record)
    }

    /// The `requirement:` lines of the access-list entries that may decrypt (read) the item.
    fn decrypt_requirements(record: &str) -> Vec<String> {
        let Some((_, access)) = record.split_once("\naccess:") else {
            return Vec::new();
        };
        access
            .split("entry ")
            .filter(|entry| {
                entry
                    .lines()
                    .any(|l| l.trim_start().starts_with("authorizations") && l.contains("decrypt"))
            })
            .flat_map(|entry| {
                entry
                    .lines()
                    .filter_map(|l| l.trim().strip_prefix("requirement: "))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The item's read access trusts exactly the app and the host by their R27 requirements.
    fn assert_acl_trusts_exactly_app_and_host(record: &str) {
        let mut found = decrypt_requirements(record);
        found.sort();
        let mut expected = vec![
            canonical_requirement(APP_REQUIREMENT).unwrap(),
            canonical_requirement(HOST_REQUIREMENT).unwrap(),
        ];
        expected.sort();
        assert_eq!(found, expected, "{record}");
        assert!(!record.contains("cdhash"), "{record}");
        assert!(!record.contains("subject.CN"), "{record}");
    }

    #[test]
    fn dump_parser_isolates_the_item_from_its_neighbours() {
        let app = canonical_requirement(APP_REQUIREMENT).unwrap();
        let host = canonical_requirement(HOST_REQUIREMENT).unwrap();
        let record = |service: &str, requirements: &[&str]| {
            let apps: String = requirements
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    format!(
                        "            {i}: /Applications/x (OK)\n                requirement: {r}\n"
                    )
                })
                .collect();
            format!(
                "keychain: \"/Users/u/Library/Keychains/login.keychain-db\"\nversion: 512\n\
                 class: \"genp\"\nattributes:\n    \"acct\"<blob>=\"loopback\"\n    \
                 \"svce\"<blob>=\"{service}\"\naccess: 2 entries\n    entry 0:\n        \
                 authorizations (6): decrypt derive export_clear export_wrapped mac sign\n        \
                 description: MyPass browser bridge endpoint\n        applications ({}):\n{apps}    \
                 entry 1:\n        authorizations (1): encrypt\n        applications: <null>\n",
                requirements.len()
            )
        };
        let foreign_team =
            format!("identifier \"com.other\" and certificate leaf[subject.OU] = {TEAM_ID}");
        let neighbour_cdhash = "cdhash H\"0123456789abcdef0123456789abcdef01234567\"";
        let ours = record("com.mypass.tests.endpoint.1", &[&app, &host]);
        let dump = [
            record("com.mypass.tests.endpoint.10", &[neighbour_cdhash]),
            ours.clone(),
            record("com.mypass.other", &[&foreign_team]),
        ]
        .concat();
        let found = item_record(&dump, "com.mypass.tests.endpoint.1", "loopback").unwrap();
        assert_eq!(found, ours);
        assert_acl_trusts_exactly_app_and_host(found);
        assert!(item_record(&dump, "com.mypass.tests.endpoint", "loopback").is_none());
        assert!(item_record(&dump, "com.mypass.tests.endpoint.1", "other").is_none());

        // A record that trusts only the app, or a CN pin, or one more application, fails.
        for bad in [
            record("svc", &[&app]),
            record("svc", &[&app, &host, &foreign_team]),
            record(
                "svc",
                &[
                    &app,
                    "identifier \"com.mypass.chromehost\" and certificate leaf[subject.CN] = \"X\"",
                ],
            ),
        ] {
            let result = std::panic::catch_unwind(|| assert_acl_trusts_exactly_app_and_host(&bad));
            assert!(result.is_err(), "{bad}");
        }
    }
}
