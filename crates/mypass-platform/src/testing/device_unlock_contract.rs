//! The shared device-unlock contract: the behaviour the session's device-unlock service relies on,
//! run against [`FakeDeviceUnlock`] in every CI run and against the macOS body in the ignored
//! Keychain tests (`MYPASS_KEYCHAIN_TESTS=1`).
//!
//! A case that needs a test hook the subject does not have (failure injection, an evaluation
//! counter, a biometric change) reports [`CaseOutcome::Skipped`] with the reason instead of
//! printing; the caller decides what to show. The real body offers no hooks, so on a Mac those
//! cases are covered by the owner's manual checks.

use crate::fallback::{UnsupportedDeviceUnlock, UnsupportedLaunchAtLogin};
use crate::testing::FakeDeviceUnlock;
use crate::{
    DeviceUnlock, DeviceUnlockAvailability, DeviceUnlockMode, LaunchAtLogin, LoginItemStatus,
    PlatformError,
};
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;

type Hook<T> = Option<Box<dyn Fn() -> T>>;
type FailHook = Option<Box<dyn Fn(PlatformError)>>;

/// One device-unlock implementation plus the test hooks it supports. Every hook drives the same
/// instance as `unlock`.
pub struct ContractSubject {
    /// The implementation under test.
    pub unlock: Arc<dyn DeviceUnlock>,
    /// Changes the enrolled biometrics (a new fingerprint set).
    pub invalidate_biometrics: Hook<()>,
    /// Makes the next `unlock` fail with the given error (a cancel, a lockout).
    pub fail_next_unlock: FailHook,
    /// Number of `unlock` evaluations (prompts) so far.
    pub evaluation_count: Hook<usize>,
    /// Makes the next `verify_presence` fail with the given error.
    pub fail_next_presence: FailHook,
    /// Number of `verify_presence` prompts so far.
    pub presence_count: Hook<usize>,
}

impl ContractSubject {
    /// A fresh [`FakeDeviceUnlock`] with every hook wired.
    pub fn fake(availability: DeviceUnlockAvailability) -> Self {
        let fake = FakeDeviceUnlock::new(availability);
        let (a, b, c, d, e) = (
            fake.clone(),
            fake.clone(),
            fake.clone(),
            fake.clone(),
            fake.clone(),
        );
        Self {
            unlock: fake,
            invalidate_biometrics: Some(Box::new(move || a.invalidate_biometrics())),
            fail_next_unlock: Some(Box::new(move |err| b.fail_next_unlock(err))),
            evaluation_count: Some(Box::new(move || c.evaluation_count())),
            fail_next_presence: Some(Box::new(move |err| d.fail_next_presence(err))),
            presence_count: Some(Box::new(move || e.presence_count())),
        }
    }

    /// A real implementation without hooks (the macOS body in the ignored tests).
    pub fn without_hooks(unlock: Arc<dyn DeviceUnlock>) -> Self {
        Self {
            unlock,
            invalidate_biometrics: None,
            fail_next_unlock: None,
            evaluation_count: None,
            fail_next_presence: None,
            presence_count: None,
        }
    }
}

/// Whether a case ran or needed a hook the subject lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseOutcome {
    Ran,
    Skipped(&'static str),
}

/// One contract case over a subject factory.
pub type Case = fn(&dyn Fn() -> ContractSubject) -> CaseOutcome;

const REASON: &str = "MyPass contract test";
const VAULT: &str = "mypass-contract-vault-a";
const OTHER_VAULT: &str = "mypass-contract-vault-b";

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn revealed(r: Result<SecretString, PlatformError>) -> Result<String, PlatformError> {
    r.map(|s| s.expose_secret().to_owned())
}

/// Forgets both test vaults so a real Keychain keeps nothing after a case.
fn cleanup(du: &dyn DeviceUnlock) {
    let _ = du.forget(VAULT);
    let _ = du.forget(OTHER_VAULT);
}

