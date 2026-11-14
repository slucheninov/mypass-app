//! Limits and character rules for every string a less trusted process (the browser extension, a
//! web page through it) hands to code that stores or shows it.
//!
//! Nothing such a caller sends reaches the vault or the UI unbounded or with characters that could
//! spoof or break the text (control characters, and the invisible format characters such as bidi
//! overrides and zero-width joiners). The bridge handlers and the passkey request parser share
//! these rules, so a title or user name is held to the same bounds whichever path wrote it.

/// Longest entry title, in characters.
pub const MAX_TITLE_CHARS: usize = 256;
/// Longest user name, in characters.
pub const MAX_USERNAME_CHARS: usize = 512;
/// Longest password, in characters.
pub const MAX_PASSWORD_CHARS: usize = 4096;
/// Longest URL or origin, in characters.
pub const MAX_URL_CHARS: usize = 8192;
/// Longest card number, in characters.
pub const MAX_CARD_NUMBER_CHARS: usize = 64;
/// Longest cardholder name, in characters.
pub const MAX_CARDHOLDER_CHARS: usize = 256;
/// Longest card expiry, in characters.
pub const MAX_EXPIRY_CHARS: usize = 32;
/// Longest card security code, in characters.
pub const MAX_CVV_CHARS: usize = 16;
/// Longest notes text, in characters.
pub const MAX_NOTES_CHARS: usize = 64 * 1024;
/// Longest one-time-password secret or URI, in characters.
pub const MAX_OTP_CHARS: usize = 2048;
/// Longest URL match mode name, in characters.
pub const MAX_MATCH_CHARS: usize = 32;
/// Longest saved identifier, in characters.
pub const MAX_IDENTIFIER_CHARS: usize = 200;

/// Unicode format characters (category Cf): soft hyphen, Arabic number signs, zero-width and
/// bidirectional marks and overrides, word joiner and invisible operators, BOM, interlinear
/// annotation, musical and tag characters. They render as nothing or reorder surrounding text.
fn is_format_char(c: char) -> bool {
    matches!(u32::from(c),
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

/// Control (Cc) or format (Cf) characters, which no name or label shown in the UI may contain.
///
/// `mypass_bridge::protocol::is_unsafe_text_char` is the same rule for the host side, which must
/// not link this crate; a bridge test pins that the two agree on every `char`.
#[must_use]
pub fn is_unsafe_text_char(c: char) -> bool {
    c.is_control() || is_format_char(c)
}

/// A single-line value that the UI shows: at most `max` characters, no control or format
/// character.
#[must_use]
pub fn is_clean_line(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(is_unsafe_text_char)
}

/// A secret or free text that may contain format characters (emoji joiners, right-to-left text)
/// but never a control character: at most `max` characters. With `allow_breaks`, a tab, a line
/// feed and a carriage return are allowed too (notes).
#[must_use]
pub fn is_clean_text(s: &str, max: usize, allow_breaks: bool) -> bool {
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
