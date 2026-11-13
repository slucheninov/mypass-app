import { afterEach, describe, expect, it, vi } from "vitest";
import { createFakeBackend } from "../../test/fakeBackend";
import { __resetCatalogsForTest, i18n, setLanguage } from "../i18n/index.svelte";
import { applyTheme, settings } from "./settings.svelte";

afterEach(() => {
  settings.stop();
  delete document.documentElement.dataset.theme;
  __resetCatalogsForTest();
  setLanguage("en");
});

describe("settings store", () => {
  it("start_loads_the_settings_and_applies_them", async () => {
    const fake = createFakeBackend();
    fake.state.settings.theme = "dark";
    fake.state.settings.language = "uk";
    fake.install();
    await settings.start();
    expect(settings.value?.theme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(i18n.lang).toBe("uk");
  });

  it("settings_update_applies_theme_and_language_immediately", async () => {
    const fake = createFakeBackend();
    let release: (() => void) | undefined;
    const original = fake.handlers.settingsUpdate;
    fake.handlers.settingsUpdate = (args) =>
      new Promise((resolve) => {
        release = () => resolve(original(args));
      });
    fake.install();
    await settings.start();
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
    const fake = createFakeBackend();
    fake.handlers.settingsUpdate = () => {
      throw { code: "io", message: "" };
    };
    fake.install();
    await settings.start();
    await expect(settings.update({ theme: "dark", language: "ru" })).rejects.toMatchObject({ code: "io" });
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
    expect(i18n.lang).toBe("en");
    expect(settings.value?.theme).toBe("system");
  });

  it("settings_changed_event_updates_store", async () => {
    const fake = createFakeBackend();
    fake.install();
    await settings.start();
    await fake.emit("settingsChanged", { ...fake.state.settings, theme: "light", clipboardClearSeconds: 45 });
    await vi.waitFor(() => expect(settings.value?.clipboardClearSeconds).toBe(45));
    expect(document.documentElement.dataset.theme).toBe("light");
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