/// Every subject case, by name (the unsupported-fallback case takes no subject and is separate).
pub fn cases() -> Vec<(&'static str, Case)> {
    vec![
        (
            "enrolled_secret_returned_after_unlock_evaluation",
            enrolled_secret_returned_after_unlock_evaluation,
        ),
        (
            "enrolment_is_per_mode_and_vault",
            enrolment_is_per_mode_and_vault,
        ),
        (
            "unlock_after_forget_is_invalidated",
            unlock_after_forget_is_invalidated,
        ),
        (
            "cancel_is_cancelled_and_does_not_forget",
            cancel_is_cancelled_and_does_not_forget,
        ),
        ("is_enrolled_never_prompts", is_enrolled_never_prompts),
        (
            "biometric_change_invalidates_biometrics_only_mode_but_not_account_password_mode",
            biometric_change_invalidates_biometrics_only_mode_but_not_account_password_mode,
        ),
        (
            "verify_presence_needs_no_enrolment_and_stores_nothing",
            verify_presence_needs_no_enrolment_and_stores_nothing,
        ),
        (
            "verify_presence_failure_is_reported_and_keeps_enrolments",
            verify_presence_failure_is_reported_and_keeps_enrolments,
        ),
    ]
}

/// Runs every subject case plus the unsupported-fallback case; returns each case's outcome.
pub fn run(make: &dyn Fn() -> ContractSubject) -> Vec<(&'static str, CaseOutcome)> {
    let mut outcomes: Vec<_> = cases()
        .into_iter()
        .map(|(name, case)| (name, case(make)))
        .collect();
    unsupported_default_reports_unavailable_and_launch_at_login_unavailable();
    outcomes.push((
        "unsupported_default_reports_unavailable_and_launch_at_login_unavailable",
        CaseOutcome::Ran,
    ));
    outcomes
}

/// `enroll` then `unlock` gives back exactly the enrolled secret, through one evaluation.
pub fn enrolled_secret_returned_after_unlock_evaluation(
    make: &dyn Fn() -> ContractSubject,
) -> CaseOutcome {
    let s = make();
    let du = s.unlock.as_ref();
    cleanup(du);
    let before = s.evaluation_count.as_ref().map(|c| c());
    assert_eq!(
        du.enroll(DeviceUnlockMode::AccountPassword, VAULT, &secret("pw-1")),
        Ok(())
    );
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::AccountPassword, VAULT),
        Ok(true)
    );
    assert_eq!(
        revealed(du.unlock(DeviceUnlockMode::AccountPassword, VAULT, REASON)),
        Ok("pw-1".to_owned())
    );
    if let (Some(before), Some(count)) = (before, &s.evaluation_count) {
        assert_eq!(count(), before + 1, "one unlock is one evaluation");
    }
    // Re-enrolling replaces the secret.
    assert_eq!(
        du.enroll(DeviceUnlockMode::AccountPassword, VAULT, &secret("pw-2")),
        Ok(())
    );
    assert_eq!(
        revealed(du.unlock(DeviceUnlockMode::AccountPassword, VAULT, REASON)),
        Ok("pw-2".to_owned())
    );
    cleanup(du);
    CaseOutcome::Ran
}

/// Enrolments are keyed by `(mode, vault_id)`: another mode or vault is not enrolled, and
/// `forget` of one vault leaves the other.
pub fn enrolment_is_per_mode_and_vault(make: &dyn Fn() -> ContractSubject) -> CaseOutcome {
    let s = make();
    let du = s.unlock.as_ref();
    cleanup(du);
    assert_eq!(
        du.enroll(DeviceUnlockMode::Biometrics, VAULT, &secret("a")),
        Ok(())
    );
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::AccountPassword, VAULT),
        Ok(false)
    );
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::Biometrics, OTHER_VAULT),
        Ok(false)
    );
    assert_eq!(
        du.enroll(DeviceUnlockMode::Biometrics, OTHER_VAULT, &secret("b")),
        Ok(())
    );
    assert_eq!(du.forget(VAULT), Ok(()));
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::Biometrics, VAULT),
        Ok(false)
    );
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::Biometrics, OTHER_VAULT),
        Ok(true)
    );
    cleanup(du);
    CaseOutcome::Ran
}

