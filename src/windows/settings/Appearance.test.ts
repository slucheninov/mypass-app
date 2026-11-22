import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { calls } from "../../test/mainApp";
import { renderSettingsApp } from "../../test/settingsApp";

afterEach(() => {
  delete document.documentElement.dataset.theme;
});

describe("Appearance", () => {
  it("the_app_icon_row_offers_system_light_dark_and_patches_app_icon", async () => {
    const { fake } = await renderSettingsApp({}, { pane: "Appearance" });
    const group = within(screen.getByRole("radiogroup", { name: "App icon" }));
    expect(group.getAllByRole("radio").map((option) => option.textContent?.trim()))
      .toEqual(["System", "Light", "Dark"]);
    await userEvent.setup().click(group.getByRole("radio", { name: "Dark" }));
    expect(calls(fake, "settingsUpdate")[0]?.args).toEqual({ patch: { appIcon: "dark" } });
    expect(fake.state.settings.theme).toBe("system");
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("theme_segmented_applies_immediately_and_persists_via_update", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp({}, { pane: "Appearance" });
    const theme = within(screen.getByRole("radiogroup", { name: "Theme" }));
    expect(theme.getByRole("radio", { name: "System" }).getAttribute("aria-checked")).toBe("true");
    await user.click(theme.getByRole("radio", { name: "Dark" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([{ theme: "dark" }]);
    await waitFor(() => expect(fake.state.settings.theme).toBe("dark"));
    await waitFor(() => expect(theme.getByRole("radio", { name: "Dark" }).getAttribute("aria-checked")).toBe("true"));
    await user.click(theme.getByRole("radio", { name: "System" }));
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("a rejected theme change goes back and shows the error", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp({}, { pane: "Appearance" });
    fake.state.settingsFailure = "internal";
    await user.click(within(screen.getByRole("radiogroup", { name: "Theme" })).getByRole("radio", { name: "Light" }));
    await screen.findByRole("alert");
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});
