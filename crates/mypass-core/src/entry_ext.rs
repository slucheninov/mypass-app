//! Vault-compatible URL lists and entry matching preferences.
use mypass_kdbx::{Entry, StringArray};
use std::collections::HashSet;
use uuid::{uuid, Uuid};
pub const URI_MATCH_MODE_FIELD: &str = "MyPass.URIMatchMode";
pub const ENTRY_TYPE_FIELD: &str = "MyPass.EntryType";
pub const ADDITIONAL_URLS_KEY: &str = "KP2A_URL";
pub const GENERATOR_HISTORY_GROUP_UUID: Uuid = uuid!("E4B1C0A2-7D58-4F13-9A6E-2C8F5B0D11A7");
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UriMatchMode {
    RegistrableDomain,
    Host,
    StartsWith,
    Exact,
    Never,
}
impl UriMatchMode {
    pub const ALL: [Self; 5] = [
        Self::RegistrableDomain,
        Self::Host,
        Self::StartsWith,
        Self::Exact,
        Self::Never,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RegistrableDomain => "registrable-domain",
            Self::Host => "host",
            Self::StartsWith => "starts-with",
            Self::Exact => "exact",
            Self::Never => "never",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    Login,
    Address,
    Card,
    Notes,
    Files,
}
/// Accessors compatible with the Swift vault conventions.
pub trait EntryExt {
    fn urls(&self) -> Vec<String>;
    fn set_urls(&mut self, urls: &[String]);
    fn uri_match_mode(&self) -> UriMatchMode;
    fn set_uri_match_mode(&mut self, mode: UriMatchMode);
    fn entry_type(&self) -> EntryType;
}
impl EntryExt for Entry {
    fn urls(&self) -> Vec<String> {
        // Only the first StringArray is consumed, exactly like the Swift accessor.
        // A hash set preserves linear expected work even for many duplicate extras.
        let extras = self
            .string_arrays
            .iter()
            .find(|a| a.key == ADDITIONAL_URLS_KEY)
            .into_iter()
            .flat_map(|a| a.values.iter().map(String::as_str));
        let mut seen = HashSet::new();
        self.field("URL")
            .into_iter()
            .chain(extras)
            .filter(|s| !s.is_empty() && seen.insert(*s))
            .map(str::to_owned)
            .collect()
    }
    fn set_urls(&mut self, urls: &[String]) {
        let mut values = urls.iter();
        self.set_field(
            "URL",
            values.next().map(String::as_str).unwrap_or(""),
            false,
        );
        self.string_arrays.retain(|a| a.key != ADDITIONAL_URLS_KEY);
        let remaining: Vec<_> = values.cloned().collect();
        if !remaining.is_empty() {
            self.string_arrays.push(StringArray {
                key: ADDITIONAL_URLS_KEY.into(),
                values: remaining,
            });
        }
    }
    fn uri_match_mode(&self) -> UriMatchMode {
        self.field(URI_MATCH_MODE_FIELD)
            .and_then(UriMatchMode::parse)
            .unwrap_or(UriMatchMode::RegistrableDomain)
    }
    fn set_uri_match_mode(&mut self, mode: UriMatchMode) {
        if mode == UriMatchMode::RegistrableDomain {
            self.remove_field(URI_MATCH_MODE_FIELD);
        } else {
            self.set_field(URI_MATCH_MODE_FIELD, mode.as_str(), false);
        }
    }
    fn entry_type(&self) -> EntryType {
        match self.field(ENTRY_TYPE_FIELD) {
            Some("Address") => EntryType::Address,
            Some("Card") => EntryType::Card,
            Some("Notes") => EntryType::Notes,
            Some("Files") => EntryType::Files,
            _ => EntryType::Login,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mypass_kdbx::{Entry, StringArray, Timestamp};
    fn entry() -> Entry {
        Entry::new(Timestamp(0))
    }
    fn extras(e: &mut Entry, values: &[&str]) {
        e.string_arrays.push(StringArray {
            key: ADDITIONAL_URLS_KEY.into(),
            values: values.iter().map(|s| (*s).into()).collect(),
        });
    }
    #[test]
    fn urls_primary_then_extras_deduplicated() {
        let mut e = entry();
        e.set_field("URL", "b", true);
        extras(&mut e, &["a", "b", "a", "c"]);
        assert_eq!(e.urls(), ["b", "a", "c"]);
        e.set_field("KP2A_URL_1", "ignored", false);
        e.set_field("KP2A_URL", "also ignored", true);
        extras(&mut e, &["second array ignored"]);
        assert_eq!(e.urls(), ["b", "a", "c"]);
    }
    #[test]
    fn urls_skip_empty_extras() {
        let mut e = entry();
        extras(&mut e, &["", "a", "", "b"]);
        assert_eq!(e.urls(), ["a", "b"]);
    }
    #[test]
    fn set_urls_round_trip_and_clear() {
        let mut e = entry();
        extras(&mut e, &["old"]);
        extras(&mut e, &["old2"]);
        e.string_arrays.push(StringArray {
            key: "other".into(),
            values: vec!["keep".into()],
        });
        e.set_urls(&["primary".into(), "extra".into(), "extra".into(), "".into()]);
        assert_eq!(e.field("URL"), Some("primary"));
        assert_eq!(e.urls(), ["primary", "extra"]);
        assert_eq!(
            e.string_arrays
                .iter()
                .find(|a| a.key == ADDITIONAL_URLS_KEY)
                .unwrap()
                .values,
            ["extra", "extra", ""]
        );
        e.set_urls(&[]);
        assert_eq!(e.field("URL"), Some(""));
        assert!(e.urls().is_empty());
        assert_eq!(e.string_arrays.len(), 1);
        e.set_urls(&["only".into()]);
        assert_eq!(e.urls(), ["only"]);
        assert_eq!(e.string_arrays.len(), 1);
    }
    #[test]
    fn uri_match_mode_defaults_and_unknown_value() {
        let mut e = entry();
        assert_eq!(e.uri_match_mode(), UriMatchMode::RegistrableDomain);
        e.set_field(URI_MATCH_MODE_FIELD, "future", false);
        assert_eq!(e.uri_match_mode(), UriMatchMode::RegistrableDomain);
        assert_eq!(UriMatchMode::parse("future"), None);
    }
    #[test]
    fn set_default_mode_removes_field() {
        let mut e = entry();
        e.set_uri_match_mode(UriMatchMode::Host);
        e.set_uri_match_mode(UriMatchMode::RegistrableDomain);
        assert_eq!(e.field(URI_MATCH_MODE_FIELD), None);
    }
    #[test]
    fn uri_match_mode_raw_values_are_kebab_case() {
        assert_eq!(
            UriMatchMode::ALL.map(UriMatchMode::as_str),
            [
                "registrable-domain",
                "host",
                "starts-with",
                "exact",
                "never"
            ]
        );
    }
    #[test]
    fn uri_match_mode_values_serialise_to_old_raw_values() {
        for (mode, raw) in UriMatchMode::ALL.into_iter().zip([
            "registrable-domain",
            "host",
            "starts-with",
            "exact",
            "never",
        ]) {
            assert_eq!(mode.as_str(), raw);
            assert_eq!(UriMatchMode::parse(raw), Some(mode));
            let mut e = entry();
            e.set_field(URI_MATCH_MODE_FIELD, raw, true);
            assert_eq!(e.uri_match_mode(), mode);
            e.set_uri_match_mode(mode);
            assert_eq!(
                e.field(URI_MATCH_MODE_FIELD),
                if mode == UriMatchMode::RegistrableDomain {
                    None
                } else {
                    Some(raw)
                }
            );
            assert!(e
                .fields
                .iter()
                .all(|f| f.key != URI_MATCH_MODE_FIELD || !f.protected));
        }
    }
    #[test]
    fn all_covers_every_mode_once() {
        assert_eq!(UriMatchMode::ALL.len(), 5);
        assert_eq!(
            UriMatchMode::ALL
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            5
        );
    }
    #[test]
    fn entry_type_unknown_reads_as_login() {
        let mut e = entry();
        assert_eq!(e.entry_type(), EntryType::Login);
        e.set_field(ENTRY_TYPE_FIELD, "future", false);
        assert_eq!(e.entry_type(), EntryType::Login);
        for (s, t) in [
            ("Login", EntryType::Login),
            ("Address", EntryType::Address),
            ("Card", EntryType::Card),
            ("Notes", EntryType::Notes),
            ("Files", EntryType::Files),
        ] {
            e.set_field(ENTRY_TYPE_FIELD, s, false);
            assert_eq!(e.entry_type(), t);
        }
    }
}
