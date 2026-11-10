//! Structured Apple password rules and a strict, non-panicking parser.
//!
//! Numeric properties are unsigned u32: Swift's signed parsing can accept `-0`
//! and wider integers. Scanning uses Unicode scalars, rather than Swift grapheme
//! clusters; the grammar's named tokens and custom members are ASCII.
use std::collections::BTreeSet;

/// A named character repertoire or a literal custom set.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CharacterClass {
    /// Uppercase ASCII letters.
    Upper,
    /// Lowercase ASCII letters.
    Lower,
    /// ASCII digits.
    Digit,
    /// ASCII special characters.
    Special,
    /// All printable ASCII characters.
    AsciiPrintable,
    /// Unicode characters.
    Unicode,
    /// Explicit ASCII-printable members.
    Custom(BTreeSet<char>),
}

/// Parsed constraints; required clauses are ANDed and alternatives within a clause are ORed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PasswordRules {
    /// Independent required clauses, preserving duplicates and order.
    pub required_clauses: Vec<Vec<CharacterClass>>,
    /// Cumulative allowed classes, including every required class.
    pub allowed_classes: Vec<CharacterClass>,
    /// Strongest lower length bound.
    pub min_length: Option<u32>,
    /// Strongest upper length bound.
    pub max_length: Option<u32>,
    /// Maximum consecutive equal characters.
    pub max_consecutive: Option<u32>,
    /// Maximum occurrences of a character (stored for consumers).
    pub max_repeating: Option<u32>,
    /// Maximum sequential run length (stored for consumers).
    pub max_sequential: Option<u32>,
}

/// Strict parsing failures, corresponding to the Swift parser's error cases.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasswordRulesParseError {
    /// Input contains no non-whitespace characters.
    #[error("empty password rules")]
    EmptyInput,
    /// Unrecognized property, or a name without a colon.
    #[error("unknown password-rules property: {0}")]
    UnknownProperty(String),
    /// Invalid class list or non-negative integer.
    #[error("malformed value for {property}: {value}")]
    MalformedValue {
        /// Property being parsed.
        property: String,
        /// Rejected value.
        value: String,
    },
    /// A bracketed class has no closing delimiter.
    #[error("unterminated custom character class")]
    UnterminatedCustomClass,
}

/// Parse Apple's password-rules grammar, returning typed failures for malformed input.
pub fn parse_password_rules(raw: &str) -> Result<PasswordRules, PasswordRulesParseError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PasswordRulesParseError::EmptyInput);
    }
    let mut scanner = Scanner {
        chars: trimmed.chars().peekable(),
    };
    let mut rules = PasswordRules::default();
    let mut saw_allowed = false;
    scanner.skip_whitespace();
    while let Some(c) = scanner.chars.peek().copied() {
        // Stray/repeated separators are tolerated, as in Apple's reference parser.
        if c == ';' {
            scanner.chars.next();
            scanner.skip_whitespace();
            continue;
        }
        let mut name = String::new();
        let mut saw_colon = false;
        for c in scanner.chars.by_ref() {
            if c == ':' {
                saw_colon = true;
                break;
            }
            name.push(c);
        }
        if !saw_colon {
            return Err(PasswordRulesParseError::UnknownProperty(name.trim().into()));
        }
        let name = name.trim().to_lowercase();
        scanner.skip_whitespace();
        match name.as_str() {
            "required" => rules.required_clauses.push(scanner.class_list(&name)?),
            "allowed" => {
                saw_allowed = true;
                for class in scanner.class_list(&name)? {
                    if !rules.allowed_classes.contains(&class) {
                        rules.allowed_classes.push(class);
                    }
                }
            }
            "minlength" | "maxlength" | "max-consecutive" | "max-repeating" | "max-sequential" => {
                let value: String = scanner.chars.by_ref().take_while(|c| *c != ';').collect();
                let value = value.trim();
                let number = value.parse::<u32>().map_err(|_| malformed(&name, value))?;
                let (slot, lower_bound) = match name.as_str() {
                    "minlength" => (&mut rules.min_length, true),
                    "maxlength" => (&mut rules.max_length, false),
                    "max-consecutive" => (&mut rules.max_consecutive, false),
                    "max-repeating" => (&mut rules.max_repeating, false),
                    _ => (&mut rules.max_sequential, false),
                };
                *slot = Some(match *slot {
                    Some(previous) if lower_bound => previous.max(number),
                    Some(previous) => previous.min(number),
                    None => number,
                });
                // Numeric scanning consumed its separator; class-list scanning leaves it.
                scanner.skip_whitespace();
                continue;
            }
            _ => return Err(PasswordRulesParseError::UnknownProperty(name)),
        }
        scanner.skip_whitespace();
        match scanner.chars.next() {
            None | Some(';') => {}
            Some(c) => {
                let remainder: String = std::iter::once(c).chain(scanner.chars).collect();
                return Err(malformed(&name, &remainder));
            }
        }
        scanner.skip_whitespace();
    }

    // Every required alternative is also allowed. With no explicit allowed property,
    // start from ascii-printable; otherwise preserve the cumulative explicit set.
    if !saw_allowed {
        rules.allowed_classes.push(CharacterClass::AsciiPrintable);
    }
    for clause in &rules.required_clauses {
        for class in clause {
            if !rules.allowed_classes.contains(class) {
                rules.allowed_classes.push(class.clone());
            }
        }
    }
    Ok(rules)
}

