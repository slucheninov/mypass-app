import { describe, expect, it } from "vitest";
import { formatShortcut, recordShortcut, shortcutFromEvent } from "./shortcuts";

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

describe("refused combinations", () => {
  it("shift_plus_letter_without_another_modifier_is_refused_deliberately_unlike_the_backend", () => {
    // `parse_shortcut` accepts Shift+L; the recorder does not, so a global shortcut never steals capitals.
    const e = key({ code: "KeyL", key: "L", shiftKey: true });
    expect(recordShortcut(e, "windows")).toEqual({ refused: "modifiers" });
    expect(recordShortcut(e, "macos")).toEqual({ refused: "modifiers" });
  });

  it("a_control_press_on_macos_is_refused_with_a_reason", () => {
    expect(recordShortcut(key({ code: "KeyL", key: "l", ctrlKey: true }), "macos")).toEqual({ refused: "modifiers" });
  });

  it("a_bare_key_and_a_modifier_only_press_say_nothing", () => {
    expect(recordShortcut(key({ code: "KeyL", key: "l" }), "macos")).toBeNull();
    expect(recordShortcut(key({ code: "MetaLeft", key: "Meta", metaKey: true }), "macos")).toBeNull();
  });

  it.each([
    ["macos", { code: "KeyQ", metaKey: true }],
    ["macos", { code: "KeyW", metaKey: true }],
    ["macos", { code: "Tab", metaKey: true }],
    ["macos", { code: "Space", metaKey: true }],
    ["windows", { code: "KeyQ", ctrlKey: true }],
    ["windows", { code: "KeyW", ctrlKey: true }],
    ["windows", { code: "F4", altKey: true }],
    ["windows", { code: "Tab", altKey: true }],
    ["windows", { code: "Delete", ctrlKey: true, altKey: true }],
  ] as const)("reserved_system_combination_is_refused_on_%s_%j", (os, init) => {
    const e = key({ key: "", ...init });
    expect(recordShortcut(e, os)).toEqual({ refused: "reserved" });
    expect(shortcutFromEvent(e, os)).toBeNull();
  });

  it("windows_only_combinations_stay_recordable_on_macos_where_they_mean_nothing_special", () => {
    expect(shortcutFromEvent(key({ code: "F4", key: "F4", altKey: true }), "macos")).toBe("Alt+F4");
    expect(shortcutFromEvent(key({ code: "Space", key: " ", ctrlKey: true }), "windows")).toBe("CommandOrControl+Space");
  });
});

describe("formatShortcut", () => {
  it("shows_super_as_command_on_macos", () => {
    expect(formatShortcut("Super+L", "macos")).toBe("\u2318L");
    expect(formatShortcut("Super+L", "windows")).toBe("Win+L");
  });
});
