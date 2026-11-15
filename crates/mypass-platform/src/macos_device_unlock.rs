//! macOS device unlock: the `LAContext` side of [`KeychainDeviceUnlock`] (the decisions and the
//! item layout are documented in `keychain_device_unlock.rs`).
//!
//! # Threads and lifetimes
//!
//! `evaluatePolicy:localizedReason:reply:` does not block; its reply block runs on a private
//! LocalAuthentication queue. [`LaContextAuth::evaluate`] keeps the context alive on its own stack
//! until the reply arrives (ARC would cancel an evaluation whose context is deallocated, the old
//! app's comment), waits on a channel with a ceiling, and on timeout invalidates the context,
//! which dismisses the prompt and fires the reply with `appCancel` (reported as `Cancelled`). The
//! waiting thread must not be the main thread: the prompt and the reply may need the main run
//! loop, so a call from the main thread is refused instead of risking a deadlock.
//!
//! One `LAContext` is created per call and never reused, so no previous evaluation can be
//! replayed (`touchIDAuthenticationAllowableReuseDuration` stays 0).

// `unsafe` is denied crate-wide and allowed in this module only: every LocalAuthentication
// binding is an `unsafe` method. Each block is minimal and carries a `// SAFETY:` note; the guard
// `unsafe_only_in_allow_listed_modules` lists this file.
#![allow(unsafe_code)]

use crate::keychain_device_unlock::{
    map_la_error, AuthPolicy, KeychainDeviceUnlock, LocalAuth, Preflight, SERVICE,
};
use crate::macos_keychain::MacKeychainSecretStore;
use crate::{PlatformError, PlatformResult};
use block2::RcBlock;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::Bool;
use objc2::MainThreadMarker;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::time::Duration;

/// How long a cancelled (timed-out) evaluation may take to deliver its `appCancel` reply before
/// the waiter stops waiting for it.
const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// The real device unlock: the app-only Keychain store and `LAContext`.
pub(crate) fn mac_device_unlock() -> KeychainDeviceUnlock<LaContextAuth> {
    KeychainDeviceUnlock::new(
        LaContextAuth,
        Arc::new(MacKeychainSecretStore::app_only()),
        SERVICE,
    )
}

/// [`LocalAuth`] over a fresh `LAContext` per call.
#[derive(Debug, Default)]
pub(crate) struct LaContextAuth;

fn new_context() -> Retained<LAContext> {
    // SAFETY: `+new` on LAContext takes no arguments and returns a fresh, retained context.
    unsafe { LAContext::new() }
}

fn la_policy(policy: AuthPolicy) -> LAPolicy {
    LAPolicy(policy.raw())
}

/// The `LAError` code of a failed `canEvaluatePolicy` / `evaluatePolicy`.
fn error_code(error: &NSError) -> i64 {
    i64::try_from(error.code()).unwrap_or(i64::MIN)
}

fn can_evaluate_with(context: &LAContext, policy: AuthPolicy) -> Result<(), i64> {
    // SAFETY: `context` is a live LAContext; the preflight check never prompts and reports its
    // failure through the returned `NSError`.
    unsafe { context.canEvaluatePolicy_error(la_policy(policy)) }.map_err(|e| error_code(&e))
}

/// The domain state `context` saw (after a successful preflight or evaluation).
fn domain_state_of(context: &LAContext) -> Option<Vec<u8>> {
    // SAFETY: `context` is a live LAContext; the property getter only reads.
    #[allow(deprecated)]
    let data = unsafe { context.evaluatedPolicyDomainState() };
    data.map(|d| d.to_vec())
}

impl LocalAuth for LaContextAuth {
    fn can_evaluate(&self, policy: AuthPolicy) -> Result<(), i64> {
        autoreleasepool(|_| can_evaluate_with(&new_context(), policy))
    }

    fn biometric_domain_state(&self) -> Preflight {
        autoreleasepool(|_| {
            let context = new_context();
            // The state is populated by a successful biometric preflight. A failed preflight
            // keeps its code (lockout is not a changed finger set; the caller classifies it).
            can_evaluate_with(&context, AuthPolicy::Biometrics)?;
            Ok(domain_state_of(&context))
        })
    }

