import { afterEach, describe, expect, it, vi } from "vitest";
import { createFakeBackend, type FakeBackendOptions } from "../../test/fakeBackend";
import { calls } from "../../test/mainApp";
import { __resetCatalogsForTest, i18n, setLanguage } from "../i18n/index.svelte";
import { session } from "./session.svelte";
import { applyTheme, settings } from "./settings.svelte";

/** A fake backend with an unlocked vault unless `opts` says otherwise. */
function backend(opts: FakeBackendOptions = {}) {
  return createFakeBackend({ session: "unlocked", ...opts });
}

/** Starts the settings store next to the session store, as `startStores` does. */
async function start(opts?: { followTheme?: boolean }) {
  await Promise.all([settings.start(opts), session.start()]);
  // The session's first answer may trigger a second (full) read; let it land.
  await new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => {
  settings.stop();
  session.stop();
  delete document.documentElement.dataset.theme;
  __resetCatalogsForTest();
  setLanguage("en");
});

describe("settings store", () => {
  it("the_tray_panel_ignores_the_theme_setting", async () => {
    const fake = backend();
    fake.state.settings.theme = "dark";
    fake.state.settings.language = "uk";
    fake.install();
    await start({ followTheme: false });
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(i18n.lang).toBe("uk");
    await fake.emit("settingsChanged", { ...fake.state.settings, theme: "light" });
    await vi.waitFor(() => expect(settings.value?.theme).toBe("light"));
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("start_loads_the_settings_and_applies_them", async () => {
    const fake = backend();
    fake.state.settings.theme = "dark";
    fake.state.settings.language = "uk";
    fake.install();
    await start();
    expect(settings.value?.theme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(i18n.lang).toBe("uk");
  });

  it("settings_update_applies_theme_and_language_immediately", async () => {
    const fake = backend();
    let release: (() => void) | undefined;
    const original = fake.handlers.settingsUpdate;
    fake.handlers.settingsUpdate = (args) =>
      new Promise((resolve) => {
        release = () => resolve(original(args));
      });
    fake.install();
    await start();
    const pending = settings.update({ theme: "dark", language: "ru" });
    // Applied before the backend answered.
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(i18n.lang).toBe("ru");
    await vi.waitFor(() => expect(release).toBeDefined());
    release?.();
    await pending;
    expect(settings.value?.theme).toBe("dark");
    expect(fake.state.settings.language).toBe("ru");
  });

  it("a_failed_update_restores_theme_and_language", async () => {
    const fake = backend();
    fake.handlers.settingsUpdate = () => {
      throw { code: "io", message: "" };
    };
    fake.install();
    await start();
    await expect(settings.update({ theme: "dark", language: "ru" })).rejects.toMatchObject({ code: "io" });
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
    expect(i18n.lang).toBe("en");
    expect(settings.value?.theme).toBe("system");
  });

  it("settings_changed_event_updates_store", async () => {
    const fake = backend();
    fake.install();
    await start();
    await fake.emit("settingsChanged", { ...fake.state.settings, theme: "light", clipboardClearSeconds: 45 });
    await vi.waitFor(() => expect(settings.value?.clipboardClearSeconds).toBe(45));
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});

describe("settings store while the vault is not unlocked", () => {
  it("reads only the public settings and still applies theme and language", async () => {
    const fake = backend({ session: "locked" });
    fake.state.settings.theme = "dark";
    fake.state.settings.language = "uk";
    fake.install();
    await start();
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(i18n.lang).toBe("uk");
    expect(settings.value).toBeNull();
    expect(settings.public?.theme).toBe("dark");
    expect(calls(fake, "settingsGet")).toEqual([]);
    expect(calls(fake, "settingsGetPublic").length).toBeGreaterThan(0);
  });

  it("a lock drops the full settings and keeps the appearance", async () => {
    const fake = backend();
    fake.state.settings.theme = "dark";
    fake.install();
    await start();
    expect(settings.value?.theme).toBe("dark");
    await fake.handlers.vaultLock({});
    await vi.waitFor(() => expect(settings.value).toBeNull());
    expect(settings.public?.theme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("a settings event while locked updates the appearance only", async () => {
    const fake = backend({ session: "locked" });
    fake.install();
    await start();
    await fake.emit("settingsChanged", { ...fake.state.settings, theme: "light", clipboardClearSeconds: 45 });
    await vi.waitFor(() => expect(document.documentElement.dataset.theme).toBe("light"));
    expect(settings.value).toBeNull();
    expect(settings.public?.theme).toBe("light");
  });

  it("an unlock loads the full settings", async () => {
    const fake = backend({ session: "locked" });
    fake.install();
    await start();
    fake.state.session = { ...fake.state.session, state: "unlocked", lockReason: null };
    await fake.emit("sessionChanged", structuredClone(fake.state.session));
    await vi.waitFor(() => expect(settings.value?.clipboardClearSeconds).toBe(30));
  });
});

describe("applyTheme", () => {
  it("theme_system_removes_data_theme_attribute", () => {
    document.documentElement.dataset.theme = "dark";
    applyTheme("system");
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });
  it("theme_dark_sets_data_theme_dark", () => {
    applyTheme("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
  it("theme_light_sets_data_theme_light", () => {
    applyTheme("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});
