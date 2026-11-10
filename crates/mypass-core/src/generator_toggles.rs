//! The single rule evaluator used by generation and toggle presentation.
use crate::{CharacterClass, GeneratorOptions, PasswordRules};
use std::collections::BTreeSet;
use CharacterClass::{AsciiPrintable, Custom, Digit, Lower, Special, Unicode, Upper};
/// Display order of the four user-facing toggles.
pub const TOGGLE_BACKED_CLASSES: [CharacterClass; 4] = [Lower, Upper, Digit, Special];
/// Non-ASCII characters belong to the symbol toggle, including Unicode letters.
pub fn toggle_backed_class_of(c: char) -> CharacterClass {
    if c.is_ascii_lowercase() {
        Lower
    } else if c.is_ascii_uppercase() {
        Upper
    } else if c.is_ascii_digit() {
        Digit
    } else {
        Special
    }
}
/// None means no restriction. Custom sets permit only their own members,
/// even when they cause the symbol toggle to appear permitted.
pub fn permitted_toggle_classes(rules: &PasswordRules) -> Option<BTreeSet<CharacterClass>> {
    // `unicode` is a superset of `ascii-printable` in Apple's grammar: it lifts the restriction too.
    if rules.allowed_classes.is_empty()
        || rules.allowed_classes.contains(&AsciiPrintable)
        || rules.allowed_classes.contains(&Unicode)
    {
        return None;
    }
    let mut result: BTreeSet<_> = TOGGLE_BACKED_CLASSES
        .into_iter()
        .filter(|c| rules.allowed_classes.contains(c))
        .collect();
    for class in &rules.allowed_classes {
        if let Custom(members) = class {
            result.extend(members.iter().copied().map(toggle_backed_class_of));
        }
    }
    Some(result)
}
/// Required named classes override preferences. A sole permitted class is
/// also forced on: otherwise a perfectly valid six-digit PIN rule can fail
/// in a bridge with saved digits-off preferences but succeed in the UI.
pub fn forced_on_toggle_classes(rules: &PasswordRules) -> BTreeSet<CharacterClass> {
    let mut result: BTreeSet<_> = rules
        .required_clauses
        .iter()
        .flatten()
        .filter(|c| TOGGLE_BACKED_CLASSES.contains(c))
        .cloned()
        .collect();
    if let Some(permitted) = permitted_toggle_classes(rules) {
        if permitted.len() == 1 {
            result.extend(permitted);
        }
    }
    result
}
/// Toggles completely excluded by the rule.
pub fn excluded_toggle_classes(rules: &PasswordRules) -> BTreeSet<CharacterClass> {
    permitted_toggle_classes(rules)
        .map(|p| {
            TOGGLE_BACKED_CLASSES
                .into_iter()
                .filter(|c| !p.contains(c))
                .collect()
        })
        .unwrap_or_default()
}
/// Derive options without mutating the user's saved choice. Length is clamped
/// to the site's bounds here; generation additionally enforces its safety cap.
pub fn effective_options(base: &GeneratorOptions, rules: &PasswordRules) -> GeneratorOptions {
    let forced = forced_on_toggle_classes(rules);
    let excluded = excluded_toggle_classes(rules);
    let resolve = |c, on| forced.contains(&c) || (on && !excluded.contains(&c));
    let mut length = base.length;
    if let Some(min) = rules.min_length {
        length = length.max(min as usize);
    }
    if let Some(max) = rules.max_length {
        length = length.min(max as usize);
    }
    GeneratorOptions {
        length,
        lowercase: resolve(Lower, base.lowercase),
        uppercase: resolve(Upper, base.uppercase),
        digits: resolve(Digit, base.digits),
        symbols: resolve(Special, base.symbols),
    }
}
#[cfg(test)]
mod tests;
