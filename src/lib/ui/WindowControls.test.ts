import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { expect, it, vi } from "vitest";
import controlsSource from "./WindowControls.svelte?raw";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
const css = readFileSync(resolve("src/app.css"), "utf8");
import shellSource from "../../screens/Shell.svelte?raw";
import WindowControls from "./WindowControls.svelte";

const win = vi.hoisted(() => ({
  isMaximized: vi.fn(async () => false),
  maximize: vi.fn(async () => { win.isMaximized.mockResolvedValue(true); }),
  unmaximize: vi.fn(async () => { win.isMaximized.mockResolvedValue(false); }),
  minimize: vi.fn(),
  close: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }));

it("window_controls_maximize_toggles_to_restore_icon", async () => {
  render(WindowControls, { os: "windows" });
  await fireEvent.click(screen.getByRole("button", { name: "Maximize" }));
  const restore = await screen.findByRole("button", { name: "Restore" });
  expect(restore.querySelector("path")?.getAttribute("d")).toContain("M2.5");
  await fireEvent.click(restore);
  await waitFor(() => expect(screen.getByRole("button", { name: "Maximize" })).toBeTruthy());
  expect(win.maximize).toHaveBeenCalledTimes(1);
  expect(win.unmaximize).toHaveBeenCalledTimes(1);
});

it("window_controls_windows_close_hover_and_sizes", () => {
  // jsdom does not lay out components; native geometry remains a desktop check.
  expect(controlsSource).toMatch(/width: 46px/);
  expect(controlsSource).toMatch(/\.windows button\s*\{\s*height: 32px/);
  expect(shellSource).toMatch(/\.titlebar.windows\s*\{\s*height: 32px/);
  expect(controlsSource).toMatch(/\.close:hover\s*\{\s*background: var\(--danger-hover\)/);
  expect(css).toContain("--danger-hover: #e81123");
});

it("titlebar_has_drag_region_and_no_drag_on_buttons", () => {
  render(WindowControls, { os: "windows" });
  for (const button of screen.getAllByRole("button")) {
    expect(button.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(button.parentElement?.hasAttribute("data-tauri-drag-region")).toBe(true);
  }
});

it("font_stack_lists_segoe_ui_variable_before_segoe_ui", () => {
  expect(css).toMatch(/--font:.*"Segoe UI Variable Text", "Segoe UI"/);
});
