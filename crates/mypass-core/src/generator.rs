//! Cryptographic password generation with site constraints.
use crate::generator_toggles::{forced_on_toggle_classes, toggle_backed_class_of};
use crate::{CharacterClass, PasswordRules};
use rand::{seq::SliceRandom, CryptoRng, Rng, RngCore};
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;
use CharacterClass::{AsciiPrintable, Custom, Digit, Lower, Special, Upper};
/// Bound allocation and repair work for public API callers, including usize::MAX.
/// Requests are capped; a site minimum above this cap is unsatisfiable.
pub const MAX_PASSWORD_LENGTH: usize = 4096;
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.<>?";
/// Preferences for a single generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratorOptions {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
}
impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: 20,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
        }
    }
}
/// Failures never contain password material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GeneratorError {
    #[error("no character class selected")]
    NoCharacterClassSelected,
    #[error("rules cannot be satisfied")]
    UnsatisfiableRules,
}
use GeneratorError::{NoCharacterClassSelected, UnsatisfiableRules};
struct EffectivePool {
    characters: Vec<char>,
    membership: BTreeMap<CharacterClass, BTreeSet<char>>,
}
fn effective_pool(opts: &GeneratorOptions, rules: &PasswordRules) -> EffectivePool {
    let required: BTreeSet<_> = rules.required_clauses.iter().flatten().cloned().collect();
    let forced = forced_on_toggle_classes(rules);
    let unrestricted =
        rules.allowed_classes.is_empty() || rules.allowed_classes.contains(&AsciiPrintable);
    let toggles = [
        (Lower, opts.lowercase, LOWER),
        (Upper, opts.uppercase, UPPER),
        (Digit, opts.digits, DIGITS),
        (Special, opts.symbols, SYMBOLS),
    ];
    let in_play =
        |c: &CharacterClass| forced.contains(c) || toggles.iter().any(|(cl, on, _)| cl == c && *on);
    let mut membership = BTreeMap::<CharacterClass, BTreeSet<char>>::new();
    for (class, _, baseline) in &toggles {
        if required.contains(class)
            || ((unrestricted || rules.allowed_classes.contains(class)) && in_play(class))
        {
            membership.insert(class.clone(), baseline.chars().collect());
        }
    }
    // Required custom members bypass toggles. Allowed-only custom members
    // obey their backing toggle, and never enable an entire baseline pool.
    for class in rules.allowed_classes.iter().chain(required.iter()) {
        if let Custom(members) = class {
            let selected = members
                .iter()
                .copied()
                .filter(|c| required.contains(class) || in_play(&toggle_backed_class_of(*c)));
            membership
                .entry(class.clone())
                .or_default()
                .extend(selected);
        }
    }
    // Unicode has no invented baseline pool. A required Unicode-only clause
    // remains unsatisfiable, just as in Swift; explicit custom scalars work.
    let characters = membership
        .values()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    EffectivePool {
        characters,
        membership,
    }
}
fn sample<R: CryptoRng + RngCore>(pool: &[char], rng: &mut R) -> Result<char, GeneratorError> {
    if pool.is_empty() {
        return Err(UnsatisfiableRules);
    }
    // gen_range rejects values that would introduce modulo bias for pools
    // whose size does not divide the RNG range. Never replace this with %.
    pool.get(rng.gen_range(0..pool.len()))
        .copied()
        .ok_or(UnsatisfiableRules)
}
fn random_candidate<R: CryptoRng + RngCore>(
    length: usize,
    pool: &[char],
    rng: &mut R,
) -> Result<Zeroizing<Vec<char>>, GeneratorError> {
    let mut result = Zeroizing::new(Vec::with_capacity(length));
    for _ in 0..length {
        result.push(sample(pool, rng)?);
    }
    Ok(result)
}
fn satisfies(candidate: &[char], rules: &PasswordRules, pool: &EffectivePool) -> bool {
    if !candidate.iter().all(|c| pool.characters.contains(c)) {
        return false;
    }
    if !rules.required_clauses.iter().all(|clause| {
        clause.iter().any(|class| {
            pool.membership
                .get(class)
                .is_some_and(|members| candidate.iter().any(|c| members.contains(c)))
        })
    }) {
        return false;
    }
    let mut previous = None;
    let mut run = 0;
    let mut sequential = 1;
    let mut direction = 0i64;
    // Counts are positional rather than a char-keyed map: temporary password
    // characters stay in the zeroizing candidate buffer.
    for &c in candidate {
        run = if previous == Some(c) { run + 1 } else { 1 };
        if rules.max_consecutive.is_some_and(|max| run > max) {
            return false;
        }
        if rules
            .max_repeating
            .is_some_and(|max| candidate.iter().filter(|x| **x == c).count() > max as usize)
        {
            return false;
        }
        if let Some(p) = previous {
            let delta = c as i64 - p as i64;
            sequential = if delta.abs() == 1 {
                if delta == direction {
                    sequential + 1
                } else {
                    2
                }
            } else {
                1
            };
            direction = delta;
        }
        if rules.max_sequential.is_some_and(|max| sequential > max) {
            return false;
        }
        previous = Some(c);
    }
    true
}
/// Repair strictly left to right against the finalized predecessor. Tracking
/// original run boundaries instead can turn aaabb into aabbb: the repair
/// itself creates a new violation at the next boundary. A forward pass
/// re-examines that boundary. Sampling is bounded; final validation rejects
/// degenerate pools with no alternative rather than looping forever.
fn repair_max_consecutive<R: CryptoRng + RngCore>(
    chars: &mut [char],
    max: u32,
    pool: &[char],
    rng: &mut R,
) -> Result<(), GeneratorError> {
    let mut previous = None;
    let mut run = 0;
    for c in chars {
        run = if previous == Some(*c) { run + 1 } else { 1 };
        if run > max {
            for _ in 0..20 {
                *c = sample(pool, rng)?;
                if Some(*c) != previous {
                    break;
                }
            }
            if Some(*c) != previous {
                run = 1;
            }
        }
        previous = Some(*c);
    }
    Ok(())
}
fn deterministic_candidate<R: CryptoRng + RngCore>(
    length: usize,
    clauses: &[Vec<char>],
    pool: &[char],
    max: Option<u32>,
    rng: &mut R,
) -> Result<Zeroizing<Vec<char>>, GeneratorError> {
    let mut chars = Zeroizing::new(Vec::with_capacity(length));
    for clause in clauses {
        chars.push(sample(clause, rng)?);
    }
    while chars.len() < length {
        chars.push(sample(pool, rng)?);
    }
    chars.shuffle(rng);
    if let Some(max) = max {
        repair_max_consecutive(&mut chars, max, pool, rng)?;
    }
    Ok(chars)
}
fn into_password(chars: &[char]) -> Zeroizing<String> {
    // Allocate UTF-8 capacity once so intermediate reallocations do not leave
    // old password fragments behind. Custom classes can contain non-ASCII.
    let mut result = Zeroizing::new(String::with_capacity(
        chars.iter().map(|c| c.len_utf8()).sum(),
    ));
    result.extend(chars.iter());
    result
}
/// Generate with an injected cryptographic RNG. Length is capped at 4096
/// characters. Rules are checked before returning; bounded search may reject
/// a satisfiable but pathologically restrictive synthetic rule.
pub fn generate<R: CryptoRng + RngCore>(
    opts: &GeneratorOptions,
    rules: Option<&PasswordRules>,
    rng: &mut R,
) -> Result<Zeroizing<String>, GeneratorError> {
    let empty = PasswordRules::default();
    let constraints = rules.unwrap_or(&empty);
    if constraints
        .min_length
        .zip(constraints.max_length)
        .is_some_and(|(min, max)| min > max)
        || constraints
            .min_length
            .is_some_and(|min| min as usize > MAX_PASSWORD_LENGTH)
    {
        return Err(UnsatisfiableRules);
    }
    let mut length = opts.length.min(MAX_PASSWORD_LENGTH);
    if let Some(min) = constraints.min_length {
        length = length.max(min as usize);
    }
    if let Some(max) = constraints.max_length {
        length = length.min(max as usize);
    }
    if constraints.required_clauses.len() > length {
        return Err(UnsatisfiableRules);
    }
    let pool = effective_pool(opts, constraints);
    let clauses: Vec<Vec<char>> = constraints
        .required_clauses
        .iter()
        .map(|clause| {
            clause
                .iter()
                .filter_map(|c| pool.membership.get(c))
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .collect();
    if clauses.iter().any(Vec::is_empty) {
        return Err(UnsatisfiableRules);
    }
    if pool.characters.is_empty() {
        return Err(NoCharacterClassSelected);
    }
    if rules.is_none() {
        return Ok(into_password(&random_candidate(
            length,
            &pool.characters,
            rng,
        )?));
    }
    // Honest rejection sampling preserves a random distribution for ordinary
    // site rules. 100 attempts suffice for the shipped catalog. Tight pools
    // can defeat it routinely: inflating this bound merely hides repair bugs.
    for _ in 0..100 {
        let candidate = random_candidate(length, &pool.characters, rng)?;
        if satisfies(&candidate, constraints, &pool) {
            return Ok(into_password(&candidate));
        }
    }
    // Construction guarantees one anchor per required clause, fills and
    // shuffles to hide anchor positions, then repairs consecutive runs. This
    // fallback exists for tight rules, not to weaken validation. Repair can
    // destroy an anchor, so every attempt must pass the complete check again.
    for _ in 0..10 {
        let candidate = deterministic_candidate(
            length,
            &clauses,
            &pool.characters,
            constraints.max_consecutive,
            rng,
        )?;
        if satisfies(&candidate, constraints, &pool) {
            return Ok(into_password(&candidate));
        }
    }
    Err(UnsatisfiableRules)
}
/// Generate using operating-system entropy. OS entropy failure fails closed
/// through rand's OsRng, consistent with the infallible RNG trait contract.
pub fn generate_os(
    opts: &GeneratorOptions,
    rules: Option<&PasswordRules>,
) -> Result<Zeroizing<String>, GeneratorError> {
    generate(opts, rules, &mut rand::rngs::OsRng)
}
#[cfg(test)]
mod tests;
