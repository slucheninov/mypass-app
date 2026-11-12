import { afterEach, describe, expect, it } from "vitest";
import { applyTheme } from "./settings.svelte";

afterEach(() => {
  delete document.documentElement.dataset.theme;
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
