//! Public Suffix List matching over canonical ACE hosts.
use std::collections::HashSet;
use std::sync::OnceLock;

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

/// Exact, wildcard-parent and exception rules stored as lowercase ACE names.
#[derive(Debug)]
pub struct PublicSuffixList {
    exact: HashSet<String>,
    wildcard: HashSet<String>,
    exception: HashSet<String>,
}

impl PublicSuffixList {
    /// Parse every rule or reject the entire registry. Dropping an invalid rule
    /// would widen matching through the implicit `*` fallback.
    pub fn parse(data: &str) -> Option<Self> {
        let mut rules = Self {
            exact: HashSet::new(),
            wildcard: HashSet::new(),
            exception: HashSet::new(),
        };
        for line in data.lines() {
            let Some(token) = line.split_whitespace().next() else {
                continue;
            };
            if token.starts_with("//") {
                continue;
            }
            let (set, name) = if let Some(name) = token.strip_prefix('!') {
                (&mut rules.exception, name)
            } else if let Some(name) = token.strip_prefix("*.") {
                (&mut rules.wildcard, name)
            } else {
                (&mut rules.exact, token)
            };
            set.insert(canonical_host(name)?);
        }
        Some(rules)
    }

    /// Longest matching suffix; exceptions override wildcards and exact rules.
    /// Canonical hosts are capped at 253 bytes with labels of at most 63 bytes,
    /// bounding repeated hashing of the borrowed candidate suffixes.
    pub fn public_suffix(&self, host: &str) -> Option<String> {
        let normalized = canonical_host(host)?;
        let mut candidate = normalized.as_str();
        let mut longest = None;
        loop {
            let parent = candidate.split_once('.').map(|(_, parent)| parent);
            if self.exception.contains(candidate) {
                return Some(parent.unwrap_or_default().to_owned());
            }
            if longest.is_none()
                && (self.exact.contains(candidate)
                    || parent.is_some_and(|parent| self.wildcard.contains(parent)))
            {
                longest = Some(candidate);
            }
            match parent {
                Some(parent) => candidate = parent,
                None => break,
            }
        }
        // The implicit `*` rule covers the final label of an unknown TLD.
        Some(longest.unwrap_or(candidate).to_owned())
    }

    /// Public suffix plus one label, or None for a suffix or a host exceeding
    /// the canonical 253-byte host / 63-byte label limits.
    pub fn registrable_domain(&self, host: &str) -> Option<String> {
        let normalized = canonical_host(host)?;
        let suffix = self.public_suffix(&normalized)?;
        let prefix = normalized.strip_suffix(&suffix)?.strip_suffix('.')?;
        let label = prefix.rsplit('.').next()?;
        Some(format!("{label}.{suffix}"))
    }
}

/// Canonical lowercase ACE host, independent of the registry. As in Swift,
/// ASCII input (including IP literals and existing ACE) is not IDNA-validated.
/// After stripping one optional trailing dot, the final ACE form must be at most
/// 253 bytes, with nonempty labels of at most 63 bytes, including for ASCII.
/// Non-ASCII input rejects `<`, `>`, `^`, `|` via the URL deny-list;
/// ASCII passes them (Swift parity).
pub fn canonical_host(host: &str) -> Option<String> {
    let ace = if host.is_ascii() {
        host.to_ascii_lowercase()
    } else {
        // Disable STD3 restrictions (underscores are used by real hosts), but
        // refuse URL delimiters rather than reinterpreting them as host data.
        // The URL deny list also catches delimiters introduced by IDNA mapping;
        // unlike STD3 it permits underscores. Uts46 is non-transitional.
        if host.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '/' | ':' | '@' | '?' | '#' | '%' | '[' | ']' | '\\')
        }) {
            return None;
        }
        Uts46::new()
            .to_ascii(
                host.as_bytes(),
                AsciiDenyList::URL,
                Hyphens::Allow,
                DnsLength::VerifyAllowRootDot,
            )
            .ok()?
            .into_owned()
    };
    let body = ace.strip_suffix('.').unwrap_or(&ace);
    if body.len() > 253
        || body
            .split('.')
            .any(|label| label.is_empty() || label.len() > 63)
    {
        return None;
    }
    Some(body.to_owned())
}

