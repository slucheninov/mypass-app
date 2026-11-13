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

/** Why a key press with a valid key was not recorded. */
export type ShortcutRefusal = "modifiers" | "reserved";

export type ShortcutRecording = { shortcut: string } | { refused: ShortcutRefusal } | null;

// Combinations the system owns. Registering one globally would hijack it in every other app
// (Ctrl+Q on Windows quits editors) or silently never fire (the OS consumes it first). Spelled
// in the canonical form, so one set serves the recorder and a test per entry.
const RESERVED_EVERYWHERE = new Set(["CommandOrControl+Q", "CommandOrControl+W", "CommandOrControl+Tab"]);
const RESERVED_MACOS = new Set(["CommandOrControl+Space"]); // Spotlight
const RESERVED_OTHER = new Set(["Alt+F4", "Alt+Tab", "CommandOrControl+Alt+Delete"]);

function isReserved(canonical: string, os: Os | null): boolean {
  if (RESERVED_EVERYWHERE.has(canonical)) return true;
  return os === "macos" ? RESERVED_MACOS.has(canonical) : RESERVED_OTHER.has(canonical);
}

// Deliberate exception to "nothing is chosen by OS name" (the plan's Global Constraints list the
// window controls and the Finder/Explorer label): Cmd and Ctrl are different physical modifiers
// whose canonical spelling is the same, and the glyphs differ, so the recorder and the formatter
// must know the OS. Nothing else in the UI branches on it for shortcuts.

/**
 * Classifies a `keydown`: a canonical shortcut, a refusal (a valid key with a modifier set that
 * cannot be used, so the UI can say why), or `null` for nothing to say (a bare key, a modifier-only
 * press, a key the backend cannot parse). The key comes from `code`, the physical key, so a
 * Cyrillic layout records the same shortcut as a Latin one.
 */
export function recordShortcut(e: KeyboardEvent, os: Os | null): ShortcutRecording {
  const key = keyOfCode(e.code);
  if (key === null) return null;
  const mac = os === "macos";
  // macOS Control has no canonical spelling: the backend reads `ctrl` as Cmd, so recording it
  // would register the wrong key.
  if (mac && e.ctrlKey) return { refused: "modifiers" };
  const parts: string[] = [];
  if (mac ? e.metaKey : e.ctrlKey) parts.push("CommandOrControl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (!mac && e.metaKey) parts.push("Super");
  if (parts.length === 0) return null;
  // Shift+letter-only is refused deliberately, although the backend's `parse_shortcut` accepts it
  // (a hand-edited settings file may hold one): a global Shift+letter would steal every capital
  // letter typed in any app. A shortcut needs a real modifier besides Shift.
  if (!parts.some((p) => p !== "Shift")) return { refused: "modifiers" };
  const canonical = [...parts, key].join("+");
  if (isReserved(canonical, os)) return { refused: "reserved" };
  return { shortcut: canonical };
}

/** The canonical shortcut for a `keydown`, or `null` when it is not a usable one (see `recordShortcut`). */
export function shortcutFromEvent(e: KeyboardEvent, os: Os | null): string | null {
  const r = recordShortcut(e, os);
  return r !== null && "shortcut" in r ? r.shortcut : null;
}

// Super is Cmd on macOS (the global-shortcut plugin treats it so); Control has no spelling there.
const MAC_SYMBOLS: Record<string, string> = {
  CommandOrControl: "⌘",
  Alt: "⌥",
  Shift: "⇧",
  Super: "⌘",
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
