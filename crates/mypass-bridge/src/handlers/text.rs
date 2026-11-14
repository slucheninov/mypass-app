//! Limits and character rules for every string the extension hands to a handler that stores it.
//!
//! The rules live in `mypass_core::text`, shared with the passkey request parser, so a value is
//! held to the same bounds whichever path writes it.

pub(crate) use mypass_core::text::{
    is_clean_line, is_clean_text, MAX_CARDHOLDER_CHARS, MAX_CARD_NUMBER_CHARS, MAX_CVV_CHARS,
    MAX_EXPIRY_CHARS, MAX_IDENTIFIER_CHARS, MAX_MATCH_CHARS, MAX_NOTES_CHARS, MAX_OTP_CHARS,
    MAX_PASSWORD_CHARS, MAX_TITLE_CHARS, MAX_URL_CHARS, MAX_USERNAME_CHARS,
};

#[cfg(test)]
mod tests {
    #[test]
    fn host_and_core_unsafe_text_rules_agree_on_every_char() {
        // The host cannot link mypass-core, so `protocol` keeps its own copy of the rule.
        for c in (0..=0x10FFFF_u32).filter_map(char::from_u32) {
            assert_eq!(
                crate::protocol::is_unsafe_text_char(c),
                mypass_core::text::is_unsafe_text_char(c),
                "U+{:04X}",
                u32::from(c)
            );
        }
    }
}
