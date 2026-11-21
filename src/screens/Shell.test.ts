import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, onTestFinished, vi } from "vitest";
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

describe("sidebar collapse", () => {
  const sidebarPane = () => screen.getByTestId("split-first");
  // jsdom keeps `inert` as a property only (browsers reflect it as an attribute).
  const isHidden = () => sidebarPane().inert && sidebarPane().style.visibility === "hidden";
  const shortcut = (target: HTMLElement) =>
    fireEvent.keyDown(target, { code: "KeyS", key: "s", ctrlKey: true, metaKey: true });

  it("toolbar_toggle_hides_and_shows_the_sidebar_and_persists", async () => {
    const user = userEvent.setup();
    await shell();
    const hide = screen.getByRole("button", { name: "Hide sidebar" });
    expect(hide.getAttribute("aria-expanded")).toBe("true");
    expect(hide.getAttribute("aria-controls")).toBe(sidebarPane().id);
    expect(isHidden()).toBe(false);
    await user.click(hide);
    const show = screen.getByRole("button", { name: "Show sidebar" });
    expect(show.getAttribute("aria-expanded")).toBe("false");
    expect(isHidden()).toBe(true);
    expect(sidebarPane().style.width).toBe("0px");
    // The sidebar separator goes with it; the list one stays.
    expect(screen.queryByRole("separator", { name: "Resize sidebar" })).toBeNull();
    expect(listHandle()).toBeTruthy();
    expect(localStorage.getItem("layout.sidebarCollapsed")).toBe("true");
    await user.click(show);
    expect(isHidden()).toBe(false);
    expect(now(sidebarHandle())).toBe("220");
    expect(localStorage.getItem("layout.sidebarCollapsed")).toBe("false");
  });

  it("the_sidebar_toggle_glyph_matches_the_other_toolbar_icons", async () => {
    await shell();
    const glyph = (name: string) => screen.getByRole("button", { name }).querySelector("svg");
    // The thin sidebar outline is drawn larger than the 16 px glyphs to carry the same weight,
    // inside the same 26 px button.
    expect(glyph("Hide sidebar")?.getAttribute("width")).toBe("18");
    expect(glyph("Lock")?.getAttribute("width")).toBe("16");
    expect(glyph("Settings")?.getAttribute("width")).toBe("16");
  });

  it("stored_collapsed_state_is_restored", async () => {
    localStorage.setItem("layout.sidebarCollapsed", "true");
    await shell();
    expect(isHidden()).toBe(true);
    expect(screen.getByRole("button", { name: "Show sidebar" })).toBeTruthy();
  });

  it.each([
    { width: 1000, sidebar: 220, list: 418 },
    { width: 820, sidebar: 198, list: 260 },
  ])("expanding at $width px leaves room for the detail minimum", async ({ width, sidebar, list }) => {
    const clientWidth = vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(width);
    onTestFinished(() => { clientWidth.mockRestore(); });
    const user = userEvent.setup();
    await shell();
    await user.click(screen.getByRole("button", { name: "Hide sidebar" }));
    await fireEvent.pointerDown(listHandle(), { clientX: 320, pointerId: 1 });
    await fireEvent.pointerMove(window, { clientX: 1000, pointerId: 1 });
    await fireEvent.pointerUp(window, { pointerId: 1 });
    expect(Number(now(listHandle()))).toBeGreaterThan(list);

    await user.click(screen.getByRole("button", { name: "Show sidebar" }));
    const restoredSidebar = Number(now(sidebarHandle()));
    const restoredList = Number(now(listHandle()));
    expect([restoredSidebar, restoredList]).toEqual([sidebar, list]);
    expect(width - restoredSidebar - restoredList - 2).toBe(360);
    expect(JSON.parse(localStorage.getItem("layout.widths") ?? "null")).toEqual([sidebar, list]);
  });

  it("control_command_s_toggles_the_sidebar_on_macos", async () => {
    await shell({ session: "unlocked", os: "macos" });
    expect(screen.getByRole("button", { name: "Hide sidebar" }).getAttribute("aria-keyshortcuts")).toBe("Control+Meta+S");
    // The Windows chord does nothing on macOS.
    await fireEvent.keyDown(window, { code: "KeyS", key: "S", ctrlKey: true, shiftKey: true });
    expect(isHidden()).toBe(false);
    await fireEvent.keyDown(window, { code: "KeyS", key: "s", ctrlKey: true, metaKey: true });
    expect(isHidden()).toBe(true);
    await fireEvent.keyDown(window, { code: "KeyS", key: "s", ctrlKey: true, metaKey: true });
    expect(isHidden()).toBe(false);
  });

  it("ctrl_shift_s_toggles_the_sidebar_on_windows", async () => {
    await shell({ session: "unlocked", os: "windows" });
    expect(screen.getByRole("button", { name: "Hide sidebar" }).getAttribute("aria-keyshortcuts")).toBe("Control+Shift+S");
    // A Cyrillic layout sends another `key` for the same physical key.
    await fireEvent.keyDown(window, { code: "KeyS", key: "\u042B", ctrlKey: true, shiftKey: true });
    expect(isHidden()).toBe(true);
    await fireEvent.keyDown(window, { code: "KeyS", key: "s", ctrlKey: true });
    expect(isHidden()).toBe(true);
  });

  it("collapsing_from_inside_the_sidebar_moves_focus_to_the_list", async () => {
    await shell();
    const all = await screen.findByRole("treeitem", { name: /^All/ });
    all.focus();
    await fireEvent.keyDown(all, { code: "KeyS", key: "s", ctrlKey: true, metaKey: true });
    expect(isHidden()).toBe(true);
    expect(document.activeElement).toBe(screen.getByRole("listbox"));
  });

  it("collapsing_from_the_sidebar_separator_moves_focus_to_the_list", async () => {
    await shell();
    const handle = sidebarHandle();
    handle.focus();
    expect(document.activeElement).toBe(handle);
    await shortcut(handle);
    expect(isHidden()).toBe(true);
    expect(document.activeElement).toBe(screen.getByRole("listbox"));
  });

  it("the_sidebar_shortcut_is_ignored_while_a_modal_is_open", async () => {
    const user = userEvent.setup();
    await shell();
    await user.click(screen.getByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Group" }));
    const dialog = await screen.findByRole("dialog", { name: "New Group" });
    const active = document.activeElement as HTMLElement;
    expect(dialog.contains(active)).toBe(true);
    expect(await shortcut(active)).toBe(true);
    expect(isHidden()).toBe(false);
    expect(document.activeElement).toBe(active);
  });

  it.each(["input", "textarea", "div"])("the_sidebar_shortcut_is_ignored_in_an_editable_%s", async (tag) => {
    await shell();
    const editable = document.createElement(tag);
    if (tag === "div") {
      editable.setAttribute("contenteditable", "true");
      editable.tabIndex = 0;
      // jsdom does not implement isContentEditable.
      Object.defineProperty(editable, "isContentEditable", { value: true });
    }
    sidebarPane().append(editable);
    editable.focus();
    expect(document.activeElement).toBe(editable);
    expect(await shortcut(editable)).toBe(true);
    expect(isHidden()).toBe(false);
    expect(document.activeElement).toBe(editable);
  });

  it("dragging_the_sidebar_separator_below_the_snap_width_collapses_it", async () => {
    await shell();
    await fireEvent.pointerDown(sidebarHandle(), { clientX: 220, pointerId: 1 });
    // Exactly the 120 px snap width is still a resize (clamped to the 180 px minimum)...
    await fireEvent.pointerMove(window, { clientX: 120, pointerId: 1 });
    expect(isHidden()).toBe(false);
    expect(now(sidebarHandle())).toBe("180");
    // ...below it the pane collapses, keeping the width it had before the drag.
    await fireEvent.pointerMove(window, { clientX: 110, pointerId: 1 });
    expect(isHidden()).toBe(true);
    expect(localStorage.getItem("layout.sidebarCollapsed")).toBe("true");
    // Dragging back out expands it again during the same drag.
    await fireEvent.pointerMove(window, { clientX: 230, pointerId: 1 });
    expect(isHidden()).toBe(false);
    await fireEvent.pointerUp(window, { pointerId: 1 });
    expect(now(sidebarHandle())).toBe("230");
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
