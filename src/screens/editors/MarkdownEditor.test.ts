import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { session } from "../../lib/stores/session.svelte";
import MarkdownEditorHarness from "../../test/MarkdownEditorHarness.svelte";

afterEach(() => vi.useRealTimers());

const area = () => screen.getByRole("textbox", { name: "Body" }) as HTMLTextAreaElement;
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;

/** Types like a user would: sets the value and fires `input`, caret at the end. */
async function type(text: string) {
  const el = area();
  el.value = text;
  el.setSelectionRange(text.length, text.length);
  await fireEvent.input(el);
}

function select(start: number, end: number) {
  area().focus();
  area().setSelectionRange(start, end);
}

describe("toolbar", () => {
  it("markdown_toolbar_wraps_selection_with_markers", async () => {
    render(MarkdownEditorHarness, { initial: "abc" });
    select(0, 3);
    await userEvent.setup().click(button("Bold"));
    expect(area().value).toBe("**abc**");
    expect([area().selectionStart, area().selectionEnd]).toEqual([2, 5]);
  });

  it("italic_code_list_heading_and_link_edit_the_text", async () => {
    render(MarkdownEditorHarness, { initial: "abc" });
    const u = userEvent.setup();
    select(0, 3);
    await u.click(button("Italic"));
    expect(area().value).toBe("*abc*");
    select(0, 5);
    await u.click(button("Code"));
    expect(area().value).toBe("`*abc*`");
    select(0, 7);
    await u.click(button("Heading"));
    expect(area().value).toBe("# `*abc*`");
    select(0, 9);
    await u.click(button("List"));
    expect(area().value).toBe("- # `*abc*`");
  });

  it("link_wraps_the_selection_and_selects_the_url_part", async () => {
    render(MarkdownEditorHarness, { initial: "abc" });
    select(0, 3);
    await userEvent.setup().click(button("Link"));
    expect(area().value).toBe("[abc](https://)");
    expect(area().value.slice(area().selectionStart, area().selectionEnd)).toBe("https://");
  });

  it("bold_with_no_selection_puts_the_caret_between_the_markers", async () => {
    render(MarkdownEditorHarness, { initial: "" });
    select(0, 0);
    await userEvent.setup().click(button("Bold"));
    expect(area().value).toBe("****");
    expect([area().selectionStart, area().selectionEnd]).toEqual([2, 2]);
  });
});

describe("undo", () => {
  it("notes_undo_redo_with_mod_z_and_mod_shift_z", async () => {
    render(MarkdownEditorHarness, { initial: "" });
    vi.useFakeTimers();
    await type("a");
    vi.advanceTimersByTime(600);
    await type("ab");
    await fireEvent.keyDown(area(), { key: "z", metaKey: true });
    expect(area().value).toBe("a");
    await fireEvent.keyDown(area(), { key: "z", metaKey: true });
    expect(area().value).toBe("");
    await fireEvent.keyDown(area(), { key: "z", metaKey: true, shiftKey: true });
    expect(area().value).toBe("a");
    await fireEvent.keyDown(area(), { key: "y", ctrlKey: true });
    expect(area().value).toBe("ab");
  });

  it("typing_within_half_a_second_is_one_undo_step", async () => {
    render(MarkdownEditorHarness, { initial: "" });
    vi.useFakeTimers();
    await type("a");
    vi.advanceTimersByTime(100);
    await type("ab");
    await fireEvent.keyDown(area(), { key: "z", metaKey: true });
    expect(area().value).toBe("");
  });

  it("toolbar_edit_is_undone_in_one_step", async () => {
    render(MarkdownEditorHarness, { initial: "abc" });
    select(0, 3);
    await userEvent.setup().click(button("Bold"));
    await fireEvent.keyDown(area(), { key: "z", metaKey: true });
    expect(area().value).toBe("abc");
    expect([area().selectionStart, area().selectionEnd]).toEqual([0, 3]);
  });

  it("undo_and_redo_buttons_disabled_when_stack_empty", async () => {
    render(MarkdownEditorHarness, { initial: "abc" });
    expect(button("Undo").disabled).toBe(true);
    expect(button("Redo").disabled).toBe(true);
    select(0, 3);
    const u = userEvent.setup();
    await u.click(button("Bold"));
    expect(button("Undo").disabled).toBe(false);
    await u.click(button("Undo"));
    expect(area().value).toBe("abc");
    expect(button("Undo").disabled).toBe(true);
    expect(button("Redo").disabled).toBe(false);
    await u.click(button("Redo"));
    expect(area().value).toBe("**abc**");
    expect(button("Redo").disabled).toBe(true);
  });

  it("undo_stack_is_cleared_on_close_and_on_a_lock_epoch_change", async () => {
    const first = render(MarkdownEditorHarness, { initial: "earlier note text" });
    select(0, 3);
    await userEvent.setup().click(button("Bold"));
    expect(button("Undo").disabled).toBe(false);
    // A lock while mounted forgets the history.
    session.lockEpoch += 1;
    await waitFor(() => expect(button("Undo").disabled).toBe(true));
    await fireEvent.keyDown(area(), { key: "z", metaKey: true });
    expect(area().value).toBe("**ear**lier note text");
    // A fresh editor starts without any of the earlier text.
    first.unmount();
    render(MarkdownEditorHarness, { initial: "fresh" });
    expect(button("Undo").disabled).toBe(true);
    await fireEvent.keyDown(area(), { key: "z", metaKey: true });
    expect(area().value).toBe("fresh");
    expect(document.body.textContent).not.toContain("earlier note text");
  });
});

describe("native history", () => {
  it("edit_menu_undo_and_redo_are_routed_to_the_stack", async () => {
    render(MarkdownEditorHarness, { initial: "abc" });
    select(0, 3);
    await userEvent.setup().click(button("Bold"));
    const undo = new InputEvent("beforeinput", { inputType: "historyUndo", cancelable: true, bubbles: true });
    area().dispatchEvent(undo);
    expect(undo.defaultPrevented).toBe(true);
    await waitFor(() => expect(area().value).toBe("abc"));
    const redo = new InputEvent("beforeinput", { inputType: "historyRedo", cancelable: true, bubbles: true });
    area().dispatchEvent(redo);
    expect(redo.defaultPrevented).toBe(true);
    await waitFor(() => expect(area().value).toBe("**abc**"));
  });
});

describe("preview", () => {
  it("preview_toggle_renders_sanitised_html", async () => {
    render(MarkdownEditorHarness, { initial: "**bold** <img src=x onerror=alert(1)><script>alert(2)</script>" });
    const toggle = button("Preview");
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    await userEvent.setup().click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    const strong = document.querySelector("strong");
    expect(strong?.textContent).toBe("bold");
    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector("img[onerror]")).toBeNull();
    await userEvent.setup().click(toggle);
    expect(area().hidden).toBe(false);
  });
});
