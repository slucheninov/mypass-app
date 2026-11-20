//! Startup failure text is usable before the runtime or settings store exists.

use crate::i18n::{self, Language};

/// Never include a raw startup error: it may contain paths or backend details.
pub fn startup_error_text(language: Language) -> (String, String) {
    (
        i18n::t_resolved(language, "MyPass could not start", &[]),
        i18n::t_resolved(language, "startup.failure.message", &[]),
    )
}

/// Called from `main` even if building the Tauri runtime itself failed.
pub fn show_error() {
    let locale = sys_locale::get_locale().unwrap_or_default();
    let (title, message) = startup_error_text(i18n::language_for_locale(&locale));
    mypass_platform::show_startup_error(&title, &message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn startup_error_text_is_localized_without_raw_error_details() {
        let en = startup_error_text(Language::En);
        assert!(en.0.contains("MyPass"));
        assert!(!en.1.is_empty());
        for lang in [Language::Uk, Language::Ru, Language::De] {
            let text = startup_error_text(lang);
            assert_ne!(text.0, en.0);
            assert_ne!(text.1, en.1);
            assert!(!text.0.is_empty() && !text.1.is_empty());
        }
    }
}
