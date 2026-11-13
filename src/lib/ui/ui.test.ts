import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectNoAxeViolations } from "../../test/axe";
import { createFakeBackend } from "../../test/fakeBackend";
import KitHarness from "../../test/KitHarness.svelte";
import MenuHarness from "../../test/MenuHarness.svelte";
import ModalHarness from "../../test/ModalHarness.svelte";
import { textSnippet } from "../../test/snippets";
import { renderWithBackend } from "../../test/render";
import { mockWindows } from "@tauri-apps/api/mocks";
import { contrast } from "../../test/contrast";
import { AVATAR_PALETTE, avatarColor, avatarLetter } from "./avatar";
import Avatar from "./Avatar.svelte";
import Button from "./Button.svelte";
import ContextMenu from "./ContextMenu.svelte";
import FieldRow from "./FieldRow.svelte";
import Icon from "./Icon.svelte";
import { ICON_NAMES } from "./icons";
import Segmented from "./Segmented.svelte";
import Select from "./Select.svelte";
import SplitPane from "./SplitPane.svelte";
import Stepper from "./Stepper.svelte";
import Toggle from "./Toggle.svelte";
import WindowControls from "./WindowControls.svelte";

describe("Modal", () => {
  it("modal_closes_on_escape_and_restores_focus", async () => {
    const user = userEvent.setup();
    const onclose = vi.fn();
    render(ModalHarness, { props: { onclose } });
    const opener = screen.getByRole("button", { name: "opener" });
    await user.click(opener);
    const dialog = await screen.findByRole("dialog", { name: "Dialog title" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    await user.keyboard("{Escape}");
    expect(onclose).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(opener);
  });

  it("modal_traps_tab_inside_dialog", async () => {
    const user = userEvent.setup();
    render(ModalHarness);
    await user.click(screen.getByRole("button", { name: "opener" }));
    const dialog = await screen.findByRole("dialog");
    const first = screen.getByRole("button", { name: "first" });
    const last = screen.getByRole("button", { name: "last" });
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    first.focus();
    await user.tab({ shift: true });
    expect(document.activeElement).toBe(last);
    await user.tab();
    expect(document.activeElement).toBe(first);
    for (let i = 0; i < 6; i++) {
      await user.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
  });
});

describe("Toggle", () => {
  it("toggle_has_switch_role_and_toggles_with_space", async () => {
    const user = userEvent.setup();
    const onchange = vi.fn();
    render(Toggle, { props: { checked: false, label: "Launch at login", onchange } });
    const sw = screen.getByRole("switch", { name: "Launch at login" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    sw.focus();
    await user.keyboard(" ");
    expect(onchange).toHaveBeenCalledWith(true);
  });
  it("disabled_toggle_does_not_fire", async () => {
    const user = userEvent.setup();
    const onchange = vi.fn();
    render(Toggle, { props: { checked: true, label: "X", disabled: true, onchange } });
    await user.click(screen.getByRole("switch", { name: "X" }));
    expect(onchange).not.toHaveBeenCalled();
  });
});

describe("Button and IconButton", () => {
  it("button_fires_onclick_and_respects_disabled", async () => {
    const user = userEvent.setup();
    const onclick = vi.fn();
    const { unmount } = render(Button, { props: { onclick, children: textSnippet("Go") } });
    await user.click(screen.getByRole("button", { name: "Go" }));
    expect(onclick).toHaveBeenCalledTimes(1);
    unmount();
    const again = vi.fn();
    render(Button, { props: { onclick: again, disabled: true, children: textSnippet("Go") } });
    await user.click(screen.getByRole("button", { name: "Go" }));
    expect(again).not.toHaveBeenCalled();
  });
});

describe("Select", () => {
  it("select_calls_onchange_with_typed_value", async () => {
    const user = userEvent.setup();
    const onchange = vi.fn();
    render(Select<number>, {
      props: {
        value: 1,
        label: "Interval",
        options: [
          { value: 1, label: "One" },
          { value: 5, label: "Five" },
        ],
        onchange,
      },
    });
    await user.selectOptions(screen.getByRole("combobox", { name: "Interval" }), "Five");
    expect(onchange).toHaveBeenCalledWith(5);
  });
});

describe("Segmented", () => {
  it("segmented_arrow_keys_move_selection", async () => {
    const user = userEvent.setup();
    const onchange = vi.fn();
    render(Segmented<string>, {
      props: {
        value: "a",
        label: "Theme",
        options: [
          { value: "a", label: "Auto" },
          { value: "b", label: "Light" },
          { value: "c", label: "Dark" },
        ],
        onchange,
      },
    });
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeTruthy();
    const auto = screen.getByRole("radio", { name: "Auto" });
    expect(auto.getAttribute("aria-checked")).toBe("true");
    auto.focus();
    await user.keyboard("{ArrowRight}");
    expect(onchange).toHaveBeenLastCalledWith("b");
    await user.keyboard("{ArrowLeft}");
    expect(onchange).toHaveBeenLastCalledWith("a"); // focus moved to Light, so Left is Auto
    auto.focus();
    await user.keyboard("{ArrowLeft}");
    expect(onchange).toHaveBeenLastCalledWith("c"); // wraps
  });
});

describe("Menu", () => {
  it("menu_arrow_keys_home_end_and_enter_select", async () => {
    const user = userEvent.setup();
    const onselect = vi.fn();
    render(MenuHarness, { props: { onselect } });
    await user.click(screen.getByRole("button", { name: "Actions" }));
    const menu = await screen.findByRole("menu");
    expect(menu).toBeTruthy();
    const items = screen.getAllByRole("menuitem");
    await waitFor(() => expect(document.activeElement).toBe(items[0]));
    await user.keyboard("{ArrowDown}"); // skips the disabled item
    expect(document.activeElement).toBe(items[2]);
    await user.keyboard("{End}");
    expect(document.activeElement).toBe(items[3]);
    await user.keyboard("{Home}");
    expect(document.activeElement).toBe(items[0]);
    await user.keyboard("{ArrowUp}"); // wraps to the last
    expect(document.activeElement).toBe(items[3]);
    await user.keyboard("{Enter}");
    expect(onselect).toHaveBeenCalledWith("rename");
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("menu_typeahead_focuses_matching_item", async () => {
    const user = userEvent.setup();
    render(MenuHarness, { props: { onselect: vi.fn() } });
    await user.click(screen.getByRole("button", { name: "Actions" }));
    await screen.findByRole("menu");
    await user.keyboard("r");
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Rename" }));
  });

  it("menu_escape_closes_and_returns_focus_to_trigger", async () => {
    const user = userEvent.setup();
    render(MenuHarness, { props: { onselect: vi.fn() } });
    const trigger = screen.getByRole("button", { name: "Actions" });
    await user.click(trigger);
    await screen.findByRole("menu");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    expect(document.activeElement).toBe(trigger);
  });

  it("disabled_menu_item_is_not_selected", async () => {
    const user = userEvent.setup();
    const onselect = vi.fn();
    render(MenuHarness, { props: { onselect } });
    await user.click(screen.getByRole("button", { name: "Actions" }));
    await user.click(await screen.findByRole("menuitem", { name: "Cut" }));
    expect(onselect).not.toHaveBeenCalled();
  });

  it("context_menu_selects_and_closes_on_escape", async () => {
    const user = userEvent.setup();
    const onselect = vi.fn();
    const onclose = vi.fn();
    render(ContextMenu, {
      props: { items: [{ id: "a", label: "Alpha" }, { id: "b", label: "Beta" }], onselect, onclose, x: 10, y: 20 },
    });
    expect(screen.getByRole("menu")).toBeTruthy();
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onselect).toHaveBeenCalledWith("b");
    await user.keyboard("{Escape}");
    expect(onclose).toHaveBeenCalled();
  });
});

describe("Avatar", () => {
  it("avatar_letter_and_hash_colour_stable", () => {
    expect(avatarLetter("GitHub")).toBe("G");
    expect(avatarLetter("")).toBe("#");
    expect(avatarLetter("42 Club")).toBe("#");
    expect(avatarLetter("\u{43f}\u{430}\u{440}\u{43e}\u{43b}\u{44c}")).toBe("\u{41f}");
    expect(avatarColor("GitHub")).toBe(avatarColor("github"));
    const colours = new Set(["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"].map(avatarColor));
    expect(colours.size).toBeGreaterThan(3);
    const a = render(Avatar, { props: { text: "GitHub" } });
    const first = a.container.querySelector("span")?.getAttribute("style");
    expect(a.container.textContent).toBe("G");
    a.unmount();
    const b = render(Avatar, { props: { text: "github" } });
    expect(b.container.querySelector("span")?.getAttribute("style")).toBe(first);
  });
});

describe("Stepper", () => {
  const props = (over: Record<string, unknown> = {}) => ({
    value: 30, min: 10, max: 120, step: 5, label: "Clear after", unit: "s", onchange: vi.fn(), ...over,
  });

  it("stepper_has_spinbutton_role_and_accessible_name", () => {
    render(Stepper, { props: props() });
    const sb = screen.getByRole("spinbutton", { name: "Clear after" });
    expect(sb.getAttribute("aria-valuemin")).toBe("10");
    expect(sb.getAttribute("aria-valuemax")).toBe("120");
    expect(sb.getAttribute("aria-valuenow")).toBe("30");
  });

  it("stepper_clamps_and_snaps_to_step", async () => {
    const user = userEvent.setup();
    for (const [typed, expected] of [["33", 35], ["7", 10], ["500", 120]] as const) {
      const p = props();
      const { unmount } = render(Stepper, { props: p });
      const sb = screen.getByRole("spinbutton", { name: "Clear after" });
      await user.clear(sb);
      await user.type(sb, `${typed}{Enter}`);
      expect(p.onchange).toHaveBeenLastCalledWith(expected);
      expect((sb as HTMLInputElement).value).toBe(String(expected));
      unmount();
    }
  });

  it("stepper_keys_and_buttons", async () => {
    const user = userEvent.setup();
    const top = props({ value: 120 });
    const a = render(Stepper, { props: top });
    screen.getByRole("spinbutton").focus();
    await user.keyboard("{ArrowUp}");
    expect(top.onchange).not.toHaveBeenCalled();
    await user.keyboard("{ArrowDown}");
    expect(top.onchange).toHaveBeenLastCalledWith(115);
    a.unmount();
    const mid = props({ value: 50 });
    render(Stepper, { props: mid });
    screen.getByRole("spinbutton").focus();
    await user.keyboard("{PageUp}");
    expect(mid.onchange).toHaveBeenLastCalledWith(75);
    await user.keyboard("{PageDown}");
    expect(mid.onchange).toHaveBeenLastCalledWith(25);
    await user.click(screen.getByRole("button", { name: "Increase" }));
    expect(mid.onchange).toHaveBeenLastCalledWith(55);
    await user.click(screen.getByRole("button", { name: "Decrease" }));
    expect(mid.onchange).toHaveBeenLastCalledWith(45);
  });
});

describe("FieldRow", () => {
  it("field_row_renders_label_and_actions", () => {
    render(FieldRow, { props: { label: "Username", children: textSnippet("alice"), actions: textSnippet("ACT") } });
    expect(screen.getByText("Username")).toBeTruthy();
    expect(screen.getByText("alice")).toBeTruthy();
    expect(screen.getByText("ACT")).toBeTruthy();
  });
});

describe("WindowControls", () => {
  it("window_controls_macos_has_only_inset_spacer", () => {
    render(WindowControls, { props: { os: "macos" } });
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    const inset = screen.getByTestId("titlebar-inset");
    expect(inset.style.width).toBe("72px");
  });

  it("window_controls_windows_close_button_labelled", async () => {
    const user = userEvent.setup();
    mockWindows("main");
    const calls: string[] = [];
    let maximized = false;
    const fake = createFakeBackend({
      plugins: {
        "plugin:window|close": () => void calls.push("close"),
        "plugin:window|minimize": () => void calls.push("minimize"),
        "plugin:window|maximize": () => { maximized = true; calls.push("maximize"); },
        "plugin:window|unmaximize": () => { maximized = false; calls.push("unmaximize"); },
        "plugin:window|is_maximized": () => maximized,
      },
    });
    const { container } = renderWithBackend(WindowControls, { os: "windows" }, fake);
    expect(container.querySelector("[data-tauri-drag-region]")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "Minimize" }));
    await user.click(screen.getByRole("button", { name: "Maximize" }));
    await user.click(await screen.findByRole("button", { name: "Restore" }));
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(calls).toEqual(["minimize", "maximize", "unmaximize", "close"]);
  });
});

describe("SplitPane", () => {
  it("split_pane_clamps_to_min_widths_on_drag", async () => {
    const proto = HTMLElement.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, "clientWidth");
    Object.defineProperty(proto, "clientWidth", { configurable: true, get: () => 1000 });
    try {
      const onresize = vi.fn();
      render(SplitPane, {
        props: {
          widths: [200, 300], min: [160, 240, 320], onresize,
          first: textSnippet("one"), second: textSnippet("two"), third: textSnippet("three"),
        },
      });
      const [h1, h2] = screen.getAllByRole("separator");
      if (!h1 || !h2) throw new Error("separators missing");
      // Drag the first handle far left: clamped to min[0].
      await fireEvent.pointerDown(h1, { clientX: 200, pointerId: 1 });
      await fireEvent.pointerMove(window, { clientX: -500, pointerId: 1 });
      expect(onresize).toHaveBeenLastCalledWith([160, 300]);
      await fireEvent.pointerUp(window, { pointerId: 1 });
      // Drag far right: the detail pane keeps its 320 px minimum (1000 - 300 - 320 = 380).
      await fireEvent.pointerDown(h1, { clientX: 200, pointerId: 1 });
      await fireEvent.pointerMove(window, { clientX: 5000, pointerId: 1 });
      expect(onresize).toHaveBeenLastCalledWith([380, 300]);
      await fireEvent.pointerUp(window, { pointerId: 1 });
      // Second handle: list width clamps to min[1] and to the room left.
      await fireEvent.pointerDown(h2, { clientX: 500, pointerId: 1 });
      await fireEvent.pointerMove(window, { clientX: -900, pointerId: 1 });
      expect(onresize).toHaveBeenLastCalledWith([200, 240]);
      await fireEvent.pointerUp(window, { pointerId: 1 });
      // Keyboard: ArrowLeft on the first handle shrinks by 16 px, never below the minimum.
      h1.focus();
      await fireEvent.keyDown(h1, { key: "ArrowLeft" });
      expect(onresize).toHaveBeenLastCalledWith([184, 300]);
    } finally {
      if (desc) Object.defineProperty(proto, "clientWidth", desc);
      else delete (proto as unknown as Record<string, unknown>).clientWidth;
    }
  });
});

describe("Icon", () => {
  it("icon_names_all_render_svg", () => {
    expect(ICON_NAMES.length).toBe(27);
    for (const name of ICON_NAMES) {
      const { container, unmount } = render(Icon, { props: { name } });
      const svg = container.querySelector("svg");
      expect(svg, name).not.toBeNull();
      expect(svg?.getAttribute("aria-hidden")).toBe("true");
      expect(svg?.querySelector("path"), name).not.toBeNull();
      unmount();
    }
  });
});

describe("accessibility", () => {
  it("components_pass_axe", async () => {
    mockWindows("main");
    const { container } = renderWithBackend(KitHarness);
    await expectNoAxeViolations(container.ownerDocument.body);
  });
});

describe("interactions", () => {
  it("avatar_palette_keeps_white_text_readable", () => {
    expect(AVATAR_PALETTE).toHaveLength(10);
    for (const colour of AVATAR_PALETTE) expect(contrast("#ffffff", colour), colour).toBeGreaterThanOrEqual(4.5);
  });

  it("menu_closes_on_outside_pointer_press", async () => {
    const user = userEvent.setup();
    render(MenuHarness, { props: { onselect: vi.fn() } });
    await user.click(screen.getByRole("button", { name: "Actions" }));
    await screen.findByRole("menu");
    await user.click(document.body);
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("context_menu_closes_on_outside_press_and_window_blur", async () => {
    const user = userEvent.setup();
    const onclose = vi.fn();
    render(ContextMenu, { props: { items: [{ id: "a", label: "Alpha" }], onselect: vi.fn(), onclose, x: 0, y: 0 } });
    await user.click(document.body);
    expect(onclose).toHaveBeenCalledTimes(1);
    await fireEvent.blur(window);
    expect(onclose).toHaveBeenCalledTimes(2);
  });

  it("modal_backdrop_click_closes_but_a_drag_ending_there_does_not", async () => {
    const user = userEvent.setup();
    const onclose = vi.fn();
    render(ModalHarness, { props: { onclose } });
    await user.click(screen.getByRole("button", { name: "opener" }));
    const dialog = await screen.findByRole("dialog");
    const backdrop = dialog.parentElement as HTMLElement;
    await fireEvent.pointerDown(dialog);
    await fireEvent.click(backdrop);
    expect(onclose).not.toHaveBeenCalled();
    await user.click(backdrop);
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("stepper_home_and_end_jump_to_the_limits", async () => {
    const user = userEvent.setup();
    const onchange = vi.fn();
    render(Stepper, { props: { value: 50, min: 10, max: 120, step: 5, label: "Seconds", onchange } });
    screen.getByRole("spinbutton").focus();
    await user.keyboard("{End}");
    expect(onchange).toHaveBeenLastCalledWith(120);
    await user.keyboard("{Home}");
    expect(onchange).toHaveBeenLastCalledWith(10);
  });

  it("split_pane_arrow_right_and_second_handle_keyboard", async () => {
    const proto = HTMLElement.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, "clientWidth");
    Object.defineProperty(proto, "clientWidth", { configurable: true, get: () => 1000 });
    try {
      const onresize = vi.fn();
      render(SplitPane, {
        props: {
          widths: [200, 300], min: [160, 240, 320], onresize,
          first: textSnippet("one"), second: textSnippet("two"), third: textSnippet("three"),
        },
      });
      const [h1, h2] = screen.getAllByRole("separator");
      if (!h1 || !h2) throw new Error("separators missing");
      await fireEvent.keyDown(h1, { key: "ArrowRight" });
      expect(onresize).toHaveBeenLastCalledWith([216, 300]);
      await fireEvent.keyDown(h2, { key: "ArrowRight" });
      expect(onresize).toHaveBeenLastCalledWith([200, 316]);
      await fireEvent.keyDown(h2, { key: "ArrowLeft" });
      expect(onresize).toHaveBeenLastCalledWith([200, 284]);
    } finally {
      if (desc) Object.defineProperty(proto, "clientWidth", desc);
      else delete (proto as unknown as Record<string, unknown>).clientWidth;
    }
  });

  it("window_controls_label_follows_window_resize", async () => {
    mockWindows("main");
    let maximized = false;
    const fake = createFakeBackend({ plugins: { "plugin:window|is_maximized": () => maximized } });
    renderWithBackend(WindowControls, { os: "windows" }, fake);
    expect(await screen.findByRole("button", { name: "Maximize" })).toBeTruthy();
    maximized = true; // e.g. snapped by the OS
    await fireEvent(window, new Event("resize"));
    expect(await screen.findByRole("button", { name: "Restore" })).toBeTruthy();
  });

  it("button_keeps_variant_style_when_the_caller_passes_a_class", () => {
    render(Button, { props: { variant: "primary", class: "extra", children: textSnippet("Go") } });
    const cls = screen.getByRole("button", { name: "Go" }).className;
    expect(cls).toContain("primary");
    expect(cls).toContain("extra");
  });
});