/// Parse the bundled ICANN and PRIVATE rules once. A failed load stays unavailable,
/// never degrades to an empty registry that could widen domain matches.
pub fn bundled() -> Option<&'static PublicSuffixList> {
    static RULES: OnceLock<Option<PublicSuffixList>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            PublicSuffixList::parse(include_str!("../resources/public_suffix_list.dat"))
        })
        .as_ref()
}

/// Bundled-registry public suffix lookup.
pub fn public_suffix(host: &str) -> Option<String> {
    bundled()?.public_suffix(host)
}
/// Bundled-registry registrable domain lookup.
pub fn registrable_domain(host: &str) -> Option<String> {
    bundled()?.registrable_domain(host)
}

#[cfg(test)]
mod tests {
    // Test-only support code: assertions may unwrap known fixtures.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    const DATA: &str = include_str!("../resources/public_suffix_list.dat");
    const COMPANY: &str = "\u{516c}\u{53f8}";
    fn check(host: &str, suffix: &str, domain: Option<&str>) {
        assert_eq!(public_suffix(host).as_deref(), Some(suffix), "{host}");
        assert_eq!(registrable_domain(host).as_deref(), domain, "{host}");
    }
    #[test]
    fn oversized_hosts_fail_closed() {
        for host in [format!("{}com", "a.".repeat(100_000)), "a".repeat(40_000)] {
            // Check canonicalization first so RED never performs the unbounded walk.
            assert!(canonical_host(&host).is_none());
            assert!(public_suffix(&host).is_none());
            assert!(registrable_domain(&host).is_none());
        }
    }
    #[test]
    fn host_length_boundary() {
        let host = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        );
        assert_eq!(host.len(), 253);
        assert_eq!(canonical_host(&host).as_deref(), Some(host.as_str()));
        assert_eq!(
            canonical_host(&format!("{host}.")).as_deref(),
            Some(host.as_str())
        );
        assert!(public_suffix(&host).is_some());
        assert!(registrable_domain(&host).is_some());
        let oversized = format!("{host}d");
        assert_eq!(oversized.len(), 254);
        assert!(canonical_host(&oversized).is_none());
        assert!(public_suffix(&oversized).is_none());
        assert!(registrable_domain(&oversized).is_none());
    }
    #[test]
    fn label_length_boundary() {
        let host = format!("{}.com", "a".repeat(63));
        assert_eq!(canonical_host(&host).as_deref(), Some(host.as_str()));
        check(&host, "com", Some(&host));
        let oversized = format!("{}.com", "a".repeat(64));
        assert!(canonical_host(&oversized).is_none());
        assert!(public_suffix(&oversized).is_none());
        assert!(registrable_domain(&oversized).is_none());
    }
    #[test]
    fn idna_final_ace_length_boundaries() {
        // U+FF41 maps to ASCII a, so the raw input can exceed the ACE byte cap.
        let valid = format!(
            "{}.{}.{}.{}",
            "\u{ff41}".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        );
        let ace = canonical_host(&valid).unwrap();
        assert_eq!(ace.len(), 253);
        assert_eq!(
            canonical_host(&format!("{valid}.")).as_deref(),
            Some(ace.as_str())
        );
        assert!(canonical_host(&format!("{valid}d")).is_none());
        assert!(canonical_host(&format!("{}.com", "\u{ff41}".repeat(64))).is_none());
        // Punycode expansion also counts toward the final label limit.
        assert!(canonical_host(&format!("{}.de", "\u{fc}".repeat(57))).is_some());
        assert!(canonical_host(&format!("{}.de", "\u{fc}".repeat(58))).is_none());
    }
    #[test]
    fn unicode_hyphens_are_allowed() {
        for (host, expected) in [
            ("-b\u{fc}cher.de", "xn---bcher-4ya.de"),
            ("b\u{fc}cher-.de", "xn--bcher--3ya.de"),
        ] {
            assert_eq!(canonical_host(host).as_deref(), Some(expected));
        }
    }
    #[test]
    fn url_deny_list_preserves_ascii_parity() {
        for c in ['<', '>', '^', '|'] {
            let ascii = format!("a{c}.de");
            assert_eq!(canonical_host(&ascii).as_deref(), Some(ascii.as_str()));
            assert!(canonical_host(&format!("b\u{fc}cher{c}.de")).is_none());
        }
    }
    #[test]
    fn exact_rule_com() {
        check("example.com", "com", Some("example.com"));
    }
    #[test]
    fn subdomain_reduces_to_registrable_domain() {
        check("www.example.com", "com", Some("example.com"));
    }
    #[test]
    fn multi_label_co_uk() {
        check("accounts.google.co.uk", "co.uk", Some("google.co.uk"));
    }
    #[test]
    fn multi_label_com_ua() {
        check("shop.example.com.ua", "com.ua", Some("example.com.ua"));
    }
    #[test]
    fn private_section_github_io() {
        check("foo.github.io", "github.io", Some("foo.github.io"));
    }
    #[test]
    fn implicit_rule_for_unknown_tld() {
        check("foo.bar.unknowntld", "unknowntld", Some("bar.unknowntld"));
    }
    #[test]
    fn wildcard_rule() {
        check("a.b.ck", "b.ck", Some("a.b.ck"));
    }
    #[test]
    fn exception_rule() {
        check("www.ck", "ck", Some("www.ck"));
        check("x.www.ck", "ck", Some("www.ck"));
    }
    #[test]
    fn host_that_is_a_public_suffix_has_no_registrable_domain() {
        for h in ["co.uk", "com"] {
            check(h, h, None);
        }
    }
    #[test]
    fn empty_host_is_none() {
        assert_eq!(canonical_host(""), None);
        assert_eq!(public_suffix(""), None);
        assert_eq!(registrable_domain(""), None);
    }
    #[test]
    fn lowercases_and_strips_trailing_dot() {
        check("EXAMPLE.COM.", "com", Some("example.com"));
    }
    #[test]
    fn bundled_unicode_suffix_matches_punycode_host() {
        check(
            "example.xn--55qx5d.hk",
            "xn--55qx5d.hk",
            Some("example.xn--55qx5d.hk"),
        );
        check(
            "www.example.xn--55qx5d.hk",
            "xn--55qx5d.hk",
            Some("example.xn--55qx5d.hk"),
        );
    }
    #[test]
    fn unicode_and_punycode_hosts_are_equivalent() {
        check(
            &format!("example.{COMPANY}.hk"),
            "xn--55qx5d.hk",
            Some("example.xn--55qx5d.hk"),
        );
        check(
            &format!("www.\u{4f8b}\u{3048}.{COMPANY}.hk"),
            "xn--55qx5d.hk",
            Some("xn--r8jz45g.xn--55qx5d.hk"),
        );
        assert_eq!(
            registrable_domain(&format!("www.\u{4f8b}\u{3048}.{COMPANY}.hk")),
            registrable_domain("www.xn--r8jz45g.xn--55qx5d.hk")
        );
    }
    #[test]
    fn sibling_domains_under_unicode_suffix_stay_isolated() {
        for name in ["alpha", "beta"] {
            check(
                &format!("login.{name}.xn--55qx5d.hk"),
                "xn--55qx5d.hk",
                Some(&format!("{name}.xn--55qx5d.hk")),
            );
        }
        for h in [format!("{COMPANY}.hk"), "xn--55qx5d.hk".into()] {
            assert_eq!(registrable_domain(&h), None);
        }
    }
    #[test]
    fn unicode_case_trailing_dot_and_normalization_forms_canonicalised() {
        for h in [
            "Example.XN--55QX5D.HK.".into(),
            format!("EXAMPLE.{COMPANY}.HK."),
            format!("example.{COMPANY}\u{3002}hk"),
        ] {
            check(&h, "xn--55qx5d.hk", Some("example.xn--55qx5d.hk"));
        }
        for h in ["b\u{00fc}cher.example.com", "bu\u{0308}cher.example.com"] {
            check(h, "com", Some("example.com"));
            assert_eq!(
                canonical_host(h).as_deref(),
                Some("xn--bcher-kva.example.com")
            );
        }
        check("b\u{fc}cher.de", "de", Some("xn--bcher-kva.de"));
    }
    #[test]
    fn unicode_label_under_ascii_suffix_returns_ace() {
        for h in ["www.b\u{fc}cher.co.uk", "www.xn--bcher-kva.co.uk"] {
            check(h, "co.uk", Some("xn--bcher-kva.co.uk"));
        }
    }
    #[test]
    fn malformed_non_ascii_hosts_fail_closed() {
        for h in [
            format!("{COMPANY}.hk/path"),
            format!("{COMPANY}.hk:8080"),
            format!("user@{COMPANY}.hk"),
            format!("{COMPANY}.hk?x=1"),
            format!("{COMPANY}.hk#frag"),
            format!("{COMPANY} .hk"),
            format!("a..{COMPANY}.hk"),
            format!(".{COMPANY}.hk"),
            format!("{COMPANY}.hk.."),
            format!("{COMPANY}.hk\n"),
            "\u{516c}\0\u{53f8}.hk".into(),
            format!("{COMPANY}.hk%2f"),
            format!("[{COMPANY}.hk]"),
            format!("{COMPANY}.hk\\x"),
            format!("{}.hk", "\u{516c}".repeat(70)),
        ] {
            assert_eq!(canonical_host(&h), None, "{h:?}");
            assert_eq!(public_suffix(&h), None);
            assert_eq!(registrable_domain(&h), None);
        }
        for h in ["a..b", ".a", "a.."] {
            assert_eq!(canonical_host(h), None);
        }
    }
    #[test]
    fn idna_mapped_url_delimiters_fail_closed() {
        for host in [
            "\u{516c}\u{53f8}\u{ff0f}x.hk",
            "\u{516c}\u{53f8}\u{ff20}x.hk",
            "\u{516c}\u{53f8}\u{ff1a}80.hk",
        ] {
            assert_eq!(canonical_host(host), None);
        }
    }
    #[test]
    fn ascii_and_ip_behaviour_unchanged() {
        for (h, expected) in [
            ("Example.COM.", "example.com"),
            ("localhost", "localhost"),
            ("127.0.0.1", "127.0.0.1"),
            ("::1", "::1"),
            ("_service.Example", "_service.example"),
            ("-host.example", "-host.example"),
            ("xn--invalid-", "xn--invalid-"),
        ] {
            assert_eq!(canonical_host(h).as_deref(), Some(expected));
        }
        assert_eq!(canonical_host("."), None);
        check("localhost", "localhost", None);
        assert_eq!(
            canonical_host("fa\u{df}.de").as_deref(),
            Some("xn--fa-hia.de")
        );
        assert_eq!(
            canonical_host("_service.b\u{fc}cher.de").as_deref(),
            Some("_service.xn--bcher-kva.de")
        );
    }
    #[test]
    fn every_bundled_rule_converts_and_unicode_rules_are_stored_as_ace() {
        let p = PublicSuffixList::parse(DATA).unwrap();
        for set in [&p.exact, &p.wildcard, &p.exception] {
            assert!(set.iter().all(|r| r.is_ascii()));
        }
        assert!(p.exact.contains("xn--55qx5d.hk"));
        assert!(!p.exact.contains(&format!("{COMPANY}.hk")));
    }
    #[test]
    fn custom_unicode_exact_wildcard_and_exception_rules() {
        let p = PublicSuffixList::parse("// comment\nexample\n\u{516c}\u{53f8}.example\n*.\u{65e5}\u{672c}.example\n!\u{4f8b}.\u{65e5}\u{672c}.example\n").unwrap();
        for h in [
            "shop.\u{516c}\u{53f8}.example",
            "shop.xn--55qx5d.example",
            "SHOP.\u{516c}\u{53f8}.EXAMPLE.",
        ] {
            assert_eq!(p.public_suffix(h).as_deref(), Some("xn--55qx5d.example"));
            assert_eq!(
                p.registrable_domain(h).as_deref(),
                Some("shop.xn--55qx5d.example")
            );
        }
        for h in [
            "\u{516c}\u{53f8}.example",
            "xn--55qx5d.example",
            "b.\u{65e5}\u{672c}.example",
        ] {
            assert_eq!(p.registrable_domain(h), None);
        }
        for h in ["a.b.\u{65e5}\u{672c}.example", "a.b.xn--wgv71a.example"] {
            assert_eq!(p.public_suffix(h).as_deref(), Some("b.xn--wgv71a.example"));
            assert_eq!(
                p.registrable_domain(h).as_deref(),
                Some("a.b.xn--wgv71a.example")
            );
        }
        assert_eq!(
            p.registrable_domain("\u{65e5}\u{672c}.example").as_deref(),
            Some("xn--wgv71a.example")
        );
        for h in [
            "\u{4f8b}.\u{65e5}\u{672c}.example",
            "www.\u{4f8b}.\u{65e5}\u{672c}.example",
            "www.xn--fsq.xn--wgv71a.example",
        ] {
            assert_eq!(p.public_suffix(h).as_deref(), Some("xn--wgv71a.example"));
            assert_eq!(
                p.registrable_domain(h).as_deref(),
                Some("xn--fsq.xn--wgv71a.example")
            );
        }
        assert_eq!(
            p.registrable_domain("x.y.\u{65e5}\u{672c}.example")
                .as_deref(),
            Some("x.y.xn--wgv71a.example")
        );
    }
    #[test]
    fn unconvertible_rule_rejects_whole_rule_set() {
        for s in [
            "com\n\u{516c}\u{53f8}/x.hk\n",
            "*.a..\u{516c}\u{53f8}\n",
            "!\u{516c}\u{53f8}.hk:80\n",
            "com\na..b\n",
        ] {
            assert!(PublicSuffixList::parse(s).is_none());
        }
    }
    #[test]
    fn canonical_host_known_answers() {
        for (h, expected) in [
            ("\u{516c}\u{53f8}.hk", "xn--55qx5d.hk"),
            ("XN--55QX5D.HK.", "xn--55qx5d.hk"),
            ("LOCALHOST", "localhost"),
        ] {
            assert_eq!(canonical_host(h).as_deref(), Some(expected));
        }
    }
    #[test]
    fn crlf_rule_file_parses_identically() {
        let lf = PublicSuffixList::parse(DATA).unwrap();
        let crlf = PublicSuffixList::parse(&DATA.replace('\n', "\r\n")).unwrap();
        assert_eq!(lf.exact, crlf.exact);
        assert_eq!(lf.wildcard, crlf.wildcard);
        assert_eq!(lf.exception, crlf.exception);
        for h in [
            "example.com",
            "www.example.com",
            "accounts.google.co.uk",
            "shop.example.com.ua",
            "foo.github.io",
            "foo.bar.unknowntld",
            "a.b.ck",
            "www.ck",
            "co.uk",
            "com",
            "localhost",
            "EXAMPLE.COM.",
            "example.xn--55qx5d.hk",
            "www.example.xn--55qx5d.hk",
            "alpha.xn--55qx5d.hk",
            "beta.xn--55qx5d.hk",
            "b\u{fc}cher.de",
            "www.b\u{fc}cher.co.uk",
            "127.0.0.1",
            "a.b.kawasaki.jp",
        ] {
            assert_eq!(lf.public_suffix(h), crlf.public_suffix(h));
            assert_eq!(lf.registrable_domain(h), crlf.registrable_domain(h));
        }
        assert!(
            PublicSuffixList::parse("  // comment\r\n\r\n com metadata\r\n")
                .unwrap()
                .exact
                .contains("com")
        );
    }
    #[test]
    fn repeated_lookups_are_fast() {
        let first = bundled().unwrap();
        assert!(std::ptr::eq(first, bundled().unwrap()));
        registrable_domain("www.example.co.uk");
        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            assert_eq!(
                registrable_domain("www.example.co.uk").as_deref(),
                Some("example.co.uk")
            );
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
