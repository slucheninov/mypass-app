//! The bundled UI catalogs for the native side (tray menu, quit dialog).
//!
//! The three flat JSON catalogs under `src/lib/i18n/locales/` are embedded at build time, so the
//! tray can show its labels in the current language without a webview and a language change needs
//! no restart. A key maps to its ICU message; the native strings looked up here are plain text
//! without placeholders. Lookup never fails: a key missing from the chosen language falls back to
//! English, and a key missing everywhere (or a malformed catalog) yields the key itself.

use std::collections::HashMap;
use std::sync::OnceLock;

/// A UI language the app ships a catalog for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    En,
    Uk,
    Ru,
}

impl Language {
    /// The BCP 47 code used in settings and file names.
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Uk => "uk",
            Language::Ru => "ru",
        }
    }

    /// The language for a code, `None` for an unsupported one.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Language::En),
            "uk" => Some(Language::Uk),
            "ru" => Some(Language::Ru),
            _ => None,
        }
    }
}

type Catalog = HashMap<String, String>;

const EN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/lib/i18n/locales/en.json"
));
const UK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/lib/i18n/locales/uk.json"
));
const RU: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/lib/i18n/locales/ru.json"
));

/// Parse a flat `{ "key": "message" }` catalog. Anything else (malformed JSON, a non-object, a
/// non-string value) yields what could be read, never a panic.
fn parse_catalog(source: &str) -> Catalog {
    let Ok(serde_json::Value::Object(object)) = serde_json::from_str(source) else {
        return Catalog::new();
    };
    object
        .into_iter()
        .filter_map(|(key, value)| match value {
            serde_json::Value::String(text) => Some((key, text)),
            _ => None,
        })
        .collect()
}

/// The parsed catalog of a language, parsed once per process.
fn catalog(lang: Language) -> &'static Catalog {
    static EN_CATALOG: OnceLock<Catalog> = OnceLock::new();
    static UK_CATALOG: OnceLock<Catalog> = OnceLock::new();
    static RU_CATALOG: OnceLock<Catalog> = OnceLock::new();
    match lang {
        Language::En => EN_CATALOG.get_or_init(|| parse_catalog(EN)),
        Language::Uk => UK_CATALOG.get_or_init(|| parse_catalog(UK)),
        Language::Ru => RU_CATALOG.get_or_init(|| parse_catalog(RU)),
    }
}

/// The message of `key` in exactly this language, no fallback.
pub fn lookup(lang: Language, key: &str) -> Option<&'static str> {
    catalog(lang).get(key).map(String::as_str)
}

/// The message of `key` in `lang`, falling back to English and then to the key itself.
pub fn translate(lang: Language, key: &str) -> &str {
    lookup(lang, key)
        .or_else(|| lookup(Language::En, key))
        .unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAY_KEYS: [&str; 7] = [
        "Unlock",
        "Lock",
        "Open MyPass",
        "Settings\u{2026}",
        "Quit",
        "Quit Anyway",
        "Cancel",
    ];

    #[test]
    fn bundled_catalogs_contain_all_tray_keys_in_all_languages() {
        for lang in [Language::En, Language::Uk, Language::Ru] {
            for key in TRAY_KEYS {
                let text = lookup(lang, key);
                assert!(
                    text.is_some_and(|t| !t.is_empty()),
                    "{lang:?} lacks {key:?}"
                );
            }
        }
        assert_ne!(translate(Language::Uk, "Unlock"), "Unlock");
        assert_ne!(translate(Language::Ru, "Unlock"), "Unlock");
    }

    #[test]
    fn unknown_key_returns_the_key_and_malformed_catalog_is_empty() {
        assert_eq!(translate(Language::Uk, "No such key"), "No such key");
        assert!(parse_catalog("not json").is_empty());
        assert!(parse_catalog("[1, 2]").is_empty());
    }

    #[test]
    fn language_codes_round_trip() {
        for lang in [Language::En, Language::Uk, Language::Ru] {
            assert_eq!(Language::from_code(lang.code()), Some(lang));
        }
        assert_eq!(Language::from_code("de"), None);
    }
}
