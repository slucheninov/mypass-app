import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import { loadPlatform, platform } from "../lib/platform.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { createFakeBackend } from "../test/fakeBackend";
import { renderMainApp } from "../test/mainApp";
import { renderWithBackend } from "../test/render";
import ShellHarness from "../test/ShellHarness.svelte";

afterEach(() => localStorage.clear());

async function shell(opts: Parameters<typeof renderMainApp>[0] = { session: "unlocked" }) {
  const app = await renderMainApp(opts);
  await screen.findByTestId("vault-shell");
  return app;
}

const sidebarHandle = () => screen.getByRole("separator", { name: "Resize sidebar" });
const listHandle = () => screen.getByRole("separator", { name: "Resize list" });
const now = (el: HTMLElement) => el.getAttribute("aria-valuenow");

describe("layout", () => {
  it("shell_has_three_panes_with_default_widths", async () => {
    await shell();
    expect(now(sidebarHandle())).toBe("220");
    expect(now(listHandle())).toBe("320");
    expect(sidebarHandle().getAttribute("aria-valuemin")).toBe("180");
    expect(listHandle().getAttribute("aria-valuemin")).toBe("260");
  });

  it("shell_renders_the_list_and_detail_slots", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    renderWithBackend(ShellHarness, {}, fake);
    expect(screen.getByText("list pane")).toBeTruthy();
    expect(screen.getByText("detail pane")).toBeTruthy();
  });

  it("splitter_widths_persist_and_clamp", async () => {
    // Stored values below the minimums are raised to them, absurd ones are capped.
    localStorage.setItem("layout.widths", JSON.stringify([10, 99999]));
    await shell();
    expect(now(sidebarHandle())).toBe("180");
    expect(now(listHandle())).toBe("640");
    // A keyboard resize is persisted under `layout.widths`.
    sidebarHandle().focus();
    await fireEvent.keyDown(sidebarHandle(), { key: "ArrowRight" });
    expect(now(sidebarHandle())).toBe("196");
    expect(JSON.parse(localStorage.getItem("layout.widths") ?? "null")).toEqual([196, 640]);
    // Shrinking stops at the pane minimum.
    for (let i = 0; i < 5; i++) await fireEvent.keyDown(sidebarHandle(), { key: "ArrowLeft" });
    expect(now(sidebarHandle())).toBe("180");
  });

  it("broken_stored_widths_fall_back_to_the_defaults", async () => {
    localStorage.setItem("layout.widths", "not json");
    await shell();
    expect(now(sidebarHandle())).toBe("220");
    expect(now(listHandle())).toBe("320");
  });

  it("stored_widths_of_the_wrong_type_fall_back_per_pane", async () => {
    localStorage.setItem("layout.widths", JSON.stringify(["x", 300]));
    await shell();
    expect(now(sidebarHandle())).toBe("220");
    expect(now(listHandle())).toBe("300");
  });
});

describe("window controls", () => {
  it("window_controls_follow_os_from_app_info", async () => {
    await shell({ session: "unlocked", os: "macos" });
    expect(screen.getByTestId("titlebar-inset")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Minimize" })).toBeNull();
  });

  it("windows_gets_its_own_buttons_at_the_end_of_the_title_bar", async () => {
    await shell({ session: "unlocked", os: "windows" });
    expect(screen.queryByTestId("titlebar-inset")).toBeNull();
    const close = screen.getByRole("button", { name: "Close" });
    expect(screen.getByRole("button", { name: "Minimize" })).toBeTruthy();
    const toolbar = screen.getByTestId("toolbar");
    // The controls come after the toolbar in the title bar.
    expect(toolbar.compareDocumentPosition(close) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("nothing_is_drawn_until_the_os_is_known", async () => {
    platform.os = null;
    const fake = createFakeBackend({ session: "unlocked" });
    renderWithBackend(ShellHarness, {}, fake);
    expect(screen.queryByTestId("titlebar-inset")).toBeNull();
    expect(screen.queryByRole("button", { name: "Minimize" })).toBeNull();
    await loadPlatform();
    await waitFor(() => expect(screen.getByTestId("titlebar-inset")).toBeTruthy());
  });
});

describe("accessibility", () => {
  it("shell_passes_axe", async () => {
    await shell();
    await screen.findByRole("treeitem", { name: /^Personal/ });
    await expectNoAxeViolations(document.body);
  });
});
