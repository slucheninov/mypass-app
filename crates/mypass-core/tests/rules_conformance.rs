// Test-only support code: independent rule oracle, never prints password material.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_core::{
    generate, parse_password_rules, CharacterClass, GeneratorOptions, PasswordRules,
    PasswordRulesIndex,
};
use rand::{rngs::StdRng, SeedableRng};
fn matches(c: char, class: &CharacterClass) -> bool {
    match class {
        CharacterClass::Lower => c.is_ascii_lowercase(),
        CharacterClass::Upper => c.is_ascii_uppercase(),
        CharacterClass::Digit => c.is_ascii_digit(),
        CharacterClass::Special => !c.is_alphanumeric(),
        CharacterClass::AsciiPrintable => (' '..='~').contains(&c),
        CharacterClass::Unicode => true,
        CharacterClass::Custom(set) => set.contains(&c),
    }
}
fn assert_conforms(password: &str, rules: &PasswordRules, host: &str) {
    let chars: Vec<_> = password.chars().collect();
    let context = format!("host={host} seed=0xC0FFEE length={}", chars.len());
    if let Some(min) = rules.min_length {
        assert!(chars.len() >= min as usize, "{context}");
    }
    if let Some(max) = rules.max_length {
        assert!(chars.len() <= max as usize, "{context}");
    }
    for clause in &rules.required_clauses {
        assert!(
            chars
                .iter()
                .any(|c| clause.iter().any(|cl| matches(*c, cl))),
            "{context}"
        );
    }
    if !rules.allowed_classes.is_empty() {
        assert!(
            chars.iter().all(|c| rules
                .allowed_classes
                .iter()
                .chain(rules.required_clauses.iter().flatten())
                .any(|cl| matches(*c, cl))),
            "{context}"
        );
    }
    if let Some(max) = rules.max_consecutive {
        assert!(
            chars
                .windows(max as usize + 1)
                .all(|w| w.iter().any(|c| *c != w[0])),
            "{context}"
        );
    }
    if let Some(max) = rules.max_repeating {
        let mut counts = std::collections::BTreeMap::new();
        for c in &chars {
            *counts.entry(c).or_insert(0usize) += 1;
        }
        assert!(counts.values().all(|n| *n <= max as usize), "{context}");
    }
    if let Some(max) = rules.max_sequential {
        assert!(
            chars.windows(max as usize + 1).all(|w| {
                let deltas: Vec<_> = w.windows(2).map(|p| p[1] as i64 - p[0] as i64).collect();
                !(deltas.iter().all(|d| *d == 1) || deltas.iter().all(|d| *d == -1))
            }),
            "{context}"
        );
    }
}
#[test]
fn every_vendored_rule_generates_conforming_passwords() {
    let index =
        PasswordRulesIndex::from_json(include_str!("../resources/password-rules.json")).unwrap();
    let mut entries: Vec<_> = index.entries().collect();
    entries.sort();
    assert_eq!(entries.len(), 436);
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);
    for (host, raw) in entries {
        let rules = parse_password_rules(raw).unwrap();
        for _ in 0..20 {
            let password = generate(&GeneratorOptions::default(), Some(&rules), &mut rng)
                .unwrap_or_else(|_| panic!("host={host} seed=0xC0FFEE length=unavailable"));
            assert_conforms(&password, &rules, host);
        }
    }
}
#[test]
fn allowed_class_check_treats_required_as_implicitly_allowed() {
    let rules = PasswordRules {
        required_clauses: vec![vec![CharacterClass::Upper]],
        allowed_classes: vec![CharacterClass::Lower],
        min_length: Some(4),
        max_length: Some(4),
        ..Default::default()
    };
    assert_conforms("Abcd", &rules, "synthetic");
}
#[test]
fn allowed_class_check_treats_empty_allow_list_as_no_restriction() {
    assert_conforms(
        "!!!!",
        &PasswordRules {
            min_length: Some(4),
            max_length: Some(4),
            ..Default::default()
        },
        "synthetic",
    );
}

#[test]
fn hand_built_repeating_and_sequential_rules() {
    let r = PasswordRules {
        max_repeating: Some(2),
        max_sequential: Some(2),
        ..Default::default()
    };
    assert_conforms("azabz", &r, "synthetic");
    for bad in ["aaaz", "abc", "cba"] {
        assert!(std::panic::catch_unwind(|| assert_conforms(bad, &r, "synthetic")).is_err());
    }
}