/// After `forget`, no mode is enrolled and `unlock` is `Invalidated`.
pub fn unlock_after_forget_is_invalidated(make: &dyn Fn() -> ContractSubject) -> CaseOutcome {
    let s = make();
    let du = s.unlock.as_ref();
    cleanup(du);
    for mode in [
        DeviceUnlockMode::Biometrics,
        DeviceUnlockMode::AccountPassword,
    ] {
        assert_eq!(du.enroll(mode, VAULT, &secret("pw")), Ok(()));
    }
    assert_eq!(du.forget(VAULT), Ok(()));
    for mode in [
        DeviceUnlockMode::Biometrics,
        DeviceUnlockMode::AccountPassword,
    ] {
        assert_eq!(du.is_enrolled(mode, VAULT), Ok(false), "{mode:?}");
        assert_eq!(
            revealed(du.unlock(mode, VAULT, REASON)),
            Err(PlatformError::Invalidated),
            "{mode:?}"
        );
    }
    // Forgetting a vault with nothing enrolled is not an error.
    assert_eq!(du.forget(VAULT), Ok(()));
    CaseOutcome::Ran
}

/// A cancelled prompt is `Cancelled` and keeps the enrolment: the next unlock still works.
pub fn cancel_is_cancelled_and_does_not_forget(make: &dyn Fn() -> ContractSubject) -> CaseOutcome {
    let s = make();
    let Some(fail_next) = &s.fail_next_unlock else {
        return CaseOutcome::Skipped("needs a failure-injection hook (manual check on a Mac)");
    };
    let du = s.unlock.as_ref();
    cleanup(du);
    assert_eq!(
        du.enroll(DeviceUnlockMode::Biometrics, VAULT, &secret("pw")),
        Ok(())
    );
    fail_next(PlatformError::Cancelled);
    assert_eq!(
        revealed(du.unlock(DeviceUnlockMode::Biometrics, VAULT, REASON)),
        Err(PlatformError::Cancelled)
    );
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::Biometrics, VAULT),
        Ok(true)
    );
    assert_eq!(
        revealed(du.unlock(DeviceUnlockMode::Biometrics, VAULT, REASON)),
        Ok("pw".to_owned())
    );
    cleanup(du);
    CaseOutcome::Ran
}

/// `is_enrolled` is a silent lookup: it never evaluates (never shows a prompt).
pub fn is_enrolled_never_prompts(make: &dyn Fn() -> ContractSubject) -> CaseOutcome {
    let s = make();
    let (Some(evaluations), Some(presence)) = (&s.evaluation_count, &s.presence_count) else {
        return CaseOutcome::Skipped("needs prompt counters (manual check on a Mac)");
    };
    let du = s.unlock.as_ref();
    cleanup(du);
    let (e0, p0) = (evaluations(), presence());
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::Biometrics, VAULT),
        Ok(false)
    );
    assert_eq!(
        du.enroll(DeviceUnlockMode::Biometrics, VAULT, &secret("pw")),
        Ok(())
    );
    for _ in 0..3 {
        assert_eq!(
            du.is_enrolled(DeviceUnlockMode::Biometrics, VAULT),
            Ok(true)
        );
        assert_eq!(
            du.is_enrolled(DeviceUnlockMode::AccountPassword, VAULT),
            Ok(false)
        );
    }
    assert_eq!((evaluations(), presence()), (e0, p0));
    cleanup(du);
    CaseOutcome::Ran
}

/// A biometric change invalidates the biometrics-only enrolment; the account-password one, whose
/// policy does not depend on the enrolled fingers, survives.
pub fn biometric_change_invalidates_biometrics_only_mode_but_not_account_password_mode(
    make: &dyn Fn() -> ContractSubject,
) -> CaseOutcome {
    let s = make();
    let Some(invalidate) = &s.invalidate_biometrics else {
        return CaseOutcome::Skipped("needs a biometric change (manual check on a Mac)");
    };
    let du = s.unlock.as_ref();
    cleanup(du);
    assert_eq!(
        du.enroll(DeviceUnlockMode::Biometrics, VAULT, &secret("bio")),
        Ok(())
    );
    assert_eq!(
        du.enroll(DeviceUnlockMode::AccountPassword, VAULT, &secret("acct")),
        Ok(())
    );
    invalidate();
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::Biometrics, VAULT),
        Ok(false)
    );
    assert_eq!(
        revealed(du.unlock(DeviceUnlockMode::Biometrics, VAULT, REASON)),
        Err(PlatformError::Invalidated)
    );
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::AccountPassword, VAULT),
        Ok(true)
    );
    assert_eq!(
        revealed(du.unlock(DeviceUnlockMode::AccountPassword, VAULT, REASON)),
        Ok("acct".to_owned())
    );
    cleanup(du);
    CaseOutcome::Ran
}

