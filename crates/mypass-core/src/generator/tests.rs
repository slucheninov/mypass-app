// Test-only support code. Assertions never format generated passwords.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use crate::parse_password_rules;
use rand::{rngs::StdRng, SeedableRng};
fn make_rng() -> StdRng {
    StdRng::seed_from_u64(7)
}
fn make_password(opts: GeneratorOptions, raw: Option<&str>) -> Zeroizing<String> {
    let rules = raw.map(|r| parse_password_rules(r).unwrap());
    generate(&opts, rules.as_ref(), &mut make_rng()).unwrap()
}
#[test]
fn generates_requested_length() {
    assert_eq!(make_password(GeneratorOptions::default(), None).len(), 20);
}
#[test]
fn only_digits_yields_only_digits() {
    let p = make_password(
        GeneratorOptions {
            length: 16,
            lowercase: false,
            uppercase: false,
            digits: true,
            symbols: false,
        },
        None,
    );
    assert_eq!(p.len(), 16);
    assert!(p.chars().all(|c| c.is_ascii_digit()));
}
#[test]
fn no_class_selected_errors() {
    let o = GeneratorOptions {
        length: 16,
        lowercase: false,
        uppercase: false,
        digits: false,
        symbols: false,
    };
    assert!(matches!(
        generate(&o, None, &mut make_rng()),
        Err(NoCharacterClassSelected)
    ));
}
#[test]
fn two_passwords_differ() {
    let o = GeneratorOptions {
        length: 32,
        ..Default::default()
    };
    assert!(generate_os(&o, None).unwrap() != generate_os(&o, None).unwrap());
}
#[test]
fn zero_length_is_empty() {
    assert!(make_password(
        GeneratorOptions {
            length: 0,
            uppercase: false,
            digits: false,
            symbols: false,
            ..Default::default()
        },
        None
    )
    .is_empty());
}
#[test]
fn each_selected_class_appears_across_many_generations() {
    let mut rng = make_rng();
    let mut seen = BTreeSet::new();
    for _ in 0..200 {
        let p = generate(&GeneratorOptions::default(), None, &mut rng).unwrap();
        seen.extend(p.chars().map(toggle_backed_class_of));
    }
    assert_eq!(seen, BTreeSet::from([Lower, Upper, Digit, Special]));
}
#[test]
fn no_modulo_bias_smoke() {
    let pool: Vec<_> = LOWER.chars().collect();
    let mut counts = [0u32; 26];
    let mut rng = make_rng();
    for _ in 0..100_000 {
        let c = sample(&pool, &mut rng).unwrap();
        counts[(c as u8 - b'a') as usize] += 1;
    }
    let mean = 100_000.0 / 26.0;
    assert!(counts
        .iter()
        .all(|n| (*n as f64 - mean).abs() <= mean * 0.1));
}
#[derive(Default)]
struct BoundaryRng {
    calls: usize,
}
impl RngCore for BoundaryRng {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
    fn next_u64(&mut self) -> u64 {
        self.calls += 1;
        if self.calls == 1 {
            u64::MAX
        } else {
            0
        }
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for byte in dest {
            *byte = self.next_u32() as u8;
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl CryptoRng for BoundaryRng {}
#[test]
fn rejection_sampling_rejects_maximum_rng_value() {
    let mut rng = BoundaryRng::default();
    let pool: Vec<_> = LOWER.chars().collect();
    assert!(sample(&pool, &mut rng).unwrap() == 'a');
    assert!(rng.calls >= 2);
}
#[test]
fn required_clauses_are_satisfied() {
    let o = GeneratorOptions {
        length: 3,
        lowercase: false,
        uppercase: false,
        digits: false,
        symbols: false,
    };
    let p = make_password(
        o,
        Some("required: upper; required: digit; required: [@]; maxlength: 3;"),
    );
    assert!(p.chars().any(|c| c.is_ascii_uppercase()));
    assert!(p.chars().any(|c| c.is_ascii_digit()));
    assert!(p.contains('@'));
}
#[test]
fn max_length_clamps_requested_length() {
    assert_eq!(
        make_password(GeneratorOptions::default(), Some("maxlength: 8;")).len(),
        8
    );
}
#[test]
fn min_length_raises_short_request() {
    assert_eq!(
        make_password(
            GeneratorOptions {
                length: 2,
                ..Default::default()
            },
            Some("minlength: 12;")
        )
        .len(),
        12
    );
}
#[test]
fn max_consecutive_respected() {
    let r = parse_password_rules("allowed: [ab]; max-consecutive: 1;").unwrap();
    let mut rng = make_rng();
    for _ in 0..500 {
        let p = generate(&GeneratorOptions::default(), Some(&r), &mut rng).unwrap();
        assert_eq!(p.len(), 20);
        assert!(p.as_bytes().windows(2).all(|w| w[0] != w[1]));
    }
}
#[test]
fn unsatisfiable_rules_error() {
    for raw in [
        "maxlength: 1; required: digit; required: upper;",
        "minlength: 9; maxlength: 2;",
        "required: unicode;",
        "allowed: [a]; max-consecutive: 1;",
    ] {
        let r = parse_password_rules(raw).unwrap();
        assert!(matches!(
            generate(&GeneratorOptions::default(), Some(&r), &mut make_rng()),
            Err(UnsatisfiableRules)
        ));
    }
}
#[test]
fn custom_class_characters_only_from_custom_set() {
    let p = make_password(GeneratorOptions::default(), Some("allowed: [abc];"));
    assert_eq!(p.len(), 20);
    assert!(p.chars().all(|c| "abc".contains(c)));
}
#[test]
fn length_clamp_bounds_huge_requests_with_and_without_rules() {
    let o = GeneratorOptions {
        length: usize::MAX,
        ..Default::default()
    };
    for raw in [None, Some("allowed: lower;")] {
        assert_eq!(make_password(o.clone(), raw).len(), 4096);
    }
    let r = PasswordRules {
        min_length: Some(4097),
        ..Default::default()
    };
    assert!(matches!(
        generate(&o, Some(&r), &mut make_rng()),
        Err(UnsatisfiableRules)
    ));
}
#[test]
fn empty_and_unicode_custom_classes() {
    let r = PasswordRules {
        required_clauses: vec![vec![Custom(BTreeSet::new())]],
        ..Default::default()
    };
    assert!(matches!(
        generate(&GeneratorOptions::default(), Some(&r), &mut make_rng()),
        Err(UnsatisfiableRules)
    ));
    let r = PasswordRules {
        allowed_classes: vec![Custom(BTreeSet::from(['\u{e9}']))],
        ..Default::default()
    };
    let p = generate(&GeneratorOptions::default(), Some(&r), &mut make_rng()).unwrap();
    assert_eq!(p.chars().count(), 20);
    assert!(p.chars().all(|c| c == '\u{e9}'));
}
#[test]
fn repetition_and_sequence_limits() {
    for raw in [
        "allowed: [abc]; maxlength: 3; max-repeating: 1;",
        "allowed: [abc]; maxlength: 3; max-sequential: 1;",
    ] {
        let r = parse_password_rules(raw).unwrap();
        let pool = effective_pool(&GeneratorOptions::default(), &r);
        let p = generate(&GeneratorOptions::default(), Some(&r), &mut make_rng()).unwrap();
        assert!(satisfies(&p.chars().collect::<Vec<_>>(), &r, &pool));
        if r.max_repeating.is_some() {
            assert_eq!(p.chars().collect::<BTreeSet<_>>().len(), 3);
        }
        if r.max_sequential.is_some() {
            assert!(p
                .as_bytes()
                .windows(2)
                .all(|w| (w[0] as i16 - w[1] as i16).abs() != 1));
        }
    }
    for field in 0..3 {
        let mut r = PasswordRules::default();
        match field {
            0 => r.max_consecutive = Some(0),
            1 => r.max_repeating = Some(0),
            _ => r.max_sequential = Some(0),
        };
        assert!(matches!(
            generate(&GeneratorOptions::default(), Some(&r), &mut make_rng()),
            Err(UnsatisfiableRules)
        ));
    }
}
#[test]
fn repair_fixes_a_run_created_by_earlier_repair() {
    let mut rng = make_rng();
    for _ in 0..1000 {
        let mut chars = Zeroizing::new("aaabb".chars().collect::<Vec<_>>());
        repair_max_consecutive(&mut chars, 2, &['a', 'b'], &mut rng).unwrap();
        assert!(chars.windows(3).all(|w| !(w[0] == w[1] && w[1] == w[2])));
    }
}
#[test]
fn deterministic_fallback_constructs_satisfying_candidate_directly() {
    let mut rng = make_rng();
    for _ in 0..100 {
        let chars = deterministic_candidate(20, &[], &['a', 'b'], Some(1), &mut rng).unwrap();
        assert!(chars.windows(2).all(|w| w[0] != w[1]));
    }
}
#[test]
fn deterministic_fallback_satisfies_each_required_clause_directly() {
    let chars = deterministic_candidate(
        3,
        &[vec!['A'], vec!['7'], vec!['@']],
        &['A', '7', '@'],
        None,
        &mut make_rng(),
    )
    .unwrap();
    assert_eq!(chars.len(), 3);
    assert!(['A', '7', '@'].iter().all(|c| chars.contains(c)));
}
#[test]
fn satisfies_rejects_disallowed_character() {
    let r = parse_password_rules("allowed: lower;").unwrap();
    let pool = effective_pool(&GeneratorOptions::default(), &r);
    assert!(satisfies(&['a'], &r, &pool));
    assert!(!satisfies(&['@'], &r, &pool));
}

#[test]
fn swift_length_bound_regressions() {
    for (requested, min, max, expected) in [
        (20, Some(10), Some(12), 12),
        (4, Some(10), Some(12), 10),
        (20, None, Some(12), 12),
        (4, Some(10), None, 10),
    ] {
        let r = PasswordRules {
            min_length: min,
            max_length: max,
            ..Default::default()
        };
        let p = generate(
            &GeneratorOptions {
                length: requested,
                ..Default::default()
            },
            Some(&r),
            &mut make_rng(),
        )
        .unwrap();
        assert_eq!(p.chars().count(), expected);
    }
    let r = PasswordRules {
        min_length: Some(12),
        max_length: Some(10),
        ..Default::default()
    };
    assert!(matches!(
        generate(
            &GeneratorOptions {
                length: 11,
                ..Default::default()
            },
            Some(&r),
            &mut make_rng()
        ),
        Err(UnsatisfiableRules)
    ));
}
#[test]
fn vivo_shaped_rule_is_satisfied() {
    let r = PasswordRules {
        allowed_classes: vec![Digit],
        max_length: Some(6),
        max_consecutive: Some(3),
        ..Default::default()
    };
    let mut rng = make_rng();
    for _ in 0..200 {
        let p = generate(
            &GeneratorOptions {
                length: 16,
                ..Default::default()
            },
            Some(&r),
            &mut rng,
        )
        .unwrap();
        assert_eq!(p.len(), 6);
        assert!(p.chars().all(|c| c.is_ascii_digit()));
        assert!(p
            .as_bytes()
            .windows(4)
            .all(|w| w.iter().any(|c| *c != w[0])));
    }
}
#[test]
fn required_named_and_custom_classes_override_disabled_toggles() {
    for r in [
        PasswordRules {
            required_clauses: vec![vec![Digit]],
            allowed_classes: vec![Digit, Lower],
            ..Default::default()
        },
        PasswordRules {
            required_clauses: vec![vec![Custom(BTreeSet::from(['!', '@', '#']))]],
            allowed_classes: vec![Custom(BTreeSet::from(['!', '@', '#'])), Lower],
            ..Default::default()
        },
    ] {
        let p = generate(
            &GeneratorOptions {
                length: 12,
                uppercase: false,
                digits: false,
                symbols: false,
                ..Default::default()
            },
            Some(&r),
            &mut make_rng(),
        )
        .unwrap();
        assert!(p.chars().any(|c| c.is_ascii_digit() || "!@#".contains(c)));
    }
}
#[test]
fn disallowed_classes_never_appear() {
    let r = parse_password_rules(
        "required: lower; required: digit; allowed: lower,digit,[-!#$&+/?@_];",
    )
    .unwrap();
    let mut rng = make_rng();
    for _ in 0..100 {
        let p = generate(&GeneratorOptions::default(), Some(&r), &mut rng).unwrap();
        assert!(p
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-!#$&+/?@_".contains(c)));
    }
}
#[test]
fn sample_handles_wide_pools_without_a_custom_random_primitive() {
    for len in [257, 500, 1000, 40000, 65535, 65536] {
        let pool: Vec<_> = (0x10000..).filter_map(char::from_u32).take(len).collect();
        let mut rng = make_rng();
        for _ in 0..500 {
            assert!(pool.contains(&sample(&pool, &mut rng).unwrap()));
        }
    }
    assert!(matches!(
        sample(&[], &mut make_rng()),
        Err(UnsatisfiableRules)
    ));
}

#[test]
fn cleanup_unicode_error_precedence_and_zero_consecutive() {
    let off = GeneratorOptions {
        lowercase: false,
        uppercase: false,
        digits: false,
        symbols: false,
        ..Default::default()
    };
    let rules = parse_password_rules("required: unicode;").unwrap();
    assert_eq!(
        generate(&off, Some(&rules), &mut make_rng()),
        Err(NoCharacterClassSelected)
    );
    // Unlike Swift's repair shortcut, zero consecutive forbids even one scalar.
    let rules = PasswordRules {
        max_consecutive: Some(0),
        ..Default::default()
    };
    assert_eq!(
        generate(
            &GeneratorOptions {
                length: 1,
                ..Default::default()
            },
            Some(&rules),
            &mut make_rng()
        ),
        Err(UnsatisfiableRules)
    );
}
#[test]
fn allowed_unicode_only_rule_still_generates_from_the_enabled_toggles() {
    // `unicode` is a superset of `ascii-printable`: every character is permitted, so the gate must
    // be open rather than reporting NoCharacterClassSelected. The alphabet stays the toggle
    // baselines (no invented non-ASCII pool).
    let rules = parse_password_rules("minlength: 12; maxlength: 12; allowed: unicode;").unwrap();
    let p = generate(
        &GeneratorOptions::default(),
        Some(&rules),
        &mut StdRng::seed_from_u64(11),
    )
    .unwrap();
    assert_eq!(p.chars().count(), 12);
    assert!(p.is_ascii());
}
