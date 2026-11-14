//! Which store the bridge endpoint record lives in (stage 5, ruling R4): the Keychain item that
//! trusts only the team-signed app and host, or the hardened 0600 file store.
//!
//! The endpoint record (port and token of the loopback server) is written by the app and read by
//! the native messaging host. In the file store any process of the same user can rewrite it; the
//! Keychain item instead carries an access list that trusts exactly two code requirements, the
//! app's and the host's, both pinned to the team `W645B93PPQ`. Never a path or a cdhash: the item
//! keeps working across updates and refuses a binary that merely sits at the right path.
//!
//! Both sides must pick the same store, so both use one predicate: the Keychain is used only when
//! the running binary satisfies its own team requirement **and** the peer binary on disk
//! satisfies the peer's. An unsigned development build (or a signed host next to an unsigned app)
//! falls back to the file store on both sides. The decision is pure ([`select_shared_store`]) and
//! tested with a fake [`SignatureProbe`]; the macOS probe and the Keychain store live in
//! `macos_keychain_acl.rs`.
//!
//! The app switch-over (writing the endpoint here, deleting the old file record) and locating the
//! peer binary are the app's and the host's (batch H); this module only answers "which store".

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
    /// The hardened 0600 file store under `AppPaths::secrets_shared`.
    File,
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

/// The one predicate both sides use: the Keychain only when this binary and the peer binary at
/// `peer` are both team-signed with the expected identifiers; otherwise (no peer found, either one
/// unsigned or ad-hoc signed) the file store.
pub fn select_shared_store(
    probe: &dyn SignatureProbe,
    side: StoreSide,
    peer: Option<&Path>,
) -> SharedStoreKind {
    let (own, peer_requirement) = side.requirements();
    match peer {
        Some(peer)
            if probe.running_binary_satisfies(own)
                && probe.binary_satisfies(peer, peer_requirement) =>
        {
            SharedStoreKind::Keychain
        }
        _ => SharedStoreKind::File,
    }
}

/// The endpoint store of the app; `host_exe` is the bundled host binary.
pub fn app_shared_store(
    paths: &AppPaths,
    host_exe: Option<&Path>,
) -> (SharedStoreKind, Arc<dyn SecretStore>) {
    shared_store_for(paths, StoreSide::App, host_exe)
}

/// The endpoint store of the native messaging host; `app_exe` is the app binary of the bundle
/// the host belongs to.
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
    if let (SharedStoreKind::Keychain, Some(peer)) = (
        select_shared_store(&crate::macos_keychain_acl::MacSignatureProbe, side, peer),
        peer,
    ) {
        let store =
            crate::macos_keychain_acl::MacEndpointKeychainStore::new(peer.to_path_buf(), side);
        return (SharedStoreKind::Keychain, Arc::new(store));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (side, peer);
    }
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
/// Whether a designated requirement (as `SecRequirementCopyString` prints it) pins the code by
/// team and identifier rather than by cdhash: the access list records the peer's designated
/// requirement, so a cdhash one would break on every update and is refused.
pub(crate) fn designated_requirement_is_team_based(text: &str, identifier: &str) -> bool {
    !text.contains("cdhash")
        && text.contains("anchor apple generic")
        && text.contains(TEAM_ID)
        && (text.contains(&format!("identifier \"{identifier}\""))
            || text.contains(&format!("identifier {identifier} ")))
}

#[cfg(any(target_os = "macos", test))]
/// The identifier a requirement constant names (`APP_REQUIREMENT` → `com.mypass.app`).
pub(crate) fn requirement_identifier(requirement: &str) -> Option<&str> {
    requirement
        .strip_prefix("identifier \"")?
        .split('"')
        .next()
        .filter(|id| !id.is_empty())
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
        assert_eq!(requirement_identifier(APP_REQUIREMENT), Some(BUNDLE_ID));
        assert_eq!(
            requirement_identifier(HOST_REQUIREMENT),
            Some(HOST_IDENTIFIER)
        );
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
        // A signed host next to an unsigned (development) app: both sides must pick the file.
        let only_host_signed = TableProbe {
            own: vec![HOST_REQUIREMENT],
            on_disk: vec![(host(), HOST_REQUIREMENT)],
        };
        // The host was swapped for another binary at the same path.
        let host_swapped = TableProbe {
            own: vec![APP_REQUIREMENT],
            on_disk: vec![],
        };
        use SharedStoreKind::{File, Keychain};
        use StoreSide::{App, Host};
        let cases: [(&TableProbe, StoreSide, Option<PathBuf>, SharedStoreKind); 9] = [
            (&both_signed, App, Some(host()), Keychain),
            (&both_signed, Host, Some(app()), Keychain),
            (&both_signed, App, None, File),
            (&both_signed, App, Some(app()), File),
            (&unsigned, App, Some(host()), File),
            (&unsigned, Host, Some(app()), File),
            (&only_host_signed, App, Some(host()), File),
            (&only_host_signed, Host, Some(app()), File),
            (&host_swapped, App, Some(host()), File),
        ];
        for (i, (probe, side, peer, expected)) in cases.into_iter().enumerate() {
            assert_eq!(
                select_shared_store(probe, side, peer.as_deref()),
                expected,
                "case {i}"
            );
        }
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
    fn designated_requirement_check() {
        let developer_id = "identifier \"com.mypass.chromehost\" and anchor apple generic and \
            certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate \
            leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate \
            leaf[subject.OU] = W645B93PPQ";
        assert!(designated_requirement_is_team_based(
            developer_id,
            HOST_IDENTIFIER
        ));
        assert!(!designated_requirement_is_team_based(
            developer_id,
            BUNDLE_ID
        ));
        assert!(!designated_requirement_is_team_based(
            "cdhash H\"0123456789abcdef0123456789abcdef01234567\"",
            HOST_IDENTIFIER
        ));
        let other_team = developer_id.replace(TEAM_ID, "ABCDE12345");
        assert!(!designated_requirement_is_team_based(
            &other_team,
            HOST_IDENTIFIER
        ));
    }

    #[cfg(unix)]
    #[test]
    fn without_a_peer_both_sides_use_the_file_store() {
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
