// Test-only support code: Swift toggle parity fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use crate::{generate, parse_password_rules};
use rand::{rngs::StdRng, SeedableRng};
const AMUNDI: &str = "minlength: 6; maxlength: 6; allowed: digit; max-consecutive: 3";
const CLASSMATES: &str = "minlength: 6; maxlength: 20; allowed: lower, upper, digit, [!@#$%^&*];";
const ACME: &str =
    "minlength: 8; maxlength: 40; required: upper; required: [!#$%&*@^]; allowed: lower,digit;";
#[test]
fn sole_allowed_class_is_permitted_forced_on_and_excludes_the_rest() {
    let r = parse_password_rules(AMUNDI).unwrap();
    assert_eq!(permitted_toggle_classes(&r), Some(BTreeSet::from([Digit])));
    assert_eq!(forced_on_toggle_classes(&r), BTreeSet::from([Digit]));
    assert_eq!(
        excluded_toggle_classes(&r),
        BTreeSet::from([Lower, Upper, Special])
    );
}
#[test]
fn sole_allowed_class_is_generated_even_with_its_own_toggle_off() {
    let r = parse_password_rules(AMUNDI).unwrap();
    let o = GeneratorOptions {
        digits: false,
        ..Default::default()
    };
    let p = generate(&o, Some(&r), &mut StdRng::seed_from_u64(7)).unwrap();
    assert_eq!(p.len(), 6);
    assert!(p.chars().all(|c| c.is_ascii_digit()));
}
#[test]
fn an_allowed_custom_symbol_class_makes_symbols_permitted_rather_than_excluded() {
    let r = parse_password_rules(CLASSMATES).unwrap();
    assert_eq!(
        permitted_toggle_classes(&r),
        Some(BTreeSet::from([Lower, Upper, Digit, Special]))
    );
    assert!(excluded_toggle_classes(&r).is_empty());
    assert!(forced_on_toggle_classes(&r).is_empty());
}
#[test]
fn an_allowed_only_custom_class_is_suppressed_when_its_toggle_is_off() {
    let r = parse_password_rules(CLASSMATES).unwrap();
    let o = GeneratorOptions {
        symbols: false,
        ..Default::default()
    };
    let mut rng = StdRng::seed_from_u64(7);
    for _ in 0..50 {
        let p = generate(&o, Some(&r), &mut rng).unwrap();
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
#[test]
fn an_allowed_only_custom_class_contributes_only_its_own_members_when_its_toggle_is_on() {
    let r = parse_password_rules(CLASSMATES).unwrap();
    let mut rng = StdRng::seed_from_u64(7);
    let mut saw = false;
    for _ in 0..50 {
        let p = generate(&GeneratorOptions::default(), Some(&r), &mut rng).unwrap();
        assert!(p
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!@#$%^&*".contains(c)));
        saw |= p.chars().any(|c| "!@#$%^&*".contains(c));
    }
    assert!(saw);
}
#[test]
fn a_required_custom_class_still_ignores_the_toggle() {
    let r = parse_password_rules(ACME).unwrap();
    let o = GeneratorOptions {
        length: 12,
        symbols: false,
        ..Default::default()
    };
    let mut rng = StdRng::seed_from_u64(7);
    for _ in 0..20 {
        let p = generate(&o, Some(&r), &mut rng).unwrap();
        assert!(p.chars().any(|c| "!#$%&*@^".contains(c)));
    }
}
#[test]
fn effective_options_derives_without_mutating_the_users_own_choice() {
    let r = parse_password_rules(AMUNDI).unwrap();
    let user = GeneratorOptions {
        digits: false,
        ..Default::default()
    };
    let derived = effective_options(&user, &r);
    assert_eq!(
        derived,
        GeneratorOptions {
            length: 6,
            lowercase: false,
            uppercase: false,
            digits: true,
            symbols: false
        }
    );
    assert_eq!(
        user,
        GeneratorOptions {
            length: 20,
            lowercase: true,
            uppercase: true,
            digits: false,
            symbols: true
        }
    );
    assert_eq!(effective_options(&derived, &r), derived);
}
#[test]
fn a_non_ascii_custom_class_member_counts_as_a_symbol() {
    let r=parse_password_rules("minlength: 8; maxlength: 38; required: lower, upper; required: digit; allowed: [-\u{e4}\u{fc}\u{f6}\u{c4}\u{dc}\u{d6}\u{df}!$%&/()=?+#,.:];").unwrap();
    assert_eq!(toggle_backed_class_of('\u{e4}'), Special);
    assert_eq!(toggle_backed_class_of('7'), Digit);
    assert_eq!(toggle_backed_class_of('Q'), Upper);
    assert_eq!(toggle_backed_class_of('q'), Lower);
    assert!(excluded_toggle_classes(&r).is_empty());
}
#[test]
fn unrestricted_and_multiple_permitted_classes_do_not_force_a_choice() {
    for r in [
        PasswordRules::default(),
        parse_password_rules("allowed: ascii-printable;").unwrap(),
    ] {
        assert_eq!(permitted_toggle_classes(&r), None);
        assert!(excluded_toggle_classes(&r).is_empty());
    }
    let r = parse_password_rules("allowed: lower,upper;").unwrap();
    assert!(forced_on_toggle_classes(&r).is_empty());
    assert_eq!(TOGGLE_BACKED_CLASSES, [Lower, Upper, Digit, Special]);
}
