//! The bundled UI catalogs for the native side (tray menu, quit dialog).
//!
//! The three flat JSON catalogs under `src/lib/i18n/locales/` are embedded at build time, so the
//! tray can show its labels in the current language without a webview and a language change needs
//! no restart. A key maps to its ICU message; the native strings looked up here are plain text
//! without placeholders. Lookup never fails: a key missing from the chosen language falls back to
//! English, and a key missing everywhere (or a malformed catalog) yields the key itself.
//!
//! Two language enums meet here: [`SettingsLanguage`] is the persisted setting (it includes
//! `System`), [`Language`] is a concrete catalog.

use mypass_session::settings::Language as SettingsLanguage;
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

/// One language's messages: a flat `{ "key": "message" }` map.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Catalog(HashMap<String, String>);

impl Catalog {
    /// Parse a flat catalog. Anything else (malformed JSON, a non-object, a non-string value)
    /// yields what could be read, never a panic.
    pub fn from_json(source: &str) -> Self {
        let Ok(serde_json::Value::Object(object)) = serde_json::from_str(source) else {
            return Self::default();
        };
        Self(
            object
                .into_iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::String(text) => Some((key, text)),
                    _ => None,
                })
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The raw message of `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// The message of `key` with `{0}`, `{1}`... replaced by `args`, `None` for an unknown key.
    pub fn format(&self, key: &str, args: &[&str]) -> Option<String> {
        self.get(key).map(|message| substitute(message, args))
    }
}

/// Replace `{n}` with `args[n]` in one pass (an argument that itself contains `{1}` is not
/// expanded again). A placeholder without an argument stays as written.
fn substitute(message: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some((head, tail)) = rest.split_once('{') {
        out.push_str(head);
        let argument = tail.split_once('}').and_then(|(name, after)| {
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let index: usize = name.parse().ok()?;
            args.get(index).map(|arg| (*arg, after))
        });
        match argument {
            Some((arg, after)) => {
                out.push_str(arg);
                rest = after;
            }
            None => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The message of `key` from `primary`, else from `fallback`, else the key itself, with `args`
/// substituted. The one lookup rule of [`t_resolved`] (tests give it stub catalogs).
pub fn translate_with(primary: &Catalog, fallback: &Catalog, key: &str, args: &[&str]) -> String {
    primary
        .format(key, args)
        .or_else(|| fallback.format(key, args))
        .unwrap_or_else(|| substitute(key, args))
}

/// The parsed catalog of a language, parsed once per process.
fn catalog(lang: Language) -> &'static Catalog {
    static EN_CATALOG: OnceLock<Catalog> = OnceLock::new();
    static UK_CATALOG: OnceLock<Catalog> = OnceLock::new();
    static RU_CATALOG: OnceLock<Catalog> = OnceLock::new();
    match lang {
        Language::En => EN_CATALOG.get_or_init(|| Catalog::from_json(EN)),
        Language::Uk => UK_CATALOG.get_or_init(|| Catalog::from_json(UK)),
        Language::Ru => RU_CATALOG.get_or_init(|| Catalog::from_json(RU)),
    }
}

/// The message of `key` in exactly this language, no fallback.
pub fn lookup(lang: Language, key: &str) -> Option<&'static str> {
    catalog(lang).get(key)
}

/// The message of `key` in `lang`, falling back to English and then to the key itself.
pub fn translate(lang: Language, key: &str) -> &str {
    lookup(lang, key)
        .or_else(|| lookup(Language::En, key))
        .unwrap_or(key)
}

/// The language a locale string ("uk-UA", "ru_RU", "de-DE") selects: `uk*` and `ru*`, else
/// English. Pure, so tests inject the locale.
pub fn language_for_locale(locale: &str) -> Language {
    let primary = locale
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match primary.as_str() {
        "uk" => Language::Uk,
        "ru" => Language::Ru,
        _ => Language::En,
    }
}

/// The concrete language of a setting: `System` resolves through `locale` (the system locale,
/// `None` when unknown, which means English).
pub fn resolve(setting: SettingsLanguage, locale: Option<&str>) -> Language {
    match setting {
        SettingsLanguage::En => Language::En,
        SettingsLanguage::Uk => Language::Uk,
        SettingsLanguage::Ru => Language::Ru,
        SettingsLanguage::System => locale.map_or(Language::En, language_for_locale),
    }
}

/// A message for the native side in the user's language, `{0}`... substituted from `args`.
/// `Language::System` resolves through `sys-locale`. Falls back to English, then to the key.
pub fn t(lang: SettingsLanguage, key: &str, args: &[&str]) -> String {
    let locale = match lang {
        SettingsLanguage::System => sys_locale::get_locale(),
        _ => None,
    };
    let resolved = resolve(lang, locale.as_deref());
    t_resolved(resolved, key, args)
}

/// [`t`] for an already resolved language.
pub fn t_resolved(lang: Language, key: &str, args: &[&str]) -> String {
    translate_with(catalog(lang), catalog(Language::En), key, args)
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
        assert!(Catalog::from_json("not json").is_empty());
        assert!(Catalog::from_json("[1, 2]").is_empty());
    }

    #[test]
    fn language_codes_round_trip() {
        for lang in [Language::En, Language::Uk, Language::Ru] {
            assert_eq!(Language::from_code(lang.code()), Some(lang));
        }
        assert_eq!(Language::from_code("de"), None);
    }

    #[test]
    fn i18n_fallback_to_english_for_unknown_key() {
        // A key no catalog holds comes back as itself, in every language.
        for lang in [Language::En, Language::Uk, Language::Ru] {
            assert_eq!(t_resolved(lang, "No such key {0}", &["x"]), "No such key x");
        }
        // A key only English holds (stub catalogs) falls back to English; without the
        // fallback the key itself would come back.
        let uk = Catalog::from_json(r#"{"A": "uk-a {0}"}"#);
        let en = Catalog::from_json(r#"{"A": "en-a", "B": "en-b {0}"}"#);
        assert_eq!(translate_with(&uk, &en, "A", &["x"]), "uk-a x");
        assert_eq!(translate_with(&uk, &en, "B", &["x"]), "en-b x");
        assert_eq!(translate_with(&uk, &en, "C {0}", &["x"]), "C x");
        assert_eq!(
            translate_with(&uk, &Catalog::default(), "B", &[]),
            "B",
            "no fallback catalog: the key"
        );
    }

    #[test]
    fn i18n_substitutes_positional_args() {
        let catalog = Catalog::from_json(r#"{"Hello {0}, {1}": "Hi {0}, {1}"}"#);
        assert_eq!(
            catalog.format("Hello {0}, {1}", &["A", "B"]).as_deref(),
            Some("Hi A, B")
        );
        // Out of order, repeated, missing and non-numeric placeholders, and no re-expansion.
        assert_eq!(substitute("{1}{0}{1}", &["a", "b"]), "bab");
        assert_eq!(substitute("{2} {x} {", &["a"]), "{2} {x} {");
        assert_eq!(substitute("{0}", &["{1}", "z"]), "{1}");
    }

    #[test]
    fn system_language_resolves_from_locale_table() {
        for (locale, expected) in [
            ("uk-UA", Language::Uk),
            ("uk", Language::Uk),
            ("ru-RU", Language::Ru),
            ("ru_RU", Language::Ru),
            ("UK-ua", Language::Uk),
            ("de-DE", Language::En),
            ("en-US", Language::En),
            ("", Language::En),
        ] {
            assert_eq!(
                resolve(SettingsLanguage::System, Some(locale)),
                expected,
                "{locale}"
            );
        }
        assert_eq!(resolve(SettingsLanguage::System, None), Language::En);
        // An explicit setting ignores the system locale.
        assert_eq!(resolve(SettingsLanguage::Ru, Some("uk-UA")), Language::Ru);
    }

    #[test]
    fn t_with_an_explicit_language_reads_the_shared_catalog() {
        let uk_lock = lookup(Language::Uk, "Lock").unwrap();
        assert_eq!(t(SettingsLanguage::Uk, "Lock", &[]), uk_lock);
        assert_eq!(
            t(
                SettingsLanguage::En,
                "MyPass %@ available\u{2026}",
                &["1.0"]
            ),
            "MyPass 1.0 available\u{2026}"
        );
    }
}