    fn evaluate(
        &self,
        policy: AuthPolicy,
        reason: &str,
        timeout: Duration,
    ) -> PlatformResult<Option<Vec<u8>>> {
        if MainThreadMarker::new().is_some() {
            return Err(PlatformError::Failed(
                "device authentication must not run on the main thread".to_owned(),
            ));
        }
        autoreleasepool(|_| {
            let context = new_context();
            if policy == AuthPolicy::Biometrics {
                // Hide "Use Password..." for the biometrics-only mode: its whole point is Touch ID.
                // SAFETY: `context` is a live LAContext; the setter copies the string.
                unsafe { context.setLocalizedFallbackTitle(Some(&NSString::from_str(""))) };
            }
            let (tx, rx) = mpsc::channel::<Result<(), i64>>();
            // The reply is delivered once; the `Mutex<Option<_>>` makes a second call a no-op.
            let tx = Mutex::new(Some(tx));
            let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
                let outcome = if success.as_bool() {
                    Ok(())
                } else {
                    // SAFETY: LocalAuthentication passes either nil or a valid NSError that lives
                    // for the duration of the reply call; it is only read here.
                    Err(unsafe { error.as_ref() }.map_or(-1, error_code))
                };
                let sender = tx.lock().unwrap_or_else(PoisonError::into_inner).take();
                if let Some(sender) = sender {
                    // A reply after the waiter gave up is dropped.
                    let _ = sender.send(outcome);
                }
            });
            // SAFETY: `context` is a live LAContext kept on this stack until the reply arrived
            // or the evaluation was invalidated below; `reason` is non-empty (checked by the
            // caller, an empty one raises an exception); the reply block is sendable because it
            // captures only a `Mutex<Option<mpsc::Sender<_>>>`, and the framework copies it.
            unsafe {
                context.evaluatePolicy_localizedReason_reply(
                    la_policy(policy),
                    &NSString::from_str(reason),
                    &reply,
                );
            }
            let outcome = match rx.recv_timeout(timeout) {
                Ok(outcome) => outcome,
                Err(_) => {
                    // SAFETY: `context` is live; invalidating ends the evaluation (the prompt
                    // disappears) and is a no-op on an already finished one.
                    unsafe { context.invalidate() };
                    let _ = rx.recv_timeout(CANCEL_GRACE);
                    return Err(PlatformError::Cancelled);
                }
            };
            outcome.map_err(map_la_error)?;
            Ok(domain_state_of(&context))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain_device_unlock::{policy_for, ItemStore};
    use crate::testing::device_unlock_contract::{self as contract, ContractSubject};
    use crate::{DeviceUnlock, DeviceUnlockMode, SecretStore};
    use objc2_local_authentication::LAError;
    use secrecy::SecretString;

    #[test]
    fn sdk_policy_values_match_auth_policy() {
        assert_eq!(
            la_policy(policy_for(DeviceUnlockMode::Biometrics)),
            LAPolicy::DeviceOwnerAuthenticationWithBiometrics
        );
        assert_eq!(
            la_policy(policy_for(DeviceUnlockMode::AccountPassword)),
            LAPolicy::DeviceOwnerAuthentication
        );
    }

    #[test]
    fn sdk_error_codes_match_the_mapping_table() {
        let code = |e: LAError| i64::try_from(e.0).unwrap();
        for e in [
            LAError::UserCancel,
            LAError::UserFallback,
            LAError::SystemCancel,
            LAError::AppCancel,
        ] {
            assert_eq!(map_la_error(code(e)), PlatformError::Cancelled);
        }
        assert_eq!(
            map_la_error(code(LAError::BiometryLockout)),
            PlatformError::Lockout
        );
        for e in [
            LAError::PasscodeNotSet,
            LAError::BiometryNotAvailable,
            LAError::BiometryNotEnrolled,
        ] {
            assert_eq!(map_la_error(code(e)), PlatformError::NotEnrolledOnDevice);
        }
    }

    // Real-Keychain tests: ignored by default (`AGENTS.md`: no non-ignored test touches the real
    // Keychain or LAContext) and additionally gated on `MYPASS_KEYCHAIN_TESTS=1`. They use a
    // unique test service, never `com.mypass.app.biometric`, and never show a prompt: only the
    // preflight (`canEvaluatePolicy`) and the item store run. Prompts (Touch ID, the account
    // password) are the owner's manual checks M4 to M7.
    // Run: `MYPASS_KEYCHAIN_TESTS=1 cargo test -p mypass-platform -- --ignored macos_device_unlock`.

    fn keychain_tests_enabled() -> bool {
        if std::env::var("MYPASS_KEYCHAIN_TESTS").as_deref() == Ok("1") {
            return true;
        }
        println!("skipped: set MYPASS_KEYCHAIN_TESTS=1 to use the real keychain");
        false
    }

    /// Unique per test in this process too (the clock alone collides between parallel tests).
    fn unique_service() -> String {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("com.mypass.tests.unlock.{}.{nanos}.{n}", std::process::id())
    }

    struct Scratch {
        du: KeychainDeviceUnlock<LaContextAuth>,
        store: Arc<MacKeychainSecretStore>,
        service: String,
    }

    impl Scratch {
        fn new() -> Self {
            let service = unique_service();
            let store = Arc::new(MacKeychainSecretStore::app_only());
            let du = KeychainDeviceUnlock::new(LaContextAuth, store.clone(), &service);
            Self { du, store, service }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for account in self.store.accounts(&self.service).unwrap_or_default() {
                let _ = self.store.delete(&self.service, &account);
            }
        }
    }

    #[test]
    #[ignore = "uses the real login keychain; set MYPASS_KEYCHAIN_TESTS=1"]
    fn macos_device_unlock_item_round_trip_without_prompt() {
        if !keychain_tests_enabled() {
            return;
        }
        let s = Scratch::new();
        let mode = DeviceUnlockMode::AccountPassword;
        if s.du.availability() == crate::DeviceUnlockAvailability::Unavailable {
            println!("skipped: this Mac has no account password policy available");
            return;
        }
        assert_eq!(s.du.is_enrolled(mode, "vault-a"), Ok(false));
        s.du.enroll(mode, "vault-a", &SecretString::from("pw".to_owned()))
            .unwrap();
        assert_eq!(s.du.is_enrolled(mode, "vault-a"), Ok(true));
        assert_eq!(
            s.du.is_enrolled(DeviceUnlockMode::Biometrics, "vault-a"),
            Ok(false)
        );
    }

    #[test]
    #[ignore = "uses the real login keychain and Touch ID preflight; set MYPASS_KEYCHAIN_TESTS=1"]
    fn macos_device_unlock_forget_removes_secret_and_domain_state() {
        if !keychain_tests_enabled() {
            return;
        }
        let s = Scratch::new();
        let biometrics = matches!(
            s.du.availability(),
            crate::DeviceUnlockAvailability::Available { biometrics: true }
        );
        if !biometrics {
            println!("skipped: no Touch ID on this Mac, so no biometric domain state");
            return;
        }
        let pw = SecretString::from("pw".to_owned());
        s.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &pw)
            .unwrap();
        s.du.enroll(DeviceUnlockMode::AccountPassword, "vault-a", &pw)
            .unwrap();
        assert_eq!(s.store.accounts(&s.service).unwrap().len(), 3);
        assert_eq!(
            s.du.is_enrolled(DeviceUnlockMode::Biometrics, "vault-a"),
            Ok(true)
        );
        s.du.forget("vault-a").unwrap();
        assert_eq!(s.store.accounts(&s.service), Ok(Vec::new()));
    }

    /// The contract suite on the real body. Only meaningful at a Mac where someone answers the
    /// prompts (each unlock and presence case shows one), so it is ignored and env-gated; the
    /// hook cases report `Skipped`.
    #[test]
    #[ignore = "shows real Touch ID / password prompts; set MYPASS_KEYCHAIN_TESTS=1 and answer them"]
    fn macos_device_unlock_contract_with_prompts() {
        if !keychain_tests_enabled() {
            return;
        }
        let s = Scratch::new();
        let make = || {
            let du: Arc<dyn DeviceUnlock> = Arc::new(KeychainDeviceUnlock::new(
                LaContextAuth,
                s.store.clone(),
                &s.service,
            ));
            ContractSubject::without_hooks(du)
        };
        for (name, outcome) in contract::run(&make) {
            println!("{name}: {outcome:?}");
        }
    }
}
