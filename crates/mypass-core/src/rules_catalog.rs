//! Host lookup over Apple's bundled password-rules catalog.
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::{parse_password_rules, PasswordRules};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogEntry {
    raw: String,
    exact_domain_match_only: bool,
}

/// Raw rules indexed by lowercase domain.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PasswordRulesIndex {
    entries: HashMap<String, CatalogEntry>,
}

impl PasswordRulesIndex {
    /// Construct an index whose entries apply to subdomains too.
    pub fn from_entries(entries: HashMap<String, String>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(domain, raw)| {
                    (
                        domain.to_lowercase(),
                        CatalogEntry {
                            raw,
                            exact_domain_match_only: false,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Decode Apple's domain-to-object JSON catalog, ignoring entries without a string rule.
    /// The optional `exact-domain-match-only` boolean restricts an entry to its exact host.
    /// Invalid JSON or a non-object root returns a decoding error. CRLF is accepted.
    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        let objects: HashMap<String, serde_json::Value> = serde_json::from_str(raw)?;
        let entries = objects
            .into_iter()
            .filter_map(|(domain, value)| {
                let raw = value.get("password-rules")?.as_str()?.to_owned();
                let exact_domain_match_only = value
                    .get("exact-domain-match-only")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                Some((
                    domain.to_lowercase(),
                    CatalogEntry {
                        raw,
                        exact_domain_match_only,
                    },
                ))
            })
            .collect();
        Ok(Self { entries })
    }

    /// Iterate the lowercase domain and raw rule of each usable entry, in unspecified order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(domain, entry)| (domain.as_str(), entry.raw.as_str()))
    }

    /// Return the most specific applicable, valid rule for a host.
    /// Malformed rules behave as absent, allowing lookup to continue at a parent.
    /// Until Task 14 wires in the PSL, lookup stops before a single-label suffix.
    pub fn rules_for_host(&self, host: &str) -> Option<PasswordRules> {
        let normalized = normalize(host)?;
        let mut candidate = normalized.as_str();
        while candidate.contains('.') {
            let is_exact_host = candidate == normalized;
            if let Some(entry) = self.entries.get(candidate) {
                if is_exact_host || !entry.exact_domain_match_only {
                    if let Ok(rules) = parse_password_rules(&entry.raw) {
                        return Some(rules);
                    }
                }
            }
            candidate = candidate.split_once('.')?.1;
        }
        None
    }
}

fn normalize(host: &str) -> Option<String> {
    let lowered = host.to_lowercase();
    let normalized = lowered.strip_suffix('.').unwrap_or(&lowered);
    if normalized.is_empty()
        || normalized
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | ':'))
    {
        return None;
    }
    Some(normalized.into())
}

