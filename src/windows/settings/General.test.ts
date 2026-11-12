import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { calls } from "../../test/mainApp";
import { renderSettingsApp } from "../../test/settingsApp";

afterEach(() => {
  vi.useRealTimers();
});

const patches = (fake: Parameters<typeof calls>[0]) => calls(fake, "settingsUpdate").map((c) => c.args.patch);
const stepper = (name: string) => screen.getByRole("spinbutton", { name }) as HTMLInputElement;

describe("language", () => {
  it("offers System, English, Ukrainian and Russian", async () => {
    await renderSettingsApp();
    const select = screen.getByRole("combobox", { name: "Language" });
    expect(within(select).getAllByRole("option").map((o) => (o as HTMLOptionElement).value)).toEqual([
      "system",
      "en",
      "uk",
      "ru",
    ]);
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "System",
      "English",
      "Ukrainian",
      "Russian",
    ]);
  });
});

describe("clipboard", () => {
  it("clipboard_stepper_clamps_10_120_step_5", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { fake } = await renderSettingsApp();
    const input = stepper("Clear clipboard after");
    expect(input.value).toBe("30");
    expect(input.getAttribute("aria-valuetext")).toBe("30 s");
    for (const [typed, shown] of [["7", "10"], ["500", "120"], ["33", "35"]] as const) {
      await fireEvent.input(input, { target: { value: typed } });
      await fireEvent.change(input);
      expect(input.value).toBe(shown);
      await vi.advanceTimersByTimeAsync(300);
    }
    expect(patches(fake)).toEqual([
      { clipboardClearSeconds: 10 },
      { clipboardClearSeconds: 120 },
      { clipboardClearSeconds: 35 },
    ]);
  });

  it("ten rapid Up presses from 30 send one update after 300 ms", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { fake } = await renderSettingsApp();
    const input = stepper("Clear clipboard after");
    for (let i = 0; i < 10; i += 1) await fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(input.value).toBe("80");
    expect(patches(fake)).toEqual([]);
    await vi.advanceTimersByTimeAsync(299);
    expect(patches(fake)).toEqual([]);
    await vi.advanceTimersByTimeAsync(2);
    expect(patches(fake)).toEqual([{ clipboardClearSeconds: 80 }]);
  });

  it("a rejected write reverts the stepper and shows an error", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { fake } = await renderSettingsApp();
    fake.state.settingsFailure = "internal";
    const input = stepper("Clear clipboard after");
    await fireEvent.keyDown(input, { key: "ArrowUp" });
    await vi.advanceTimersByTimeAsync(400);
    await screen.findByRole("alert");
    await waitFor(() => expect(input.value).toBe("30"));
  });

  it("a pending change is sent when the pane closes before the timer fires", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { fake } = await renderSettingsApp();
    await fireEvent.keyDown(stepper("Clear clipboard after"), { key: "ArrowUp" });
    await fireEvent.click(screen.getByRole("tab", { name: "Vault" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(patches(fake)).toEqual([{ clipboardClearSeconds: 35 }]);
  });

  it("clipboard_setting_has_no_never_option", async () => {
    await renderSettingsApp();
    expect(screen.queryByRole("option", { name: "Never" })).toBeNull();
    expect(screen.queryByText("Never")).toBeNull();
  });

  it("no_tray_click_action_setting_rendered", async () => {
    await renderSettingsApp();
    expect(screen.queryByText(/Clicking the icon/)).toBeNull();
    expect(screen.queryByLabelText(/Clicking the icon/)).toBeNull();
  });
});

describe("menu bar icon", () => {
  it("toggles showMenuBarExtra immediately", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp();
    const toggle = screen.getByRole("switch", { name: "Show menu bar icon" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await user.click(toggle);
    expect(patches(fake)).toEqual([{ showMenuBarExtra: false }]);
    await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("false"));
  });
});

