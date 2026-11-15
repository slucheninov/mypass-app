//! Windows always uses current-user DPAPI files, without a plaintext fallback.
//! On macOS, the bridge endpoint lives in Keychain for team-signed binaries and the hardened
//! 0600 file store for unsigned development binaries.
//!
//! The endpoint record (port and token of the loopback server) is written by the app and read by
//! the native messaging host. In the file store any process of the same user can rewrite it; the
//! Keychain item instead carries an access list that trusts exactly two code requirements, the
//! app's and the host's, both pinned to the team `W645B93PPQ`. Never a path or a cdhash: the item
//! keeps working across updates and refuses a binary that merely sits at the right path.
//!
//! The macOS choice fails closed: a binary that satisfies its own team requirement always uses the
//! Keychain, never the file store. When its peer is missing or not team-signed, writing the record
//! fails (the bridge does not start) instead of downgrading to a store any process can rewrite.
//! Only an unsigned or ad-hoc signed development binary uses the file store, so development runs
//! sign both binaries or neither (a signed host next to an unsigned app cannot reach it). The
//! decision is pure ([`select_shared_store`]) and tested with a fake [`SignatureProbe`]; the macOS
//! probe and the Keychain store live in `macos_keychain_acl.rs`.
//!
//! Windows encrypted publication replaces the exact legacy filename; no legacy plaintext is
//! read or migrated. Only macOS Keychain publication removes a separate legacy file.

use crate::{AppPaths, SecretStore};
#[cfg(any(target_os = "macos", test))]
use crate::{PlatformError, PlatformResult};
use std::path::Path;
use std::sync::Arc;

/// The Developer ID team of every MyPass binary.
pub const TEAM_ID: &str = "W645B93PPQ";

/// The code-signing identifier of the native messaging host.
pub const HOST_IDENTIFIER: &str = "com.mypass.chromehost";

/// The code requirement of the app: its identifier, an Apple-issued chain, our team.
pub const APP_REQUIREMENT: &str = concat!(
    "identifier \"com.mypass.app\" and anchor apple generic and ",
    "certificate leaf[subject.OU] = \"W645B93PPQ\""
);

/// The code requirement of the native messaging host.
pub const HOST_REQUIREMENT: &str = concat!(
    "identifier \"com.mypass.chromehost\" and anchor apple generic and ",
    "certificate leaf[subject.OU] = \"W645B93PPQ\""
);

/// Answers code-signing questions about binaries. The macOS body uses the Security framework;
/// tests use a table.
pub trait SignatureProbe {
    /// Whether the running process satisfies `requirement`.
    fn running_binary_satisfies(&self, requirement: &str) -> bool;

    /// Whether the binary at `path` (on disk, statically) satisfies `requirement`.
    fn binary_satisfies(&self, path: &Path, requirement: &str) -> bool;
}

/// Where the endpoint record lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedStoreKind {
    /// The Keychain item trusting the team-signed app and host.
    Keychain,
    /// Current-user DPAPI encrypted files on Windows; no plaintext fallback.
    Dpapi,
    /// The hardened 0600 file store under `AppPaths::secrets_shared`.
    File,
}

impl SharedStoreKind {
    pub fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Keychain => "keychain",
            Self::Dpapi => "dpapi",
            Self::File => "developmentFile",
        }
    }
    pub fn can_reinstall_host(self) -> bool {
        matches!(self, Self::File | Self::Dpapi)
    }
}
#[derive(Clone, Copy)]
pub enum StorePlatform {
    Macos,
    Windows,
    Other,
}
/// Injected platform policy: Windows never consults Apple signatures or chooses File.
pub fn select_store_for(
    platform: StorePlatform,
    probe: &dyn SignatureProbe,
    side: StoreSide,
) -> SharedStoreKind {
    match platform {
        StorePlatform::Windows => SharedStoreKind::Dpapi,
        StorePlatform::Macos => {
            let (own, _) = side.requirements();
            if probe.running_binary_satisfies(own) {
                SharedStoreKind::Keychain
            } else {
                SharedStoreKind::File
            }
        }
        StorePlatform::Other => SharedStoreKind::File,
    }
}

/// Which binary is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreSide {
    App,
    Host,
}

impl StoreSide {
    /// (own requirement, peer requirement).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn requirements(self) -> (&'static str, &'static str) {
        match self {
            Self::App => (APP_REQUIREMENT, HOST_REQUIREMENT),
            Self::Host => (HOST_REQUIREMENT, APP_REQUIREMENT),
        }
    }
}

