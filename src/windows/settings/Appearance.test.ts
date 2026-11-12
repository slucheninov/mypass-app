import { screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { calls } from "../../test/mainApp";
import { renderSettingsApp } from "../../test/settingsApp";

afterEach(() => {
  delete document.documentElement.dataset.theme;
});

describe("Appearance", () => {
  it("theme_segmented_applies_immediately_and_persists_via_update", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp({}, { pane: "Appearance" });
    expect(screen.getByRole("radio", { name: "System" }).getAttribute("aria-checked")).toBe("true");
    await user.click(screen.getByRole("radio", { name: "Dark" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([{ theme: "dark" }]);
    await waitFor(() => expect(fake.state.settings.theme).toBe("dark"));
    await waitFor(() => expect(screen.getByRole("radio", { name: "Dark" }).getAttribute("aria-checked")).toBe("true"));
    await user.click(screen.getByRole("radio", { name: "System" }));
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("a rejected theme change goes back and shows the error", async () => {
    const user = userEvent.setup();
    const { fake } = await renderSettingsApp({}, { pane: "Appearance" });
    fake.state.settingsFailure = "internal";
    await user.click(screen.getByRole("radio", { name: "Light" }));
    await screen.findByRole("alert");
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});
