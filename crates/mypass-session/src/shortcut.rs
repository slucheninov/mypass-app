//! Global shortcut strings: parsing and the one canonical spelling stored in settings.
//!
//! The canonical form is the accelerator syntax of Tauri's global-shortcut plugin: modifiers in
//! the fixed order `CommandOrControl`, `Alt`, `Shift`, `Super`, then exactly one key, joined by
//! `+`. Parsing is case-insensitive and accepts the aliases the old app and users write
//! (`cmd`, `command`, `ctrl`, `control`, `CmdOrCtrl`, `option`, `opt`); the old app stored
//! `shift+command+L`. `ctrl` deliberately maps to `CommandOrControl` (Cmd on macOS, Ctrl on
//! Windows), so a shortcut means the same thing on both systems.

/// A shortcut modifier, in canonical order (the derive order is the sort order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortcutModifier {
    CommandOrControl,
    Alt,
    Shift,
    Super,
}

impl ShortcutModifier {
    /// The canonical spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CommandOrControl => "CommandOrControl",
            Self::Alt => "Alt",
            Self::Shift => "Shift",
            Self::Super => "Super",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "cmdorctrl" | "cmd" | "command" | "ctrl" | "control" => {
                Some(Self::CommandOrControl)
            }
            "alt" | "option" | "opt" => Some(Self::Alt),
            "shift" => Some(Self::Shift),
            "super" | "meta" | "win" => Some(Self::Super),
            _ => None,
        }
    }
}

/// A validated shortcut: modifiers sorted and de-duplicated, the key in canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedShortcut {
    pub modifiers: Vec<ShortcutModifier>,
    pub key: String,
}

impl ParsedShortcut {
    /// The canonical accelerator string.
    pub fn canonical(&self) -> String {
        let mut parts: Vec<&str> = self.modifiers.iter().map(|m| m.as_str()).collect();
        parts.push(&self.key);
        parts.join("+")
    }
}

/// Why a shortcut string was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShortcutError {
    #[error("the shortcut is empty")]
    Empty,
    #[error("the shortcut has an empty part")]
    EmptyPart,
    #[error("the shortcut needs at least one modifier")]
    NoModifier,
    #[error("the shortcut needs a key")]
    NoKey,
    #[error("the shortcut has more than one key")]
    MultipleKeys,
    #[error("unknown key: {0}")]
    UnknownKey(String),
}

/// Named keys: accepted spellings (lower case) and the canonical name.
const NAMED_KEYS: &[(&[&str], &str)] = &[
    (&["space"], "Space"),
    (&["enter", "return"], "Enter"),
    (&["tab"], "Tab"),
    (&["escape", "esc"], "Escape"),
    (&["backspace"], "Backspace"),
    (&["delete", "del"], "Delete"),
    (&["insert"], "Insert"),
    (&["home"], "Home"),
    (&["end"], "End"),
    (&["pageup"], "PageUp"),
    (&["pagedown"], "PageDown"),
    (&["up", "arrowup"], "ArrowUp"),
    (&["down", "arrowdown"], "ArrowDown"),
    (&["left", "arrowleft"], "ArrowLeft"),
    (&["right", "arrowright"], "ArrowRight"),
    (&["minus", "-"], "Minus"),
    (&["equal", "="], "Equal"),
    (&["comma", ","], "Comma"),
    (&["period", "."], "Period"),
    (&["slash", "/"], "Slash"),
    (&["backslash", "\\"], "Backslash"),
    (&["semicolon", ";"], "Semicolon"),
    (&["quote", "'"], "Quote"),
    (&["backquote", "`"], "Backquote"),
    (&["bracketleft", "["], "BracketLeft"),
    (&["bracketright", "]"], "BracketRight"),
];

fn parse_key(token: &str) -> Option<String> {
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
    }
    let lower = token.to_ascii_lowercase();
    if let Some(n) = lower.strip_prefix('f').and_then(|d| d.parse::<u8>().ok()) {
        if (1..=24).contains(&n) && !lower.starts_with("f0") {
            return Some(format!("F{n}"));
        }
    }
    NAMED_KEYS
        .iter()
        .find(|(aliases, _)| aliases.contains(&lower.as_str()))
        .map(|(_, name)| (*name).to_owned())
}