/// Both sides choose DPAPI unconditionally on Windows. On macOS, Keychain whenever team-signed with its
/// expected identifier (fail closed: the peer is checked when the record is written, and a
/// missing or foreign peer makes that write fail rather than choosing the file store); the file
/// store only for an unsigned or ad-hoc signed development binary.
pub fn select_shared_store(probe: &dyn SignatureProbe, side: StoreSide) -> SharedStoreKind {
    let platform = if cfg!(windows) {
        StorePlatform::Windows
    } else if cfg!(target_os = "macos") {
        StorePlatform::Macos
    } else {
        StorePlatform::Other
    };
    select_store_for(platform, probe, side)
}

/// The only host path accepted by the signed app: a sibling inside its own app bundle.
/// A signed standalone executable must fail closed, rather than register a writable copy.
pub fn signed_bundle_host(app_exe: &Path) -> Option<std::path::PathBuf> {
    let macos = app_exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    if macos.file_name()? != "MacOS"
        || contents.file_name()? != "Contents"
        || bundle.extension()? != "app"
    {
        return None;
    }
    Some(macos.join("mypass-native-host"))
}

/// The endpoint store of the app; `host_exe` is the bundled host binary, trusted next to the app
/// (`None`: not found, so a signed app cannot write the record).
pub fn app_shared_store(
    paths: &AppPaths,
    host_exe: Option<&Path>,
) -> (SharedStoreKind, Arc<dyn SecretStore>) {
    shared_store_for(paths, StoreSide::App, host_exe)
}

/// The endpoint store of the native messaging host; `app_exe` is the app binary of the bundle
/// the host belongs to (only needed if the host ever writes the record).
pub fn host_shared_store(
    paths: &AppPaths,
    app_exe: Option<&Path>,
) -> (SharedStoreKind, Arc<dyn SecretStore>) {
    shared_store_for(paths, StoreSide::Host, app_exe)
}

fn shared_store_for(
    paths: &AppPaths,
    side: StoreSide,
    peer: Option<&Path>,
) -> (SharedStoreKind, Arc<dyn SecretStore>) {
    #[cfg(target_os = "macos")]
    if select_shared_store(&crate::macos_keychain_acl::MacSignatureProbe, side)
        == SharedStoreKind::Keychain
    {
        let store = crate::macos_keychain_acl::MacEndpointKeychainStore::new(
            peer.map(Path::to_path_buf),
            side,
        );
        return (SharedStoreKind::Keychain, Arc::new(store));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (side, peer);
    }
    #[cfg(windows)]
    return (
        SharedStoreKind::Dpapi,
        Arc::new(crate::DpapiSecretStore::shared(paths)),
    );
    #[cfg(not(windows))]
    (SharedStoreKind::File, Arc::new(crate::shared_store(paths)))
}

#[cfg(any(target_os = "macos", test))]
/// `errSecSuccess`.
pub(crate) const ERR_SEC_SUCCESS: i32 = 0;
#[cfg(any(target_os = "macos", test))]
/// `errSecDuplicateItem`.
pub(crate) const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
#[cfg(any(target_os = "macos", test))]
/// How many adds a write tries before it reports a concurrent writer or tampering.
pub(crate) const MAX_ADD_ATTEMPTS: usize = 3;

#[cfg(any(target_os = "macos", test))]
/// The two Keychain operations of a write, so the order is tested without the Keychain.
pub(crate) trait ItemOps {
    /// Deletes the item; "not found" is `Ok`.
    fn delete(&self) -> PlatformResult<()>;
    /// One add-only `SecItemAdd` with the trusted access list; returns the `OSStatus`.
    fn add(&self) -> i32;
}

#[cfg(any(target_os = "macos", test))]
/// Delete, then add. The item is never updated: an update keeps the existing access list, and an
/// item planted by another process may trust anyone. A duplicate after the delete means someone
/// re-planted it in between: delete and add again, at most [`MAX_ADD_ATTEMPTS`] adds, then report
/// [`PlatformError::SecretStoreTampered`].
pub(crate) fn delete_then_add(ops: &dyn ItemOps) -> PlatformResult<()> {
    ops.delete()?;
    for attempt in 1..=MAX_ADD_ATTEMPTS {
        match ops.add() {
            ERR_SEC_SUCCESS => return Ok(()),
            ERR_SEC_DUPLICATE_ITEM if attempt < MAX_ADD_ATTEMPTS => ops.delete()?,
            ERR_SEC_DUPLICATE_ITEM => break,
            code => return Err(PlatformError::Failed(format!("keychain status {code}"))),
        }
    }
    Err(PlatformError::SecretStoreTampered)
}