describe("hotkeys", () => {
  const lockRow = () => screen.getByRole("group", { name: "Lock hotkey" });
  const showRow = () => screen.getByRole("group", { name: "Show hotkey" });

  it("shows the stored shortcut with the platform's modifier names and an empty one as Record shortcut", async () => {
    await renderSettingsApp({ os: "macos" });
    expect(within(lockRow()).getByRole("button", { name: /Lock hotkey/ }).textContent).toBe("⌘⇧L");
    expect(within(showRow()).getByRole("button", { name: /Show hotkey/ }).textContent).toBe("Record shortcut");
  });

  it("uses Ctrl names on Windows", async () => {
    await renderSettingsApp({ os: "windows" });
    expect(within(lockRow()).getByRole("button", { name: /Lock hotkey/ }).textContent).toBe("Ctrl+Shift+L");
  });

  it("hotkey_recorder_captures_and_sends_canonical_string", async () => {
    const { fake } = await renderSettingsApp({ os: "macos" });
    const recorder = within(showRow()).getByRole("button", { name: /Show hotkey/ });
    recorder.focus();
    // Cyrillic layout: the physical key is recorded.
    await fireEvent.keyDown(recorder, { code: "KeyK", key: "\u043B", metaKey: true, altKey: true });
    expect(patches(fake)).toEqual([{ shortcutShow: "CommandOrControl+Alt+K" }]);
    await waitFor(() => expect(within(showRow()).getByRole("button", { name: /Show hotkey/ }).textContent).toBe("⌘⌥K"));
  });

  it("ignores a bare key and a modifier-only press while recording", async () => {
    const { fake } = await renderSettingsApp({ os: "macos" });
    const recorder = within(showRow()).getByRole("button", { name: /Show hotkey/ });
    recorder.focus();
    await fireEvent.keyDown(recorder, { code: "KeyK", key: "k" });
    await fireEvent.keyDown(recorder, { code: "MetaLeft", key: "Meta", metaKey: true });
    expect(patches(fake)).toEqual([]);
  });

  it("a refused combination says why instead of being swallowed silently", async () => {
    const { fake } = await renderSettingsApp({ os: "macos" });
    const recorder = within(showRow()).getByRole("button", { name: /Show hotkey/ });
    recorder.focus();
    // Control on macOS has no canonical spelling; Shift alone is not a real modifier.
    await fireEvent.keyDown(recorder, { code: "KeyK", key: "k", ctrlKey: true });
    await within(showRow()).findByText("That shortcut needs Command, Ctrl or Alt with a key");
    await fireEvent.keyDown(recorder, { code: "KeyK", key: "K", shiftKey: true });
    expect(within(showRow()).queryByText("That shortcut is reserved by the system")).toBeNull();
    // A reserved system combination gets its own message.
    await fireEvent.keyDown(recorder, { code: "KeyQ", key: "q", metaKey: true });
    await within(showRow()).findByText("That shortcut is reserved by the system");
    expect(patches(fake)).toEqual([]);
    // The next accepted shortcut clears the hint.
    await fireEvent.keyDown(recorder, { code: "KeyK", key: "k", metaKey: true, altKey: true });
    await waitFor(() => expect(within(showRow()).queryByText("That shortcut is reserved by the system")).toBeNull());
    expect(patches(fake)).toEqual([{ shortcutShow: "CommandOrControl+Alt+K" }]);
  });

  it("hotkey_clear_sends_empty_string", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp();
    await user.click(within(lockRow()).getByRole("button", { name: "Clear" }));
    expect(patches(fake)).toEqual([{ shortcutLock: "" }]);
  });

  it("shortcut_conflict_event_shows_inline_message", async () => {
    const { fake } = await renderSettingsApp();
    await fake.emit("shortcutConflictEvent", { kind: "show", shortcut: "CommandOrControl+Alt+K" });
    await within(showRow()).findByText("That shortcut is already in use");
    expect(within(lockRow()).queryByText("That shortcut is already in use")).toBeNull();
  });

  it("a new recording clears the conflict message", async () => {
    const { fake } = await renderSettingsApp({ os: "macos" });
    await fake.emit("shortcutConflictEvent", { kind: "lock", shortcut: "CommandOrControl+Shift+L" });
    await within(lockRow()).findByText("That shortcut is already in use");
    const recorder = within(lockRow()).getByRole("button", { name: /Lock hotkey/ });
    recorder.focus();
    await fireEvent.keyDown(recorder, { code: "KeyJ", key: "j", metaKey: true });
    await waitFor(() => expect(within(lockRow()).queryByText("That shortcut is already in use")).toBeNull());
  });

  it("never asks for the startup conflicts (only the main window takes them)", async () => {
    const { fake } = await renderSettingsApp();
    expect(calls(fake, "shortcutConflictsTake")).toEqual([]);
  });
});

describe("launch at login", () => {
  it("login_item_row_hidden_when_status_unsupported", async () => {
    await renderSettingsApp({}, { prepare: (fake) => (fake.state.loginItem = "unsupported") });
    expect(screen.queryByRole("switch", { name: "Launch at login" })).toBeNull();
  });

  it("reflects the status and re-reads it after each toggle", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp();
    const toggle = await screen.findByRole("switch", { name: "Launch at login" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    const before = calls(fake, "settingsLoginItemStatus").length;
    await user.click(toggle);
    expect(patches(fake)).toEqual([{ launchAtLogin: true }]);
    await waitFor(() => expect(calls(fake, "settingsLoginItemStatus").length).toBe(before + 1));
    await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("true"));
  });

  it("login_item_requires_approval_shows_message_and_keeps_switch_off", async () => {
    const user = userEvent.setup();
    await renderSettingsApp({}, { prepare: (fake) => (fake.state.loginItem = "requiresApproval") });
    const toggle = await screen.findByRole("switch", { name: "Launch at login" });
    await screen.findByText("Approval required. Open System Settings > General > Login Items and allow MyPass.");
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    await user.click(toggle);
    await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("false"));
  });

  it("login_item_error_message_is_shown_and_switch_reverts", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp({}, { prepare: (f) => (f.state.loginItemError = "Operation not permitted") });
    const toggle = await screen.findByRole("switch", { name: "Launch at login" });
    await user.click(toggle);
    await screen.findByText("Could not update login item: Operation not permitted");
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(fake.state.settings.launchAtLogin).toBe(false);
  });
});
