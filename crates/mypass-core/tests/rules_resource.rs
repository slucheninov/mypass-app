//! Tests over the real, verbatim vendored catalog.
use mypass_core::{parse_password_rules, PasswordRulesIndex};
const RAW: &str = include_str!("../resources/password-rules.json");

#[test]
fn bundled_json_has_436_entries_and_all_parse() {
    let index = PasswordRulesIndex::from_json(RAW).unwrap();
    assert_eq!(index.entries().count(), 436);
    for (host, raw) in index.entries() {
        assert!(parse_password_rules(raw).is_ok(), "{host}: {raw}");
    }
}

#[test]
fn json_with_crlf_line_endings_parses_identically() {
    let lf = PasswordRulesIndex::from_json(RAW).unwrap();
    let crlf = PasswordRulesIndex::from_json(&RAW.replace('\n', "\r\n")).unwrap();
    assert_eq!(lf, crlf);
    assert_eq!(crlf.entries().count(), 436);
    assert_eq!(crlf.rules_for_host("163.com").unwrap().min_length, Some(6));
}
