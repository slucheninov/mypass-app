// Keyboard events and canonical global-shortcut strings. The canonical spelling is the backend's
// (`mypass_session::shortcut`): modifiers in the order CommandOrControl, Alt, Shift, Super, then one
// key, joined by `+`. `CommandOrControl` is Cmd on macOS and Ctrl elsewhere, so a shortcut means the
// same on both systems.
import type { Os } from "./platform.svelte";

const NAMED_KEYS: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  Escape: "Escape",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
  Minus: "Minus",
  Equal: "Equal",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  Backquote: "Backquote",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
};

/** The canonical key for a physical `KeyboardEvent.code`, or `null` for a modifier or an unsupported key. */
function keyOfCode(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter?.[1]) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit?.[1]) return digit[1];
  const fn = /^F([1-9]|1[0-9]|2[0-4])$/.exec(code);
  if (fn) return `F${fn[1]}`;
  return Object.hasOwn(NAMED_KEYS, code) ? (NAMED_KEYS[code] ?? null) : null;
}

/**
 * The canonical shortcut for a `keydown`, or `null` when it is not a usable shortcut: a bare key, a
 * modifier-only press, a key the backend cannot parse, or (on macOS) a Control press, which has no
 * canonical spelling because the backend reads `ctrl` as Cmd. The key comes from `code`, the
 * physical key, so a Cyrillic layout records the same shortcut as a Latin one.
 */
export function shortcutFromEvent(e: KeyboardEvent, os: Os | null): string | null {
  const key = keyOfCode(e.code);
  if (key === null) return null;
  const mac = os === "macos";
  if (mac && e.ctrlKey) return null;
  const parts: string[] = [];
  if (mac ? e.metaKey : e.ctrlKey) parts.push("CommandOrControl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (!mac && e.metaKey) parts.push("Super");
  // A shortcut needs a real modifier: Shift alone is not enough.
  if (!parts.some((p) => p !== "Shift")) return null;
  return [...parts, key].join("+");
}

const MAC_SYMBOLS: Record<string, string> = {
  CommandOrControl: "⌘",
  Alt: "⌥",
  Shift: "⇧",
  Super: "⌃",
};

function modifierName(part: string, os: Os | null): string {
  switch (part) {
    case "CommandOrControl":
      return "Ctrl";
    case "Super":
      return os === "windows" ? "Win" : "Super";
    default:
      return part;
  }
}

/** A canonical shortcut as the platform writes it: symbols on macOS (`⌘⇧L`), names elsewhere (`Ctrl+Shift+L`). */
export function formatShortcut(canonical: string, os: Os | null): string {
  if (canonical === "") return "";
  const parts = canonical.split("+");
  if (os === "macos") return parts.map((p) => MAC_SYMBOLS[p] ?? p).join("");
  return parts.map((p) => modifierName(p, os)).join("+");
}