/// Parses `s`: at least one modifier and exactly one known key are required.
pub fn parse_shortcut(s: &str) -> Result<ParsedShortcut, ShortcutError> {
    if s.trim().is_empty() {
        return Err(ShortcutError::Empty);
    }
    let mut modifiers = Vec::new();
    let mut key = None;
    for raw in s.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            return Err(ShortcutError::EmptyPart);
        }
        if let Some(m) = ShortcutModifier::parse(token) {
            modifiers.push(m);
        } else {
            let k = parse_key(token).ok_or_else(|| ShortcutError::UnknownKey(token.to_owned()))?;
            if key.replace(k).is_some() {
                return Err(ShortcutError::MultipleKeys);
            }
        }
    }
    let key = key.ok_or(ShortcutError::NoKey)?;
    if modifiers.is_empty() {
        return Err(ShortcutError::NoModifier);
    }
    modifiers.sort();
    modifiers.dedup();
    Ok(ParsedShortcut { modifiers, key })
}

/// The canonical spelling of `s` (see the module documentation).
pub fn canonical(s: &str) -> Result<String, ShortcutError> {
    parse_shortcut(s).map(|p| p.canonical())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_old_app_string_is_canonicalised() {
        assert_eq!(
            canonical("shift+command+L").unwrap(),
            "CommandOrControl+Shift+L"
        );
    }

    #[test]
    fn cmd_or_ctrl_alias_canonicalised() {
        assert_eq!(
            canonical("CmdOrCtrl+Shift+L").unwrap(),
            "CommandOrControl+Shift+L"
        );
    }

    #[test]
    fn ctrl_shift_l_canonical_equals_command_variant() {
        assert_eq!(
            canonical("ctrl+shift+l").unwrap(),
            canonical("Command+Shift+L").unwrap()
        );
        assert_eq!(
            canonical("control+SHIFT+l").unwrap(),
            "CommandOrControl+Shift+L"
        );
    }

    #[test]
    fn modifier_order_is_fixed() {
        assert_eq!(
            canonical("super+shift+opt+cmd+k").unwrap(),
            "CommandOrControl+Alt+Shift+Super+K"
        );
        assert_eq!(canonical("Option+Space").unwrap(), "Alt+Space");
    }

    #[test]
    fn parsed_shortcut_exposes_modifiers_and_key() {
        let p = parse_shortcut(" Shift + Cmd + 7 ").unwrap();
        assert_eq!(
            p.modifiers,
            vec![ShortcutModifier::CommandOrControl, ShortcutModifier::Shift]
        );
        assert_eq!(p.key, "7");
    }

    #[test]
    fn bare_key_rejected() {
        assert_eq!(parse_shortcut("L"), Err(ShortcutError::NoModifier));
    }

    #[test]
    fn two_keys_rejected() {
        assert_eq!(parse_shortcut("Cmd+K+L"), Err(ShortcutError::MultipleKeys));
    }

    #[test]
    fn modifier_only_rejected() {
        assert_eq!(parse_shortcut("Cmd+Shift"), Err(ShortcutError::NoKey));
    }

    #[test]
    fn empty_and_malformed_rejected() {
        assert_eq!(parse_shortcut(""), Err(ShortcutError::Empty));
        assert_eq!(parse_shortcut("   "), Err(ShortcutError::Empty));
        assert_eq!(parse_shortcut("Cmd++L"), Err(ShortcutError::EmptyPart));
        assert!(matches!(
            parse_shortcut("Cmd+Hyper"),
            Err(ShortcutError::UnknownKey(_))
        ));
    }

    #[test]
    fn function_key_with_modifier_ok() {
        assert_eq!(canonical("ctrl+F5").unwrap(), "CommandOrControl+F5");
        assert_eq!(canonical("alt+f12").unwrap(), "Alt+F12");
        assert!(parse_shortcut("alt+F25").is_err());
    }

    #[test]
    fn named_keys_canonicalised() {
        assert_eq!(canonical("cmd+escape").unwrap(), "CommandOrControl+Escape");
        assert_eq!(canonical("cmd+pageup").unwrap(), "CommandOrControl+PageUp");
        assert_eq!(canonical("cmd+/").unwrap(), "CommandOrControl+Slash");
    }
}
