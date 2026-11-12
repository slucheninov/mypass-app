import { screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import { calls } from "../../test/mainApp";
import { renderSettingsApp } from "../../test/settingsApp";

describe("About", () => {
  it("about_shows_version_from_app_info_and_license", async () => {
    await renderSettingsApp({}, { pane: "About" });
    expect(await screen.findByText("Version 0.0.0-test")).toBeTruthy();
    expect(screen.getByText("Licensed under Apache-2.0")).toBeTruthy();
    expect(screen.getByText("Third-party notices")).toBeTruthy();
  });

  it("shows the repository addresses as selectable text, not links (the settings window has no opener)", async () => {
    const { fake } = await renderSettingsApp({}, { pane: "About" });
    await screen.findByText("Version 0.0.0-test");
    expect(screen.queryAllByRole("link")).toEqual([]);
    expect(screen.getByText("https://github.com/slucheninov/mypass-app")).toBeTruthy();
    expect(calls(fake, "appInfo").length).toBeGreaterThan(0);
  });
});