fn malformed(property: &str, value: &str) -> PasswordRulesParseError {
    PasswordRulesParseError::MalformedValue {
        property: property.into(),
        value: value.into(),
    }
}

struct Scanner<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Scanner<'_> {
    fn skip_whitespace(&mut self) {
        while self.chars.next_if(|c| c.is_whitespace()).is_some() {}
    }

    fn class_list(
        &mut self,
        property: &str,
    ) -> Result<Vec<CharacterClass>, PasswordRulesParseError> {
        let mut classes = Vec::new();
        self.skip_whitespace();
        while let Some(c) = self.chars.peek().copied() {
            if c == ';' {
                break;
            }
            if c == '[' {
                self.chars.next();
                let members = self.custom_class()?;
                // Empty classes, including classes emptied by filtering non-ASCII members,
                // contribute nothing, matching Apple's reference behavior.
                if !members.is_empty() {
                    classes.push(CharacterClass::Custom(members));
                }
            } else {
                let mut token = String::new();
                while let Some(c) = self.chars.next_if(|c| *c != ',' && *c != ';') {
                    token.push(c);
                }
                let identifier = token.trim().to_lowercase();
                let class = match identifier.as_str() {
                    "upper" => CharacterClass::Upper,
                    "lower" => CharacterClass::Lower,
                    "digit" => CharacterClass::Digit,
                    "special" => CharacterClass::Special,
                    "ascii-printable" => CharacterClass::AsciiPrintable,
                    "unicode" => CharacterClass::Unicode,
                    _ => return Err(malformed(property, &identifier)),
                };
                classes.push(class);
            }
            self.skip_whitespace();
            match self.chars.peek() {
                None | Some(';') => break,
                Some(',') => {
                    self.chars.next();
                    self.skip_whitespace();
                }
                _ => return Err(malformed(property, &self.chars.clone().collect::<String>())),
            }
        }
        if classes.is_empty() {
            return Err(malformed(property, ""));
        }
        Ok(classes)
    }

    fn custom_class(&mut self) -> Result<BTreeSet<char>, PasswordRulesParseError> {
        let mut members = BTreeSet::new();
        for (position, c) in self.chars.by_ref().enumerate() {
            // A hyphen is literal only as the very first character after '['.
            // Non-ASCII-printable characters are silently skipped, as in Swift.
            if !(' '..='~').contains(&c) || (c == '-' && position != 0) {
                continue;
            }
            if c == ']' {
                // First ']' is a literal member only when immediately followed by ']'.
                // The second bracket then closes the class; it is not a general escape.
                if self.chars.next_if_eq(&']').is_some() {
                    members.insert(']');
                }
                return Ok(members);
            }
            members.insert(c);
        }
        Err(PasswordRulesParseError::UnterminatedCustomClass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CharacterClass::*;

    #[test]
    fn and_across_required_clauses_or_within_a_clause() {
        let rules = parse_password_rules("minlength: 8; maxlength: 15; max-consecutive: 3; required: digit; required: special; required: upper,lower;").unwrap();
        assert_eq!(rules.min_length, Some(8));
        assert_eq!(rules.max_length, Some(15));
        assert_eq!(rules.max_consecutive, Some(3));
        assert_eq!(
            rules.required_clauses,
            vec![vec![Digit], vec![Special], vec![Upper, Lower]]
        );
        assert_eq!(
            rules.allowed_classes.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([AsciiPrintable, Digit, Special, Upper, Lower])
        );
    }

    #[test]
    fn custom_bracketed_class_mixed_with_named_classes() {
        let rules = parse_password_rules("minlength: 8; maxlength: 8; max-consecutive: 3; required: digit; required: upper,lower,[#$+./:=?@[^_|~];").unwrap();
        let custom = Custom(BTreeSet::from([
            '#', '$', '+', '.', '/', ':', '=', '?', '@', '[', '^', '_', '|', '~',
        ]));
        assert_eq!(
            rules.required_clauses,
            vec![vec![Digit], vec![Upper, Lower, custom.clone()]]
        );
        assert!(rules.allowed_classes.contains(&custom));
    }

    #[test]
    fn doubled_closing_bracket_is_literal_bracket() {
        let rules = parse_password_rules(r##"minlength: 8; required: digit; required: [- !"#$&'()*+,.:;<=>?@[^_`{|}~]]; allowed: lower, upper;"##).unwrap();
        let members = BTreeSet::from([
            '-', ' ', '!', '"', '#', '$', '&', '\'', '(', ')', '*', '+', ',', '.', ':', ';', '<',
            '=', '>', '?', '@', '[', '^', '_', '`', '{', '|', '}', '~', ']',
        ]);
        assert_eq!(members.len(), 30);
        assert_eq!(rules.min_length, Some(8));
        assert_eq!(
            rules.required_clauses,
            vec![vec![Digit], vec![Custom(members)]]
        );
        assert!(rules.allowed_classes.contains(&Lower));
        assert!(rules.allowed_classes.contains(&Upper));
    }

    #[test]
    fn allowed_is_cumulative_across_multiple_clauses() {
        let rules = parse_password_rules(
            "required: upper; allowed: upper; allowed: lower; minlength: 12; maxlength: 73;",
        )
        .unwrap();
        assert_eq!(rules.required_clauses, vec![vec![Upper]]);
        assert_eq!(rules.allowed_classes, vec![Upper, Lower]);
        assert_eq!(rules.min_length, Some(12));
        assert_eq!(rules.max_length, Some(73));
    }

    #[test]
    fn required_class_folded_into_allowed_even_without_an_allowed_property() {
        let rules = parse_password_rules("required: upper; required: upper; required: digit; required: digit; required: special; required: special; allowed: lower;").unwrap();
        assert_eq!(
            rules.required_clauses,
            vec![
                vec![Upper],
                vec![Upper],
                vec![Digit],
                vec![Digit],
                vec![Special],
                vec![Special]
            ]
        );
        assert_eq!(
            rules.allowed_classes.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([Lower, Upper, Digit, Special])
        );
    }

    #[test]
    fn conflicting_max_consecutive_resolves_to_minimum() {
        assert_eq!(
            parse_password_rules("max-consecutive: 5; max-consecutive: 3;")
                .unwrap()
                .max_consecutive,
            Some(3)
        );
    }
    #[test]
    fn conflicting_min_length_resolves_to_maximum() {
        assert_eq!(
            parse_password_rules("minlength: 12; minlength: 7; minlength: 23;")
                .unwrap()
                .min_length,
            Some(23)
        );
    }
    #[test]
    fn conflicting_max_length_resolves_to_minimum() {
        assert_eq!(
            parse_password_rules("maxlength: 10; maxlength: 7; maxlength: 12;")
                .unwrap()
                .max_length,
            Some(7)
        );
    }
    #[test]
    fn defaults_to_ascii_printable_when_nothing_is_allowed_explicitly() {
        let rules = parse_password_rules("minlength: 6; maxlength: 16;").unwrap();
        assert_eq!(rules.allowed_classes, vec![AsciiPrintable]);
        assert!(rules.required_clauses.is_empty());
    }
    #[test]
    fn max_repeating_and_max_sequential_are_parsed_and_stored() {
        let rules =
            parse_password_rules("max-consecutive: 2; max-repeating: 3; max-sequential: 4;")
                .unwrap();
        assert_eq!(rules.max_consecutive, Some(2));
        assert_eq!(rules.max_repeating, Some(3));
        assert_eq!(rules.max_sequential, Some(4));
    }
    #[test]
    fn empty_input_is_rejected() {
        for raw in ["", "   "] {
            assert_eq!(
                parse_password_rules(raw),
                Err(PasswordRulesParseError::EmptyInput)
            );
        }
    }
    #[test]
    fn unknown_property_name_is_rejected() {
        assert_eq!(
            parse_password_rules("foo: bar;"),
            Err(PasswordRulesParseError::UnknownProperty("foo".into()))
        );
        assert_eq!(
            parse_password_rules("no colon"),
            Err(PasswordRulesParseError::UnknownProperty("no colon".into()))
        );
    }
    #[test]
    fn malformed_numeric_value_is_rejected() {
        assert_eq!(
            parse_password_rules("minlength: abc;"),
            Err(PasswordRulesParseError::MalformedValue {
                property: "minlength".into(),
                value: "abc".into()
            })
        );
    }
    #[test]
    fn unterminated_custom_class_is_rejected() {
        assert_eq!(
            parse_password_rules("required: [abc"),
            Err(PasswordRulesParseError::UnterminatedCustomClass)
        );
    }
    #[test]
    fn parse_error_cases_do_not_panic() {
        for raw in [
            "required: ;".to_owned(),
            "required: [".into(),
            "minlength: -1;".into(),
            "minlength: 4294967296;".into(),
            "required: \0;".into(),
            "required: \u{e9};".into(),
            "x".repeat(10_000),
            format!("minlength: {};", "9".repeat(10_000)),
        ] {
            assert!(parse_password_rules(&raw).is_err(), "{raw:?}");
        }
    }
    #[test]
    fn case_whitespace_empty_and_duplicate_class_semantics() {
        let rules = parse_password_rules(" ; ; MiNlEnGtH : 0; REQUIRED: UPPER,\nlower,; allowed: unicode,ascii-printable; allowed: unicode; ;").unwrap();
        assert_eq!(rules.min_length, Some(0));
        assert_eq!(rules.required_clauses, vec![vec![Upper, Lower]]);
        assert_eq!(
            rules.allowed_classes,
            vec![Unicode, AsciiPrintable, Upper, Lower]
        );
        assert_eq!(
            parse_password_rules("; ;").unwrap(),
            PasswordRules {
                allowed_classes: vec![AsciiPrintable],
                ..Default::default()
            }
        );
        assert_eq!(
            parse_password_rules("required: [],upper")
                .unwrap()
                .required_clauses,
            vec![vec![Upper]]
        );
        for raw in [
            "required: [];",
            "required: upper,,lower;",
            "required: mystery;",
            "required: [a]upper;",
        ] {
            assert!(parse_password_rules(raw).is_err(), "{raw}");
        }
    }
    #[test]
    fn custom_class_filters_nonprintable_and_noninitial_hyphen() {
        let rules = parse_password_rules("required: [-a-b\0\u{e9}];").unwrap();
        assert_eq!(
            rules.required_clauses,
            vec![vec![Custom(BTreeSet::from(['-', 'a', 'b']))]]
        );
    }
    #[test]
    fn numeric_boundaries_and_all_duplicate_bounds() {
        let rules = parse_password_rules("minlength: +0; maxlength: 4294967295; max-repeating: 5; max-repeating: 2; max-repeating: 4; max-sequential: 3; max-sequential: 1; max-sequential: 2;").unwrap();
        assert_eq!(rules.min_length, Some(0));
        assert_eq!(rules.max_length, Some(u32::MAX));
        assert_eq!(rules.max_repeating, Some(2));
        assert_eq!(rules.max_sequential, Some(1));
        for raw in ["minlength: ;", "minlength: 1.5;", "minlength: 1 2;"] {
            assert!(parse_password_rules(raw).is_err());
        }
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    #[test]
    fn hostile_rule_shapes_are_bounded_by_input() {
        assert!(parse_password_rules(&format!("allowed: [{}", "a".repeat(60_000))).is_err());
        assert!(parse_password_rules(&format!("allowed: [{}];", "a".repeat(10_000))).is_ok());
        assert_eq!(
            parse_password_rules(&"required: digit;".repeat(3000))
                .unwrap()
                .required_clauses
                .len(),
            3000
        );
        assert!(parse_password_rules("minlength: -0;").is_err());
        assert!(parse_password_rules("minlength: 4294967296;").is_err());
    }
}
