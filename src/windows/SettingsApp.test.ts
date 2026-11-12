import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { settings } from "../lib/stores/settings.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { calls } from "../test/mainApp";
import { renderSettingsApp } from "../test/settingsApp";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  try {
    window.localStorage.clear();
  } catch {
    // ignore
  }
});

const PANES = ["General", "Vault", "Security", "Browser", "Appearance", "About"];

describe("SettingsApp", () => {
  it("lists the six panes and opens General first", async () => {
    await renderSettingsApp();
    const tabs = within(screen.getByRole("tablist")).getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent?.trim())).toEqual(PANES);
    expect(screen.getByRole("tab", { name: "General" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("General");
  });

  it("language_change_updates_ui_language_immediately", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp();
    await user.selectOptions(screen.getByRole("combobox", { name: "Language" }), "uk");
    // The pane titles switch to the catalog values at once, before the answer arrives.
    expect(screen.getByRole("tab", { name: "\u0417\u0430\u0433\u0430\u043B\u044C\u043D\u0456" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "\u0421\u0445\u043E\u0432\u0438\u0449\u0435" })).toBeTruthy();
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([{ language: "uk" }]);
    await waitFor(() => expect(fake.state.settings.language).toBe("uk"));
    expect(document.documentElement.lang).toBe("uk");
  });

  it("a rejected language change puts the old language back and shows the error", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp();
    fake.state.settingsFailure = "internal";
    await user.selectOptions(screen.getByRole("combobox", { name: "Language" }), "ru");
    await screen.findByRole("alert");
    expect(screen.getByRole("tab", { name: "General" })).toBeTruthy();
    expect((screen.getByRole("combobox", { name: "Language" }) as HTMLSelectElement).value).toBe("system");
  });

  it("settings_changed_event_updates_open_window", async () => {
    const { fake } = await renderSettingsApp({}, { pane: "Security" });
    expect((screen.getByRole("combobox", { name: "Lock after inactivity" }) as HTMLSelectElement).value).toBe(
      "thirtyMinutes",
    );
    fake.state.settings = { ...fake.state.settings, autoLockIdleTimeout: "oneHour", lockOnScreenLock: false };
    await fake.emit("settingsChanged", structuredClone(fake.state.settings));
    await waitFor(() =>
      expect((screen.getByRole("combobox", { name: "Lock after inactivity" }) as HTMLSelectElement).value).toBe("oneHour"),
    );
    expect(screen.getByRole("switch", { name: "Lock when the screen locks" }).getAttribute("aria-checked")).toBe("false");
  });

  it("last_pane_remembered_in_local_storage_with_try_catch", async () => {
    const user = userEvent.setup();
    const first = await renderSettingsApp();
    await user.click(screen.getByRole("tab", { name: "Security" }));
    expect(window.localStorage.getItem("layout.settingsPane")).toBe("Security");
    first.unmount();
    await renderSettingsApp({}, { pane: "Security" });
    expect(screen.getByRole("tab", { name: "Security" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Security");
  });

  it("works when localStorage throws", async () => {
    const user = userEvent.setup();
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    await renderSettingsApp();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("General");
    await user.click(screen.getByRole("tab", { name: "About" }));
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("About");
  });

  it("an unknown remembered pane falls back to General", async () => {
    await renderSettingsApp({}, { pane: "Nonsense" });
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("General");
  });

  it("arrow keys move between the panes", async () => {
    await renderSettingsApp();
    const general = screen.getByRole("tab", { name: "General" });
    general.focus();
    await fireEvent.keyDown(general, { key: "ArrowDown" });
    expect(screen.getByRole("tab", { name: "Vault" }).getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(screen.getByRole("tab", { name: "Vault" }));
  });

  it("settings_pass_axe", async () => {
    const user = userEvent.setup();
    const { container } = await renderSettingsApp({ deviceUnlockAvailable: true }, {
      prepare: (fake) => {
        fake.state.backups = [{ id: "backup-1700000000.kdbx", created: 1_700_000_000_000, size: 4096 }];
        fake.state.backupFailure = { message: "disk full", at: 1_700_000_100_000 };
        fake.state.loginItem = "requiresApproval";
      },
    });
    for (const name of PANES) {
      await user.click(screen.getByRole("tab", { name }));
      await screen.findByRole("heading", { level: 1, name });
      await expectNoAxeViolations(container);
    }
  });

  it("does not wait for the entries: the settings window never lists them", async () => {
    const { fake } = await renderSettingsApp();
    expect(calls(fake, "entryList")).toEqual([]);
    expect(settings.value).not.toBeNull();
  });
});
