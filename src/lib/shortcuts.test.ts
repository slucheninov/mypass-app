import { describe, expect, it } from "vitest";
import { formatShortcut, shortcutFromEvent } from "./shortcuts";

function key(init: KeyboardEventInit & { code: string }): KeyboardEvent {
  return new KeyboardEvent("keydown", init);
}

describe("shortcutFromEvent", () => {
  it("shortcut_from_event_meta_shift_l_on_macos", () => {
    expect(shortcutFromEvent(key({ code: "KeyL", key: "l", metaKey: true, shiftKey: true }), "macos")).toBe(
      "CommandOrControl+Shift+L",
    );
  });

  it("shortcut_from_event_ctrl_shift_l_on_windows", () => {
    expect(shortcutFromEvent(key({ code: "KeyL", key: "L", ctrlKey: true, shiftKey: true }), "windows")).toBe(
      "CommandOrControl+Shift+L",
    );
  });

  it("shortcut_bare_key_returns_null", () => {
    expect(shortcutFromEvent(key({ code: "KeyL", key: "l" }), "macos")).toBeNull();
    expect(shortcutFromEvent(key({ code: "KeyL", key: "L", shiftKey: true }), "windows")).toBe(
      null,
    );
  });

  it("shortcut_modifier_only_returns_null", () => {
    expect(shortcutFromEvent(key({ code: "MetaLeft", key: "Meta", metaKey: true }), "macos")).toBeNull();
    expect(shortcutFromEvent(key({ code: "ShiftLeft", key: "Shift", shiftKey: true, ctrlKey: true }), "windows")).toBeNull();
  });

  it("records the physical key, so a non-Latin layout gives the same shortcut", () => {
    // Cyrillic layout: `key` is the Cyrillic letter, `code` is still KeyL.
    expect(shortcutFromEvent(key({ code: "KeyL", key: "\u0434", metaKey: true, shiftKey: true }), "macos")).toBe(
      "CommandOrControl+Shift+L",
    );
  });

  it("orders modifiers like the backend and maps digits, function and named keys", () => {
    expect(shortcutFromEvent(key({ code: "Digit7", key: "7", metaKey: true, altKey: true, shiftKey: true }), "macos")).toBe(
      "CommandOrControl+Alt+Shift+7",
    );
    expect(shortcutFromEvent(key({ code: "F5", key: "F5", ctrlKey: true }), "linux")).toBe("CommandOrControl+F5");
    expect(shortcutFromEvent(key({ code: "Space", key: " ", ctrlKey: true }), "windows")).toBe("CommandOrControl+Space");
    expect(shortcutFromEvent(key({ code: "ArrowUp", key: "ArrowUp", ctrlKey: true }), "windows")).toBe(
      "CommandOrControl+ArrowUp",
    );
  });

  it("maps the Windows key to Super and refuses a Control press on macOS (no canonical spelling)", () => {
    expect(shortcutFromEvent(key({ code: "KeyL", key: "l", metaKey: true }), "windows")).toBe("Super+L");
    expect(shortcutFromEvent(key({ code: "KeyL", key: "l", ctrlKey: true }), "macos")).toBeNull();
  });

  it("returns null for a key the backend cannot parse", () => {
    expect(shortcutFromEvent(key({ code: "Numpad1", key: "1", ctrlKey: true }), "windows")).toBeNull();
    expect(shortcutFromEvent(key({ code: "", key: "Unidentified", ctrlKey: true }), "windows")).toBeNull();
  });
});

describe("formatShortcut", () => {
  it("uses the platform's modifier names", () => {
    expect(formatShortcut("CommandOrControl+Shift+L", "macos")).toBe("⌘⇧L");
    expect(formatShortcut("CommandOrControl+Shift+L", "windows")).toBe("Ctrl+Shift+L");
    expect(formatShortcut("CommandOrControl+Alt+Super+F5", "linux")).toBe("Ctrl+Alt+Super+F5");
    expect(formatShortcut("", "macos")).toBe("");
  });
});
