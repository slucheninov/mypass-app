//! Endpoint listener trust: macOS code requirements, Windows PID/current-user ownership.
use crate::endpoint_store::APP_REQUIREMENT;
pub trait PeerProcessProbe {
    fn listener_pid(&self, port: u16) -> Option<i32>;
    fn process_satisfies(&self, pid: i32, requirement: &str) -> bool;
}
pub fn verify_listener(probe: &dyn PeerProcessProbe, port: u16) -> bool {
    port != 0
        && probe
            .listener_pid(port)
            .is_some_and(|pid| pid > 0 && probe.process_satisfies(pid, APP_REQUIREMENT))
}
pub fn signed_app_listens_on(port: u16) -> bool {
    #[cfg(target_os = "macos")]
    {
        verify_listener(&crate::macos_peer_process::MacPeerProcessProbe, port)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = port;
        false
    }
}
/// Unsigned Windows checks process ownership, not publisher identity. PID/port reuse between
/// lookup and verification, and same-user endpoint rewriting remain residual risks. HMAC still
/// authenticates the listener before any token-bearing request is sent.
pub trait WindowsListenerProbe {
    /// Exactly one IPv4 127.0.0.1 LISTEN row at this port; missing/ambiguous/failure => None.
    fn listener_pid(&self, port: u16) -> Option<u32>;
    /// Accessible live process whose token user SID equals this process's token user SID.
    fn current_user_owns_live_process(&self, pid: u32) -> bool;
}
pub fn verify_windows_listener(
    probe: &dyn WindowsListenerProbe,
    port: u16,
    expected_pid: u32,
) -> bool {
    port != 0
        && expected_pid != 0
        && probe.listener_pid(port) == Some(expected_pid)
        && probe.current_user_owns_live_process(expected_pid)
}
/// Platform/store-specific peer policy; used only after connecting, before hello/token traffic.
pub fn endpoint_peer_is_trusted(kind: crate::SharedStoreKind, port: u16, pid: u32) -> bool {
    match kind {
        crate::SharedStoreKind::File => cfg!(not(windows)),
        crate::SharedStoreKind::Keychain => signed_app_listens_on(port),
        crate::SharedStoreKind::Dpapi => {
            #[cfg(windows)]
            return verify_windows_listener(&crate::windows::WindowsListenerProbe, port, pid);
            #[cfg(not(windows))]
            {
                let _ = (port, pid);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake(Option<i32>, bool);
    impl PeerProcessProbe for Fake {
        fn listener_pid(&self, _: u16) -> Option<i32> {
            self.0
        }
        fn process_satisfies(&self, pid: i32, requirement: &str) -> bool {
            assert_eq!(pid, 42);
            assert_eq!(requirement, APP_REQUIREMENT);
            self.1
        }
    }
    #[test]
    fn missing_foreign_or_invalid_peer_fails_closed() {
        for f in [
            Fake(None, true),
            Fake(Some(-1), true),
            Fake(Some(42), false),
        ] {
            assert!(!verify_listener(&f, 1234));
        }
        assert!(verify_listener(&Fake(Some(42), true), 1234));
        assert!(!verify_listener(&Fake(Some(42), true), 0));
    }
}

#[cfg(test)]
mod windows_policy_tests {
    use super::*;
    struct Probe(Option<u32>, bool);
    impl WindowsListenerProbe for Probe {
        fn listener_pid(&self, _: u16) -> Option<u32> {
            self.0
        }
        fn current_user_owns_live_process(&self, _: u32) -> bool {
            self.1
        }
    }
    #[test]
    fn windows_dpapi_requires_positive_matching_pid_and_current_user() {
        for (port, pid, actual, owned) in [
            (0, 42, Some(42), true),
            (123, 0, Some(42), true),
            (123, 41, Some(42), true),
            (123, 42, None, true),
            (123, 42, Some(42), false),
        ] {
            assert!(!verify_windows_listener(&Probe(actual, owned), port, pid));
        }
        assert!(verify_windows_listener(&Probe(Some(42), true), 123, 42));
    }
    #[test]
    fn windows_selection_does_not_depend_on_apple_signatures() {
        use crate::endpoint_store::*;
        struct NoApple;
        impl SignatureProbe for NoApple {
            fn running_binary_satisfies(&self, _: &str) -> bool {
                panic!("Windows used Apple probe")
            }
            fn binary_satisfies(&self, _: &std::path::Path, _: &str) -> bool {
                panic!("Windows used Apple probe")
            }
        }
        for side in [StoreSide::App, StoreSide::Host] {
            assert_eq!(
                select_store_for(StorePlatform::Windows, &NoApple, side),
                SharedStoreKind::Dpapi
            );
        }
        assert_eq!(SharedStoreKind::Dpapi.diagnostic_name(), "dpapi");
        assert!(SharedStoreKind::Dpapi.can_reinstall_host());
    }
}