/// `verify_presence` works without any enrolment, is one prompt, and stores nothing.
pub fn verify_presence_needs_no_enrolment_and_stores_nothing(
    make: &dyn Fn() -> ContractSubject,
) -> CaseOutcome {
    let s = make();
    let du = s.unlock.as_ref();
    cleanup(du);
    let before = s.presence_count.as_ref().map(|c| c());
    assert_eq!(du.verify_presence(REASON), Ok(()));
    if let (Some(before), Some(count)) = (before, &s.presence_count) {
        assert_eq!(count(), before + 1, "one presence check is one prompt");
    }
    for mode in [
        DeviceUnlockMode::Biometrics,
        DeviceUnlockMode::AccountPassword,
    ] {
        assert_eq!(du.is_enrolled(mode, VAULT), Ok(false), "{mode:?}");
    }
    CaseOutcome::Ran
}

/// A failed presence check passes the error through and touches no enrolment.
pub fn verify_presence_failure_is_reported_and_keeps_enrolments(
    make: &dyn Fn() -> ContractSubject,
) -> CaseOutcome {
    let s = make();
    let Some(fail_next) = &s.fail_next_presence else {
        return CaseOutcome::Skipped("needs a failure-injection hook (manual check on a Mac)");
    };
    let du = s.unlock.as_ref();
    cleanup(du);
    assert_eq!(
        du.enroll(DeviceUnlockMode::AccountPassword, VAULT, &secret("pw")),
        Ok(())
    );
    for e in [
        PlatformError::Cancelled,
        PlatformError::Lockout,
        PlatformError::NotEnrolledOnDevice,
    ] {
        fail_next(e.clone());
        assert_eq!(du.verify_presence(REASON), Err(e));
    }
    assert_eq!(du.verify_presence(REASON), Ok(()));
    assert_eq!(
        revealed(du.unlock(DeviceUnlockMode::AccountPassword, VAULT, REASON)),
        Ok("pw".to_owned())
    );
    cleanup(du);
    CaseOutcome::Ran
}

/// The fallback bodies: device unlock is `Unavailable` (also for the presence check, through the
/// trait's default body) and launch at login reports `Unsupported`.
pub fn unsupported_default_reports_unavailable_and_launch_at_login_unavailable() {
    let du = UnsupportedDeviceUnlock;
    assert_eq!(du.availability(), DeviceUnlockAvailability::Unavailable);
    assert_eq!(
        du.verify_presence(REASON),
        Err(PlatformError::DeviceUnlockUnavailable)
    );
    assert!(du.is_enrolled(DeviceUnlockMode::Biometrics, VAULT).is_err());
    assert!(revealed(du.unlock(DeviceUnlockMode::Biometrics, VAULT, REASON)).is_err());
    let login = UnsupportedLaunchAtLogin;
    assert_eq!(login.status(), Ok(LoginItemStatus::Unsupported));
    assert_eq!(login.is_enabled(), Ok(false));
    assert!(login.set_enabled(true).is_err());
}

/// The fake with device unlock unavailable fails every call but `availability` with
/// `DeviceUnlockUnavailable`, the presence check included.
pub fn unavailable_fake_refuses_everything() {
    let s = ContractSubject::fake(DeviceUnlockAvailability::Unavailable);
    let du = s.unlock.as_ref();
    let unavailable = Err(PlatformError::DeviceUnlockUnavailable);
    assert_eq!(du.availability(), DeviceUnlockAvailability::Unavailable);
    assert_eq!(du.verify_presence(REASON), unavailable);
    assert_eq!(
        du.enroll(DeviceUnlockMode::Biometrics, VAULT, &secret("pw")),
        unavailable
    );
    assert_eq!(du.forget(VAULT), unavailable);
    assert_eq!(
        du.is_enrolled(DeviceUnlockMode::Biometrics, VAULT),
        Err(PlatformError::DeviceUnlockUnavailable)
    );
}
