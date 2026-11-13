//! Limits and character rules for every string the extension hands to a handler that stores it.
//!
//! The extension is a separate, less trusted process: nothing it sends reaches the vault or the
//! UI unbounded or with characters that could spoof or break the text (control characters, and the
//! invisible format characters such as bidi overrides and zero-width joiners).

use crate::protocol::is_unsafe_text_char;

pub(crate) const MAX_TITLE_CHARS: usize = 256;
pub(crate) const MAX_USERNAME_CHARS: usize = 512;
pub(crate) const MAX_PASSWORD_CHARS: usize = 4096;
pub(crate) const MAX_URL_CHARS: usize = 2048;
pub(crate) const MAX_CARD_NUMBER_CHARS: usize = 64;
pub(crate) const MAX_CARDHOLDER_CHARS: usize = 256;
pub(crate) const MAX_EXPIRY_CHARS: usize = 32;
pub(crate) const MAX_CVV_CHARS: usize = 16;
pub(crate) const MAX_NOTES_CHARS: usize = 64 * 1024;
pub(crate) const MAX_OTP_CHARS: usize = 2048;
pub(crate) const MAX_MATCH_CHARS: usize = 32;
pub(crate) const MAX_IDENTIFIER_CHARS: usize = 200;

/// A single-line value that the UI shows: at most `max` characters, no control or format
/// character.
pub(crate) fn is_clean_line(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(is_unsafe_text_char)
}

/// A secret or free text that may contain format characters (emoji joiners, right-to-left text)
/// but never a control character: at most `max` characters. With `allow_breaks`, a tab, a line
/// feed and a carriage return are allowed too (notes).
pub(crate) fn is_clean_text(s: &str, max: usize, allow_breaks: bool) -> bool {
    s.chars().count() <= max
        && !s
            .chars()
            .any(|c| c.is_control() && !(allow_breaks && matches!(c, '\t' | '\n' | '\r')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_reject_control_and_format_characters_and_overlong_text() {
        assert!(is_clean_line("Bank \u{1F3E6}", 10));
        assert!(!is_clean_line("a\u{0007}b", 10));
        assert!(!is_clean_line("a\nb", 10));
        assert!(!is_clean_line("a\u{202E}b", 10));
        assert!(!is_clean_line("a\u{200D}b", 10));
        assert!(is_clean_line(&"x".repeat(10), 10));
        assert!(!is_clean_line(&"x".repeat(11), 10));
    }

    #[test]
    fn text_allows_format_characters_and_breaks_only_when_asked() {
        assert!(is_clean_text("a\u{200D}b", 10, false));
        assert!(!is_clean_text("a\nb", 10, false));
        assert!(is_clean_text("a\r\n\tb", 10, true));
        assert!(!is_clean_text("a\u{0000}b", 10, true));
        assert!(!is_clean_text(&"x".repeat(11), 10, true));
    }
}
