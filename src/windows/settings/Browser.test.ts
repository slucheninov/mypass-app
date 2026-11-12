import { screen, within } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import { renderSettingsApp } from "../../test/settingsApp";

describe("Browser", () => {
  it("browser_pane_is_placeholder_with_no_interactive_controls", async () => {
    await renderSettingsApp({}, { pane: "Browser" });
    const panel = screen.getByRole("tabpanel");
    expect(within(panel).getByText("Browser integration arrives in an upcoming version.")).toBeTruthy();
    expect(panel.querySelectorAll("button, input, select, textarea, a, [role=switch], [tabindex]").length).toBe(0);
  });
});
