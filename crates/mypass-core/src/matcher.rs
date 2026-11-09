//! URL-based credential selection, with PSL-aware default matching.
use crate::{
    entry_ext::{EntryExt, EntryType, UriMatchMode, GENERATOR_HISTORY_GROUP_UUID},
    psl,
};
use mypass_kdbx::{Database, Entry};
use url::{Host, Url};

/// Bound URL parser/IDNA work and allocations for untrusted page and vault URLs.
/// 8 KiB accommodates ordinary URLs without allowing unbounded parser input.
pub const MAX_MATCH_URL_BYTES: usize = 8192;

fn parse_lenient(input: &str) -> Option<Url> {
    let input = input.trim();
    if input.len() > MAX_MATCH_URL_BYTES {
        return None;
    }
    if input.is_empty() {
        return None;
    }
    // Url requires brackets around an IPv6 authority; accept bare addresses as well (R13).
    if let Ok(ip) = input.parse::<std::net::Ipv6Addr>() {
        return Url::parse(&format!("https://[{ip}]/")).ok();
    }
    if let Ok(url) = Url::parse(input) {
        if url.has_host() {
            return Some(url);
        }
    }
    // Do not reinterpret a malformed explicit authority as a scheme-less host.
    let authority = input.split(['/', '?', '#']).next()?;
    if authority.ends_with(':')
        && input
            .get(authority.len()..)
            .is_some_and(|tail| tail.starts_with("//"))
    {
        return None;
    }
    Url::parse(&format!("https://{input}"))
        .ok()
        .filter(Url::has_host)
}
// Targets retain explicit parsed schemes, including hostless ones, so a mailto
// URL cannot become an HTTPS authority via the entry-only lenient retry.
fn parse_target(input: &str) -> Option<Url> {
    let input = input.trim();
    if input.len() > MAX_MATCH_URL_BYTES {
        return None;
    }
    if input.parse::<std::net::Ipv6Addr>().is_ok() {
        return parse_lenient(input);
    }
    let host_port = input.split_once(':').is_some_and(|(host, rest)| {
        let hostname =
            host == "localhost" || host.contains('.') || host.parse::<std::net::IpAddr>().is_ok();
        let port = rest.split(['/', '?', '#']).next().unwrap_or_default();
        hostname && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
    });
    if input.contains(':') && !input.contains("://") && !host_port {
        return None;
    }
    let url = match Url::parse(input) {
        Ok(url) if url.has_host() || !host_port => url,
        _ => parse_lenient(input)?,
    };
    (matches!(url.scheme(), "http" | "https") && url.has_host()).then_some(url)
}
fn host(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Ipv4(ip) => Some(ip.to_string()),
        Host::Ipv6(ip) => Some(ip.to_string()),
        Host::Domain(s) => Some(s.strip_suffix('.').unwrap_or(s).to_owned()),
    }
}
fn domain(url: &Url) -> Option<String> {
    let host = host(url)?;
    match url.host()? {
        Host::Domain(_) => psl::registrable_domain(&host).or(Some(host)),
        _ => Some(host),
    }
}
/// Returns the registrable domain, or the whole canonical host for IPs/bare suffixes.
/// Uses the same scheme-agnostic lenient parsing as stored entry URLs.
pub fn registrable_domain_of(url_or_host: &str) -> Option<String> {
    domain(&parse_lenient(url_or_host)?)
}
/// Returns the complete canonical host without port or IPv6 brackets.
/// Stored entry URLs use this lenient, scheme-agnostic parsing too: for Swift
/// parity, `mailto:a@example.com` retries as an HTTPS authority with host `example.com`.
/// Credential targets instead require a parsed HTTP(S) scheme and a host.
pub fn host_of(url_or_host: &str) -> Option<String> {
    host(&parse_lenient(url_or_host)?)
}
fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && host(a) == host(b) && a.port() == b.port()
}
fn rest(url: &Url) -> String {
    // Excludes credentials, origin and fragment. Url normalises an empty web path
    // to '/', so exact mode intentionally equates https://x and https://x/.
    let mut rest = url.path().to_owned();
    if let Some(query) = url.query() {
        rest.push('?');
        rest.push_str(query);
    }
    rest
}
fn matches_parsed(entry: &Entry, target: &Url) -> bool {
    let mode = entry.uri_match_mode();
    if mode == UriMatchMode::Never {
        return false;
    }
    let target_domain = if mode == UriMatchMode::RegistrableDomain {
        domain(target)
    } else {
        None
    };
    entry
        .urls()
        .iter()
        .filter_map(|s| parse_lenient(s))
        .any(|url| {
            match mode {
                UriMatchMode::Never => false,
                UriMatchMode::RegistrableDomain => {
                    target_domain.is_some() && domain(&url) == target_domain
                }
                UriMatchMode::Host => host(&url) == host(target) && url.port() == target.port(),
                UriMatchMode::Exact => same_origin(&url, target) && rest(&url) == rest(target),
                UriMatchMode::StartsWith => {
                    // Decision 9 accepts /app matching /app-legacy: the user chooses
                    // a string prefix within their site's path layout. Cross-origin
                    // overmatch is never accepted: https://example.com must not offer
                    // credentials to https://example.com.attacker.com. Compare the
                    // origin (scheme, host, non-default port) BEFORE applying the prefix
                    // to path + query only; never prefix-match the concatenated URL.
                    same_origin(&url, target) && rest(target).starts_with(&rest(&url))
                }
            }
        })
}
/// Matches one entry's primary or additional URLs using its stored mode.
/// Targets must parse to HTTP(S) with a host; entry URLs remain scheme-agnostic.
pub fn matches(entry: &Entry, target_url: &str) -> bool {
    parse_target(target_url).is_some_and(|target| matches_parsed(entry, &target))
}
/// Filters entries in input order, parsing the target once.
pub fn matching_entries<'a>(entries: &[&'a Entry], target_url: &str) -> Vec<&'a Entry> {
    let Some(target) = parse_target(target_url) else {
        return vec![];
    };
    entries
        .iter()
        .copied()
        .filter(|e| matches_parsed(e, &target))
        .collect()
}
/// Finds credentials in document order, pruning recycle/history groups and non-credential types.
pub fn credential_candidates<'a>(db: &'a Database, target_url: &str) -> Vec<&'a Entry> {
    let mut excluded = vec![GENERATOR_HISTORY_GROUP_UUID];
    excluded.extend(db.meta.recycle_bin_uuid);
    let entries: Vec<_> = db
        .entries_excluding(&excluded)
        .into_iter()
        .filter(|e| !matches!(e.entry_type(), EntryType::Address | EntryType::Files))
        .collect();
    matching_entries(&entries, target_url)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry_ext::{
        EntryExt, UriMatchMode, ENTRY_TYPE_FIELD, GENERATOR_HISTORY_GROUP_UUID,
    };
    use mypass_kdbx::{Group, Timestamp};
    fn make_entry(url: &str, title: &str) -> Entry {
        let mut e = Entry::new(Timestamp(0));
        e.set_field("URL", url, false);
        e.set_field("Title", title, false);
        e
    }
    fn check(mode: UriMatchMode, entry: &str, target: &str, expected: bool) {
        let mut e = make_entry(entry, "Entry");
        e.set_uri_match_mode(mode);
        assert_eq!(
            matches(&e, target),
            expected,
            "{mode:?}: {entry} -> {target}"
        );
        assert_eq!(matching_entries(&[&e], target).len(), usize::from(expected));
    }
    fn rd(s: &str) -> Option<String> {
        registrable_domain_of(s)
    }
    #[test]
    fn cleanup_target_host_shapes() {
        for ip in ["fe80::1", "abcd::1", "fd00::1"] {
            check(UriMatchMode::Host, &format!("https://[{ip}]/"), ip, true);
        }
    }
    #[test]
    fn cleanup_scheme_tokens_are_not_host_ports() {
        for target in ["javascript:80", "ftp:", "ftp:80", "custom:123"] {
            assert!(parse_target(target).is_none(), "{target}");
        }
        for target in ["example.com:80", "localhost:8080", "127.0.0.1:80"] {
            assert!(parse_target(target).is_some(), "{target}");
        }
    }
    #[test]
    fn targets_require_parsed_http_scheme_in_every_selection_api() {
        for mode in UriMatchMode::ALL {
            let mut e = make_entry("https://example.com/", "Entry");
            e.set_uri_match_mode(mode);
            let mut db = Database::new("test");
            db.root.entries.push(e.clone());
            for target in [
                "java\tscript://example.com/",
                "java\nscript://example.com/",
                "\u{1}javascript://example.com/",
                "\u{1}file://example.com/x",
                "blob://example.com/",
                "chrome://example.com/",
                "ftp://example.com/",
                "ws://example.com/",
                "foo://example.com/",
                "mailto:a@example.com",
                "data:text/plain,hello",
                "javascript:alert(1)",
                "file:///x",
            ] {
                check(mode, "https://example.com/", target, false);
                // Equal non-web entry/target origins must also fail Exact/StartsWith.
                check(mode, target, target, false);
                assert!(
                    credential_candidates(&db, target).is_empty(),
                    "{mode:?}: {target:?}"
                );
            }
            for target in [
                "https://example.com/",
                "HTTPS://EXAMPLE.COM/",
                "\u{1}https://example.com/",
                "h\tttps://example.com/",
                "h\nttps://example.com/",
            ] {
                check(
                    mode,
                    "https://example.com/",
                    target,
                    mode != UriMatchMode::Never,
                );
                assert_eq!(
                    credential_candidates(&db, target).len(),
                    usize::from(mode != UriMatchMode::Never)
                );
            }
            check(
                mode,
                "http://example.com/",
                "http://example.com/",
                mode != UriMatchMode::Never,
            );
        }
    }
    #[test]
    fn trailing_root_dot_is_canonical_in_host_exact_and_prefix_modes() {
        for mode in [
            UriMatchMode::Host,
            UriMatchMode::Exact,
            UriMatchMode::StartsWith,
        ] {
            for host in ["example.com", "127.0.0.1"] {
                check(
                    mode,
                    &format!("https://{host}"),
                    &format!("https://{host}./"),
                    true,
                );
                check(
                    mode,
                    &format!("https://{host}./"),
                    &format!("https://{host}/"),
                    true,
                );
            }
        }
    }
    #[test]
    fn lenient_retry_allows_scheme_delimiters_in_path_and_query() {
        for mode in [UriMatchMode::RegistrableDomain, UriMatchMode::Host] {
            for (input, target) in [
                ("example.com/r?u=https://x.com", "https://example.com/"),
                ("localhost:8080/?next=http://x", "https://localhost:8080/"),
            ] {
                check(mode, input, target, true);
                check(mode, target, input, true);
            }
        }
        assert_eq!(host_of("foo://x"), Some("x".into()));
        assert_eq!(host_of("foo://["), None);
        assert_eq!(host_of("https://%zz"), None);
        for mode in UriMatchMode::ALL {
            check(mode, "https://foo/", "foo://x", false);
            check(mode, "foo://x", "https://foo/", false);
        }
    }
    #[test]
    fn trimmed_target_and_entry_url_length_boundaries() {
        let prefix = "https://example.com/";
        let boundary = format!("{prefix}{}", "a".repeat(8192 - prefix.len()));
        let oversized = format!("{boundary}a");
        assert_eq!(boundary.len(), 8192);
        assert_eq!(oversized.len(), 8193);
        for mode in UriMatchMode::ALL {
            let expected = mode != UriMatchMode::Never;
            check(mode, &boundary, &boundary, expected);
            check(mode, &boundary, &format!(" \t{boundary}\n "), expected);
            check(mode, &format!(" \t{boundary}\n "), &boundary, expected);
            check(mode, &boundary, &oversized, false);
            check(mode, &oversized, &boundary, false);
        }
    }
    #[test]
    fn registrable_and_host_modes_ignore_scheme_difference() {
        // Deliberate Swift parity; the scheme-downgrade decision is flagged for the owner.
        for mode in [UriMatchMode::RegistrableDomain, UriMatchMode::Host] {
            check(mode, "https://example.com/", "http://example.com/", true);
        }
        for mode in [UriMatchMode::Exact, UriMatchMode::StartsWith] {
            check(mode, "https://example.com/", "http://example.com/", false);
            check(mode, "http://example.com/", "https://example.com/", false);
        }
    }
    #[test]
    fn generic_helpers_and_entry_urls_preserve_lenient_non_web_parsing() {
        // Swift parity: retrying mailto as an HTTPS authority yields example.com.
        assert_eq!(host_of("mailto:a@example.com"), Some("example.com".into()));
        assert_eq!(rd("mailto:a@example.com"), Some("example.com".into()));
        for mode in UriMatchMode::ALL {
            check(
                mode,
                "mailto:a@example.com",
                "https://example.com/",
                mode != UriMatchMode::Never,
            );
        }
        for input in [
            "foo://example.com/",
            "\u{1}file://example.com/x",
            "java\tscript://example.com/",
        ] {
            assert_eq!(host_of(input), Some("example.com".into()));
            assert_eq!(rd(input), Some("example.com".into()));
            check(UriMatchMode::Host, input, "https://example.com/", true);
        }
    }
    #[test]
    fn strips_one_subdomain_label() {
        assert_eq!(
            rd("https://login.example.com/path"),
            Some("example.com".into())
        );
    }
    #[test]
    fn bare_apex_unchanged() {
        assert_eq!(rd("https://example.com"), Some("example.com".into()));
    }
    #[test]
    fn url_without_scheme() {
        assert_eq!(rd("example.com/login"), Some("example.com".into()));
    }
    #[test]
    fn co_uk_kept_whole() {
        assert_eq!(
            rd("https://accounts.google.co.uk/signin"),
            Some("google.co.uk".into())
        );
    }
    #[test]
    fn empty_is_none() {
        assert_eq!(rd(""), None);
    }
    #[test]
    fn unconvertible_non_ascii_host_is_none() {
        assert_eq!(rd("https://%E5%85%AC%2F%E5%8F%B8.hk/"), None);
    }
    #[test]
    fn percent_encoded_unicode_host_equals_ace() {
        assert_eq!(
            rd("https://login.example.%E5%85%AC%E5%8F%B8.hk/path"),
            Some("example.xn--55qx5d.hk".into())
        );
    }
    #[test]
    fn case_insensitive() {
        assert_eq!(rd("https://Example.COM"), Some("example.com".into()));
        check(
            UriMatchMode::RegistrableDomain,
            "https://Example.com",
            "https://EXAMPLE.COM",
            true,
        );
    }
    #[test]
    fn bare_public_suffix_compares_whole() {
        for s in ["github.io", "co.uk"] {
            assert_eq!(rd(&format!("https://{s}")), Some(s.into()));
        }
    }
    #[test]
    fn localhost_matches_localhost_and_differs_from_loopback() {
        assert_eq!(rd("http://localhost:8080/"), Some("localhost".into()));
        assert_ne!(rd("http://localhost/"), rd("http://127.0.0.1/"));
    }
    #[test]
    fn unicode_and_ace_equivalent() {
        for s in [
            "https://login.example.xn--55qx5d.hk/path",
            "https://login.example.\u{516c}\u{53f8}.hk/path",
            "example.\u{516c}\u{53f8}.hk",
            "https://EXAMPLE.\u{516c}\u{53f8}.HK./",
        ] {
            assert_eq!(rd(s), Some("example.xn--55qx5d.hk".into()));
        }
    }
    #[test]
    fn bare_unicode_public_suffix_compares_whole_in_one_spelling() {
        for s in [
            "https://xn--55qx5d.hk",
            "https://\u{516c}\u{53f8}.hk",
            "https://%E5%85%AC%E5%8F%B8.hk",
            "https://%E5%85%AC%E5%8F%B8.HK./",
        ] {
            assert_eq!(rd(s), Some("xn--55qx5d.hk".into()));
        }
        for s in ["https://%E4%BE%8B%E3%81%88/", "https://xn--r8jz45g/"] {
            assert_eq!(rd(s), Some("xn--r8jz45g".into()));
        }
    }
    #[test]
    fn no_overmatch_across_sites_under_unicode_suffix() {
        for t in [
            "https://beta.xn--55qx5d.hk",
            "https://beta.\u{516c}\u{53f8}.hk",
        ] {
            check(
                UriMatchMode::RegistrableDomain,
                "https://alpha.xn--55qx5d.hk",
                t,
                false,
            );
        }
    }
    #[test]
    fn same_site_across_unicode_and_ace_spellings() {
        for e in [
            "https://alpha.\u{516c}\u{53f8}.hk",
            "https://alpha.xn--55qx5d.hk",
        ] {
            for t in [
                "https://login.alpha.xn--55qx5d.hk",
                "https://login.alpha.\u{516c}\u{53f8}.hk",
            ] {
                check(UriMatchMode::RegistrableDomain, e, t, true);
            }
        }
    }
    #[test]
    fn distinct_ipv4_do_not_collapse() {
        let ips = ["127.0.0.1", "192.168.0.1", "10.0.0.1"];
        for (i, a) in ips.iter().enumerate() {
            assert_eq!(rd(a), Some((*a).into()));
            for b in ips.iter().skip(i + 1) {
                assert_ne!(rd(a), rd(b));
            }
        }
    }
    #[test]
    fn ipv4_with_port_and_path_equals_bare() {
        assert_eq!(rd("127.0.0.1"), rd("http://127.0.0.1:8766/callback?x=1"));
    }
    #[test]
    fn ipv6_bracketed_equals_unbracketed() {
        for s in ["::1", "[::1]", "http://[::1]/", "http://[::1]:8080/path"] {
            assert_eq!(rd(s), Some("::1".into()));
            assert_eq!(host_of(s), Some("::1".into()));
        }
    }
    #[test]
    fn ipv6_distinct_addresses_differ() {
        assert_ne!(rd("http://[::1]/"), rd("http://[::2]/"));
    }
    #[test]
    fn ipv4_mapped_ipv6_addresses_distinct() {
        let hosts = [
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:192.168.0.1]/",
            "http://[::ffff:10.0.0.1]/",
        ];
        for (i, a) in hosts.iter().enumerate() {
            assert!(rd(a).is_some());
            for b in hosts.iter().skip(i + 1) {
                assert_ne!(rd(a), rd(b));
            }
        }
    }
    #[test]
    fn ipv4_trailing_dot_equals_without() {
        assert_eq!(rd("http://127.0.0.1./"), rd("http://127.0.0.1/"));
        assert_eq!(rd("https://example.com./path"), Some("example.com".into()));
    }
    #[test]
    fn ipv4_nonstandard_spellings_normalise() {
        assert_eq!(rd("http://0x7f.1/"), Some("127.0.0.1".into()));
    }
    #[test]
    fn private_ip_entry_not_offered_on_loopback() {
        check(
            UriMatchMode::RegistrableDomain,
            "http://192.168.0.1/admin",
            "http://127.0.0.1:8766/login",
            false,
        );
    }
    #[test]
    fn private_ip_entry_offered_on_its_own_host() {
        check(
            UriMatchMode::RegistrableDomain,
            "http://192.168.0.1/admin",
            "http://192.168.0.1/login",
            true,
        );
    }
    #[test]
    fn includes_subdomain_match() {
        check(
            UriMatchMode::RegistrableDomain,
            "https://example.com/login",
            "https://login.example.com/signin",
            true,
        );
    }
    #[test]
    fn excludes_non_matching_domain() {
        check(
            UriMatchMode::RegistrableDomain,
            "https://example.com",
            "https://other.com",
            false,
        );
    }
    #[test]
    fn excludes_entries_with_empty_url() {
        check(
            UriMatchMode::RegistrableDomain,
            "",
            "https://example.com",
            false,
        );
    }
    #[test]
    fn no_overmatch_across_co_uk_sites() {
        check(
            UriMatchMode::RegistrableDomain,
            "https://www.amazon.co.uk",
            "https://accounts.google.co.uk",
            false,
        );
    }
    #[test]
    fn matches_same_co_uk_site() {
        check(
            UriMatchMode::RegistrableDomain,
            "https://google.co.uk",
            "https://mail.google.co.uk",
            true,
        );
    }
    #[test]
    fn no_overmatch_across_github_io_sites() {
        check(
            UriMatchMode::RegistrableDomain,
            "https://foo.github.io",
            "https://bar.github.io",
            false,
        );
    }
    #[test]
    fn host_mode_exact_host_and_port_case_insensitive() {
        check(
            UriMatchMode::Host,
            "https://Example.com:8443/login",
            "https://example.com:8443/anything",
            true,
        );
    }
    #[test]
    fn host_mode_does_not_match_subdomain() {
        check(
            UriMatchMode::Host,
            "https://example.com",
            "https://login.example.com",
            false,
        );
    }
    #[test]
    fn host_mode_different_port_no_match() {
        check(
            UriMatchMode::Host,
            "https://example.com:8443",
            "https://example.com:9443",
            false,
        );
    }
    #[test]
    fn host_mode_compares_ip_literals_whole() {
        check(
            UriMatchMode::Host,
            "http://192.168.0.1/admin",
            "http://127.0.0.1/",
            false,
        );
    }
    #[test]
    fn exact_mode_identical_normalised_url_only() {
        check(
            UriMatchMode::Exact,
            "HTTPS://Example.com:443/Login?x=1",
            "https://example.com/Login?x=1",
            true,
        );
    }
    #[test]
    fn exact_mode_path_is_case_sensitive() {
        check(
            UriMatchMode::Exact,
            "https://example.com/API/v2/Login",
            "https://example.com/api/v2/login",
            false,
        );
    }
    #[test]
    fn exact_mode_different_path_no_match() {
        check(
            UriMatchMode::Exact,
            "https://example.com/app",
            "https://example.com/app/extra",
            false,
        );
    }
    #[test]
    fn exact_mode_empty_path_equals_slash() {
        check(
            UriMatchMode::Exact,
            "https://example.com",
            "https://example.com/",
            true,
        );
    }
    #[test]
    fn starts_with_matches_any_page_under_prefix() {
        check(
            UriMatchMode::StartsWith,
            "https://example.com/app",
            "https://example.com/app/settings",
            true,
        );
    }
    #[test]
    fn starts_with_accepts_documented_prefix_overmatch() {
        check(
            UriMatchMode::StartsWith,
            "https://example.com/app",
            "https://example.com/app-legacy",
            true,
        );
    }
    #[test]
    fn starts_with_shorter_url_no_match() {
        check(
            UriMatchMode::StartsWith,
            "https://example.com/app/settings",
            "https://example.com/app",
            false,
        );
    }
    #[test]
    fn starts_with_lookalike_domain_never_matches() {
        check(
            UriMatchMode::StartsWith,
            "https://example.com",
            "https://example.com.attacker.com/anything",
            false,
        );
    }
    #[test]
    fn starts_with_different_port_no_match() {
        check(
            UriMatchMode::StartsWith,
            "https://example.com:8443/app",
            "https://example.com:9443/app",
            false,
        );
    }
    #[test]
    fn starts_with_multi_tenant_path_matches() {
        check(
            UriMatchMode::StartsWith,
            "https://example.com/tenant",
            "https://example.com/tenant/dashboard",
            true,
        );
    }
    #[test]
    fn never_mode_never_matches() {
        check(
            UriMatchMode::Never,
            "https://example.com",
            "https://example.com",
            false,
        );
    }
    #[test]
    fn matches_on_additional_url_when_primary_differs() {
        let mut e = make_entry("https://primary-only.example.org", "Multi-URL Entry");
        assert!(!matches(&e, "https://secondary.example.com/login"));
        e.set_urls(&[
            "https://primary-only.example.org".into(),
            "https://secondary.example.com".into(),
        ]);
        assert!(matches(&e, "https://secondary.example.com/login"));
        // Pin additional URL selection through host mode too.
        e.set_uri_match_mode(UriMatchMode::Host);
        assert!(matches(&e, "https://secondary.example.com/login"));
    }
    #[test]
    fn no_overmatch_with_extras() {
        let mut e = make_entry("https://primary-only.example.com", "Multi-URL Entry");
        e.set_urls(&[
            "https://primary-only.example.com".into(),
            "https://secondary.example.com".into(),
        ]);
        assert!(!matches(&e, "https://totally-unrelated.example.org"));
    }
    #[test]
    fn every_mode_treats_empty_target_as_no_match() {
        for mode in UriMatchMode::ALL {
            for t in ["", "  "] {
                check(mode, "https://example.com", t, false);
            }
        }
    }
    #[test]
    fn every_mode_treats_entry_without_url_as_no_match() {
        for mode in UriMatchMode::ALL {
            check(mode, "", "https://example.com", false);
        }
    }
    #[test]
    fn userinfo_uses_real_host() {
        for mode in UriMatchMode::ALL {
            check(
                mode,
                "https://example.com/",
                "https://example.com@evil.com/",
                false,
            );
            check(
                mode,
                "https://evil.com/",
                "https://example.com@evil.com/",
                mode != UriMatchMode::Never,
            );
        }
    }
    #[test]
    fn hostless_targets_never_match() {
        for mode in UriMatchMode::ALL {
            for t in [
                "javascript:alert(1)",
                "data:text/plain,hello",
                "file:///example.com/",
                "javascript://example.com/foo",
                "file://example.com/foo",
            ] {
                check(mode, "https://example.com/", t, false);
                check(mode, t, t, false);
            }
        }
    }
    #[test]
    fn oversized_urls_rejected() {
        let huge = format!("https://example.com/{}", "a".repeat(8193));
        assert_eq!(rd(&huge), None);
        for mode in UriMatchMode::ALL {
            check(mode, "https://example.com/", &huge, false);
            check(mode, &huge, "https://example.com/", false);
        }
    }
    #[test]
    fn walks_subgroups() {
        let mut db = Database::new("test");
        let mut g = Group::new("Sub", Timestamp(0));
        let e = make_entry("https://example.com", "Entry");
        let id = e.uuid;
        g.entries.push(e);
        db.root.groups.push(g);
        assert_eq!(
            credential_candidates(&db, "https://example.com")
                .iter()
                .map(|e| e.uuid)
                .collect::<Vec<_>>(),
            [id]
        );
    }
    #[test]
    fn recycle_bin_entries_excluded() {
        let mut db = Database::new("test");
        let e = make_entry("https://example.com", "Entry");
        let id = e.uuid;
        db.add_entry(db.root_id(), e, Timestamp(0)).unwrap();
        db.delete_entry(id, Timestamp(1)).unwrap();
        assert!(db.find_entry(id).is_some());
        assert!(credential_candidates(&db, "https://example.com").is_empty());
    }
    #[test]
    fn generator_history_group_excluded() {
        let mut db = Database::new("test");
        let mut g = Group::new("History", Timestamp(0));
        g.uuid = GENERATOR_HISTORY_GROUP_UUID;
        let mut child = Group::new("child", Timestamp(0));
        child
            .entries
            .push(make_entry("https://example.com", "hidden"));
        g.groups.push(child);
        db.root.groups.push(g);
        assert!(credential_candidates(&db, "https://example.com").is_empty());
    }
    #[test]
    fn address_and_files_entries_excluded_but_card_and_notes_follow_rule() {
        let mut db = Database::new("test");
        for t in ["Address", "Files", "Card", "Notes"] {
            let mut e = make_entry("https://example.com", t);
            e.set_field(ENTRY_TYPE_FIELD, t, false);
            db.root.entries.push(e);
        }
        assert_eq!(
            credential_candidates(&db, "https://example.com")
                .iter()
                .map(|e| e.title())
                .collect::<Vec<_>>(),
            ["Card", "Notes"]
        );
        assert!(credential_candidates(&db, "https://other.com").is_empty());
    }
    #[test]
    fn login_entry_included() {
        let mut db = Database::new("test");
        db.root
            .entries
            .push(make_entry("https://example.com", "Entry"));
        assert_eq!(credential_candidates(&db, "https://example.com").len(), 1);
    }
    #[test]
    fn history_snapshots_excluded() {
        let mut db = Database::new("test");
        let mut e = make_entry("https://other.com", "Entry");
        e.history.push(make_entry("https://example.com", "old"));
        db.root.entries.push(e);
        assert!(credential_candidates(&db, "https://example.com").is_empty());
    }
    #[test]
    fn host_of_preserves_full_host() {
        assert_eq!(
            host_of("account.samsung.com/login"),
            Some("account.samsung.com".into())
        );
        assert_eq!(host_of("localhost:8080/x"), Some("localhost".into()));
    }
}
