//! Listener authentication for the signed native host. Unsigned development skips this check.
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