#[cfg(any(target_os = "macos", test))]
/// Whether a designated requirement is exactly the expected one (R27): both arguments are
/// canonical texts, as `SecRequirementCopyString` prints a compiled requirement (the macOS side
/// compiles the expected constant and prints it back, so quoting and spacing never differ). The
/// access list records the binary's own designated requirement, so anything but this exact
/// expression is refused: an extra `or` branch (which trusts unrelated code), a `subject.CN` pin
/// (which breaks on certificate renewal), a `cdhash` (which breaks on every update), another
/// identifier or team, a negation or a missing anchor. Never a substring test.
pub(crate) fn designated_requirement_is_exact(
    actual_canonical: &str,
    expected_canonical: &str,
) -> bool {
    !expected_canonical.trim().is_empty() && actual_canonical == expected_canonical
}

/// Store selection without opening Keychain or reading an endpoint (diagnostics).
pub fn running_store_kind(side: StoreSide) -> SharedStoreKind {
    #[cfg(target_os = "macos")]
    {
        select_shared_store(&crate::macos_keychain_acl::MacSignatureProbe, side)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = side;
        #[cfg(windows)]
        return SharedStoreKind::Dpapi;
        #[cfg(not(windows))]
        SharedStoreKind::File
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BUNDLE_ID;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;

    struct TableProbe {
        own: Vec<&'static str>,
        on_disk: Vec<(PathBuf, &'static str)>,
    }

    impl SignatureProbe for TableProbe {
        fn running_binary_satisfies(&self, requirement: &str) -> bool {
            self.own.contains(&requirement)
        }
        fn binary_satisfies(&self, path: &Path, requirement: &str) -> bool {
            self.on_disk
                .iter()
                .any(|(p, r)| p == path && *r == requirement)
        }
    }

    fn host() -> PathBuf {
        PathBuf::from("/Applications/MyPass.app/Contents/MacOS/mypass-host")
    }

    fn app() -> PathBuf {
        PathBuf::from("/Applications/MyPass.app/Contents/MacOS/mypass-app")
    }

    #[test]
    fn signed_host_path_is_confined_to_the_running_bundle() {
        assert_eq!(
            signed_bundle_host(Path::new(
                "/Applications/MyPass.app/Contents/MacOS/mypass-app"
            )),
            Some("/Applications/MyPass.app/Contents/MacOS/mypass-native-host".into())
        );
        for p in [
            "/target/debug/mypass-app",
            "/Applications/MyPass.app/mypass-app",
            "/Applications/MyPass.app/Contents/mypass-app",
            "/tmp/Other/Contents/MacOS/mypass-app",
        ] {
            assert!(signed_bundle_host(Path::new(p)).is_none());
        }
    }

    #[test]
    fn requirements_pin_identifier_anchor_and_team() {
        assert_eq!(
            APP_REQUIREMENT,
            "identifier \"com.mypass.app\" and anchor apple generic and certificate \
             leaf[subject.OU] = \"W645B93PPQ\""
        );
        assert_eq!(
            HOST_REQUIREMENT,
            "identifier \"com.mypass.chromehost\" and anchor apple generic and certificate \
             leaf[subject.OU] = \"W645B93PPQ\""
        );
        assert!(APP_REQUIREMENT.starts_with(&format!("identifier \"{BUNDLE_ID}\" ")));
        assert!(HOST_REQUIREMENT.starts_with(&format!("identifier \"{HOST_IDENTIFIER}\" ")));
        for r in [APP_REQUIREMENT, HOST_REQUIREMENT] {
            assert!(!r.contains("cdhash") && r.contains(TEAM_ID));
        }
    }

    #[test]
    fn selection_table() {
        let both_signed = TableProbe {
            own: vec![APP_REQUIREMENT, HOST_REQUIREMENT],
            on_disk: vec![(host(), HOST_REQUIREMENT), (app(), APP_REQUIREMENT)],
        };
        let unsigned = TableProbe {
            own: vec![],
            on_disk: vec![],
        };
        // A signed host next to an unsigned (development) app: they disagree, and the bridge
        // fails rather than letting the signed host trust a rewritable file.
        let only_host_signed = TableProbe {
            own: vec![HOST_REQUIREMENT],
            on_disk: vec![(host(), HOST_REQUIREMENT)],
        };
        // A signed app whose host was swapped: still the Keychain (the write then fails).
        let host_swapped = TableProbe {
            own: vec![APP_REQUIREMENT],
            on_disk: vec![],
        };
        use SharedStoreKind::{File, Keychain};
        use StoreSide::{App, Host};
        let cases: [(&TableProbe, StoreSide, SharedStoreKind); 6] = [
            (&both_signed, App, Keychain),
            (&both_signed, Host, Keychain),
            (&unsigned, App, File),
            (&unsigned, Host, File),
            (&only_host_signed, Host, Keychain),
            (&host_swapped, App, Keychain),
        ];
        for (i, (probe, side, expected)) in cases.into_iter().enumerate() {
            assert_eq!(
                select_store_for(StorePlatform::Macos, probe, side),
                expected,
                "case {i}"
            );
        }
        // The app requirement never satisfies the host side and the other way round.
        assert_eq!(
            select_store_for(StorePlatform::Macos, &only_host_signed, App),
            File
        );
        assert_eq!(
            select_store_for(StorePlatform::Macos, &host_swapped, Host),
            File
        );
        assert!(!both_signed.binary_satisfies(&app(), HOST_REQUIREMENT));
    }

    struct FakeOps {
        log: RefCell<Vec<&'static str>>,
        adds: RefCell<Vec<i32>>,
        fail_delete: Cell<bool>,
    }

    impl FakeOps {
        fn new(adds: &[i32]) -> Self {
            Self {
                log: RefCell::new(Vec::new()),
                adds: RefCell::new(adds.iter().rev().copied().collect()),
                fail_delete: Cell::new(false),
            }
        }
    }

    impl ItemOps for FakeOps {
        fn delete(&self) -> PlatformResult<()> {
            self.log.borrow_mut().push("delete");
            if self.fail_delete.get() {
                Err(PlatformError::Failed("keychain status -1".into()))
            } else {
                Ok(())
            }
        }
        fn add(&self) -> i32 {
            self.log.borrow_mut().push("add");
            self.adds.borrow_mut().pop().unwrap_or(ERR_SEC_SUCCESS)
        }
    }

    #[test]
    fn write_deletes_first_then_adds() {
        let ops = FakeOps::new(&[ERR_SEC_SUCCESS]);
        delete_then_add(&ops).unwrap();
        assert_eq!(*ops.log.borrow(), ["delete", "add"]);
    }

    #[test]
    fn replanted_item_is_deleted_again_then_tampering_is_reported() {
        let ops = FakeOps::new(&[ERR_SEC_DUPLICATE_ITEM, ERR_SEC_SUCCESS]);
        delete_then_add(&ops).unwrap();
        assert_eq!(*ops.log.borrow(), ["delete", "add", "delete", "add"]);

        let ops = FakeOps::new(&[ERR_SEC_DUPLICATE_ITEM; 3]);
        assert_eq!(
            delete_then_add(&ops).unwrap_err(),
            PlatformError::SecretStoreTampered
        );
        assert_eq!(
            *ops.log.borrow(),
            ["delete", "add", "delete", "add", "delete", "add"]
        );
    }

    #[test]
    fn other_add_status_and_delete_failure_stop_the_write() {
        let ops = FakeOps::new(&[-25308]);
        assert_eq!(
            delete_then_add(&ops).unwrap_err(),
            PlatformError::Failed("keychain status -25308".into())
        );
        let ops = FakeOps::new(&[]);
        ops.fail_delete.set(true);
        assert!(delete_then_add(&ops).is_err());
        assert_eq!(*ops.log.borrow(), ["delete"]);
    }

    #[test]
    fn designated_requirement_must_equal_the_expected_text() {
        // The canonical texts come from the Security framework on macOS (tested there with the
        // real compiler); here the comparison itself: equality, never containment.
        let expected = "identifier \"com.mypass.chromehost\" and anchor apple generic and \
            certificate leaf[subject.OU] = W645B93PPQ";
        assert!(designated_requirement_is_exact(expected, expected));
        for near_miss in [
            format!("{expected} or anchor apple"),
            format!("({expected}) or identifier \"com.evil\""),
            format!("{expected} and certificate leaf[subject.CN] = \"Apple Development: X\""),
            expected.replace("com.mypass.chromehost", "com.mypass.app"),
            expected.replace(TEAM_ID, "ABCDE12345"),
            expected.replace(" and anchor apple generic", ""),
            format!("{expected} "),
            String::new(),
        ] {
            assert!(
                !designated_requirement_is_exact(&near_miss, expected),
                "{near_miss}"
            );
        }
        assert!(!designated_requirement_is_exact("", ""));
        assert!(!designated_requirement_is_exact(" ", " "));
    }

    #[cfg(unix)]
    #[test]
    fn unsigned_test_binary_uses_the_file_store_on_both_sides() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::macos(tmp.path());
        let (kind, store) = app_shared_store(&paths, None);
        assert_eq!(kind, SharedStoreKind::File);
        store.set("svc", "acct", b"endpoint").unwrap();
        let (kind, store) = host_shared_store(&paths, None);
        assert_eq!(kind, SharedStoreKind::File);
        assert_eq!(
            store.get("svc", "acct").unwrap(),
            Some(b"endpoint".to_vec())
        );
    }
}
