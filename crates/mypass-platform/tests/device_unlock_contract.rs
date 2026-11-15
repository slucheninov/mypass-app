//! The shared device-unlock contract (`mypass_platform::testing::device_unlock_contract`) against
//! the fake, one test per case. The macOS body runs the same suite in its ignored Keychain tests.

use mypass_platform::testing::device_unlock_contract::{
    self as contract, CaseOutcome, ContractSubject,
};
use mypass_platform::DeviceUnlockAvailability;

fn fake() -> ContractSubject {
    ContractSubject::fake(DeviceUnlockAvailability::Available { biometrics: true })
}

fn ran(outcome: CaseOutcome) {
    assert_eq!(outcome, CaseOutcome::Ran, "the fake supports every hook");
}

#[test]
fn enrolled_secret_returned_after_unlock_evaluation() {
    ran(contract::enrolled_secret_returned_after_unlock_evaluation(
        &fake,
    ));
}

#[test]
fn enrolment_is_per_mode_and_vault() {
    ran(contract::enrolment_is_per_mode_and_vault(&fake));
}

#[test]
fn unlock_after_forget_is_invalidated() {
    ran(contract::unlock_after_forget_is_invalidated(&fake));
}

#[test]
fn cancel_is_cancelled_and_does_not_forget() {
    ran(contract::cancel_is_cancelled_and_does_not_forget(&fake));
}

#[test]
fn is_enrolled_never_prompts() {
    ran(contract::is_enrolled_never_prompts(&fake));
}

#[test]
fn biometric_change_invalidates_biometrics_only_mode_but_not_account_password_mode() {
    ran(
        contract::biometric_change_invalidates_biometrics_only_mode_but_not_account_password_mode(
            &fake,
        ),
    );
}

#[test]
fn verify_presence_needs_no_enrolment_and_stores_nothing() {
    ran(contract::verify_presence_needs_no_enrolment_and_stores_nothing(&fake));
}

#[test]
fn verify_presence_failure_is_reported_and_keeps_enrolments() {
    ran(contract::verify_presence_failure_is_reported_and_keeps_enrolments(&fake));
}

#[test]
fn unsupported_default_reports_unavailable_and_launch_at_login_unavailable() {
    contract::unsupported_default_reports_unavailable_and_launch_at_login_unavailable();
}

#[test]
fn unavailable_fake_refuses_everything() {
    contract::unavailable_fake_refuses_everything();
}

/// `run` covers every case and the fake runs them all (the macOS body skips the hook cases).
#[test]
fn whole_suite_runs_against_the_fake_without_skips() {
    let outcomes = contract::run(&fake);
    assert_eq!(outcomes.len(), contract::cases().len() + 1);
    for (name, outcome) in outcomes {
        assert_eq!(outcome, CaseOutcome::Ran, "{name}");
    }
}

/// Without hooks (the shape the macOS body is run in), the hook cases skip and the rest run.
#[test]
fn subject_without_hooks_skips_only_the_hook_cases() {
    let bare = || {
        let s = fake();
        ContractSubject::without_hooks(s.unlock)
    };
    let skipped: Vec<_> = contract::run(&bare)
        .into_iter()
        .filter(|(_, o)| matches!(o, CaseOutcome::Skipped(_)))
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        skipped,
        [
            "cancel_is_cancelled_and_does_not_forget",
            "is_enrolled_never_prompts",
            "biometric_change_invalidates_biometrics_only_mode_but_not_account_password_mode",
            "verify_presence_failure_is_reported_and_keeps_enrolments",
        ]
    );
}