/// Look up a host in the lazily initialized bundled catalog.
/// A corrupt bundled JSON file degrades to an empty index rather than panicking.
/// Resource tests separately assert that the actual snapshot loads and every rule parses.
pub fn bundled_rules_for_host(host: &str) -> Option<PasswordRules> {
    static INDEX: OnceLock<PasswordRulesIndex> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            PasswordRulesIndex::from_json(include_str!("../resources/password-rules.json"))
                .unwrap_or_default()
        })
        .rules_for_host(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CharacterClass::*;
    fn fixture(domain: &str, raw: &str) -> PasswordRulesIndex {
        PasswordRulesIndex::from_entries(HashMap::from([(domain.into(), raw.into())]))
    }
    #[test]
    fn subdomain_walk_finds_parent_rule() {
        let index = fixture("example.com", "minlength: 10; required: digit;");
        let rules = index.rules_for_host("login.accounts.example.com").unwrap();
        assert_eq!(rules.min_length, Some(10));
        assert_eq!(rules.required_clauses, vec![vec![Digit]]);
    }
    #[test]
    #[ignore = "needs psl (Task 14)"]
    fn lookup_stops_at_the_registrable_domain() {
        assert_eq!(
            fixture("co.uk", "minlength: 20;").rules_for_host("attacker.example.co.uk"),
            None
        );
    }
    #[test]
    #[ignore = "needs psl (Task 14)"]
    fn lookup_stops_at_a_three_label_suffix() {
        assert_eq!(
            fixture("service.gov.uk", "minlength: 20;")
                .rules_for_host("attacker.example.service.gov.uk"),
            None
        );
    }
    #[test]
    fn trailing_dot_normalized() {
        assert_eq!(
            fixture("example.com", "minlength: 10;")
                .rules_for_host("example.com.")
                .unwrap()
                .min_length,
            Some(10)
        );
    }
    #[test]
    fn unknown_host_none() {
        assert_eq!(
            fixture("example.com", "minlength: 10;").rules_for_host("totally-unrelated.org"),
            None
        );
    }
    #[test]
    fn exact_domain_only_entry_does_not_match_subdomain() {
        let index = PasswordRulesIndex::from_json(r#"{"accounts.example.com":{"password-rules":"minlength: 12;","exact-domain-match-only":true}}"#).unwrap();
        assert_eq!(
            index
                .rules_for_host("accounts.example.com")
                .unwrap()
                .min_length,
            Some(12)
        );
        assert_eq!(index.rules_for_host("login.accounts.example.com"), None);
    }
    #[test]
    fn malformed_rule_yields_none() {
        assert_eq!(
            fixture("example.com", "this-is-not-a-recognized-property: 1;")
                .rules_for_host("example.com"),
            None
        );
    }
    #[test]
    fn catalog_reads_real_bundled_resource() {
        assert!(bundled_rules_for_host("163.com").is_some());
        let rules = bundled_rules_for_host("account.samsung.com").unwrap();
        assert_eq!(rules.min_length, Some(8));
        assert_eq!(rules.max_length, Some(15));
        assert_eq!(rules.max_consecutive, Some(3));
        assert_eq!(
            rules.required_clauses,
            vec![vec![Digit], vec![Special], vec![Upper, Lower]]
        );
        assert_eq!(
            rules.allowed_classes,
            vec![AsciiPrintable, Digit, Special, Upper, Lower]
        );
    }
    #[test]
    fn normalization_validation_and_bare_tld_stop() {
        let index = fixture("EXAMPLE.COM", "minlength: 10;");
        assert_eq!(
            index
                .rules_for_host("LOGIN.EXAMPLE.COM.")
                .unwrap()
                .min_length,
            Some(10)
        );
        for host in [
            "",
            ".",
            "example.com..",
            " example.com",
            "login. example.com",
            "example.com/path",
            "example.com:443",
            "https://example.com",
        ] {
            assert_eq!(index.rules_for_host(host), None, "{host}");
        }
        let tld = fixture("com", "minlength: 99;");
        assert_eq!(tld.rules_for_host("a.example.com"), None);
        assert_eq!(tld.rules_for_host("com"), None);
    }
    #[test]
    fn json_loader_filters_entries_and_preserves_flags() {
        let index = PasswordRulesIndex::from_json(r#"{"a.com":{"password-rules":"minlength: 7;"},"b.com":{},"c.com":{"password-rules":7},"d.com":null}"#).unwrap();
        assert_eq!(
            index.entries().collect::<Vec<_>>(),
            vec![("a.com", "minlength: 7;")]
        );
        assert_eq!(
            index.rules_for_host("sub.a.com").unwrap().min_length,
            Some(7)
        );
        assert!(PasswordRulesIndex::from_json("{").is_err());
        assert!(PasswordRulesIndex::from_json("[]").is_err());
    }
    #[test]
    fn invalid_specific_rule_falls_back_to_valid_parent() {
        let index = PasswordRulesIndex::from_entries(HashMap::from([
            ("login.example.com".into(), "bad: 1;".into()),
            ("example.com".into(), "minlength: 11;".into()),
        ]));
        assert_eq!(
            index
                .rules_for_host("login.example.com")
                .unwrap()
                .min_length,
            Some(11)
        );
    }
}
