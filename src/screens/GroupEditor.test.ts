import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import EmojiPicker from "../lib/ui/EmojiPicker.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { PERSONAL_GROUP_ID, WORK_GROUP_ID } from "../test/fixtures";
import { apiError } from "../test/fakeBackend";
import { calls, renderMainApp } from "../test/mainApp";

async function openEditor(name: RegExp) {
  const user = userEvent.setup();
  const app = await renderMainApp({ session: "unlocked" });
  const row = await screen.findByRole("treeitem", { name });
  await fireEvent.contextMenu(row);
  await user.click(await screen.findByRole("menuitem", { name: "Edit group…" }));
  const dialog = await screen.findByRole("dialog", { name: "Edit group…" });
  return { user, dialog, ...app };
}

describe("group editor", () => {
  it("group_editor_saves_name_and_emoji", async () => {
    const { user, dialog, fake } = await openEditor(/^Personal/);
    const name = within(dialog).getByLabelText("Group name") as HTMLInputElement;
    expect(name.value).toBe("Personal");
    await user.clear(name);
    await user.type(name, "Work");
    await user.click(within(dialog).getByRole("button", { name: "\u{1F4BC}" }));
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "groupUpdate")).toHaveLength(1));
    expect(calls(fake, "groupUpdate")[0]?.args).toEqual({ id: PERSONAL_GROUP_ID, name: "Work", emoji: "\u{1F4BC}" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("the_editor_starts_with_the_current_name_and_emoji", async () => {
    const { dialog } = await openEditor(/Work/);
    expect((within(dialog).getByLabelText("Group name") as HTMLInputElement).value).toBe("Work");
    expect((within(dialog).getByLabelText("Icon") as HTMLInputElement).value).toBe("\u{1F4BC}");
    expect(within(dialog).getByRole("button", { name: "\u{1F4BC}" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("remove_icon_clears_the_emoji", async () => {
    const { user, dialog, fake } = await openEditor(/Work/);
    await user.click(within(dialog).getByRole("button", { name: "Remove icon" }));
    expect((within(dialog).getByLabelText("Icon") as HTMLInputElement).value).toBe("");
    expect((within(dialog).getByRole("button", { name: "Remove icon" }) as HTMLButtonElement).disabled).toBe(true);
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "groupUpdate")).toHaveLength(1));
    expect(calls(fake, "groupUpdate")[0]?.args).toEqual({ id: WORK_GROUP_ID, name: "Work", emoji: null });
  });

  it("an_empty_name_cannot_be_saved", async () => {
    const { user, dialog, fake } = await openEditor(/^Personal/);
    await user.clear(within(dialog).getByLabelText("Group name"));
    expect((within(dialog).getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
    await user.type(within(dialog).getByLabelText("Group name"), "   {Enter}");
    expect(calls(fake, "groupUpdate")).toHaveLength(0);
  });

  it("enter_in_the_name_field_saves", async () => {
    const { user, dialog, fake } = await openEditor(/^Personal/);
    await user.type(within(dialog).getByLabelText("Group name"), "s{Enter}");
    await waitFor(() => expect(calls(fake, "groupUpdate")[0]?.args.name).toBe("Personals"));
  });

  it("enter_during_ime_composition_does_not_save", async () => {
    const { dialog, fake } = await openEditor(/^Personal/);
    await fireEvent.keyDown(within(dialog).getByLabelText("Group name"), { key: "Enter", isComposing: true });
    expect(calls(fake, "groupUpdate")).toHaveLength(0);
  });

  it("cancel_closes_without_a_call", async () => {
    const { user, dialog, fake } = await openEditor(/^Personal/);
    await user.type(within(dialog).getByLabelText("Group name"), "x");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls(fake, "groupUpdate")).toHaveLength(0);
  });

  it("a_backend_error_is_shown_in_the_dialog_and_it_stays_open", async () => {
    const { user, dialog, fake } = await openEditor(/^Personal/);
    fake.handlers.groupUpdate = () => {
      throw apiError("invalidInput");
    };
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect((await within(dialog).findByRole("alert")).textContent).toBe("Invalid input");
    expect(screen.getByRole("dialog", { name: "Edit group…" })).toBeTruthy();
  });

  it("group_editor_passes_axe", async () => {
    const { dialog } = await openEditor(/^Personal/);
    await expectNoAxeViolations(dialog);
  });
});

describe("emoji picker", () => {
  it("shows_a_curated_grid_of_48", () => {
    render(EmojiPicker, { props: { value: null, onchange: vi.fn() } });
    const grid = screen.getByRole("group", { name: "Emoji" });
    expect(within(grid).getAllByRole("button")).toHaveLength(48);
  });

  it("emoji_field_limits_to_one_grapheme", async () => {
    const user = userEvent.setup();
    const onchange = vi.fn();
    render(EmojiPicker, { props: { value: null, onchange } });
    const field = screen.getByLabelText("Icon") as HTMLInputElement;
    await user.type(field, "ab");
    expect(field.value).toBe("a");
    expect(onchange).toHaveBeenLastCalledWith("a");
    // A family emoji is one grapheme made of several code points; a flag is two regional indicators.
    const family = "\u{1F468}‍\u{1F469}‍\u{1F467}";
    await user.clear(field);
    await user.click(field);
    await user.paste(family);
    expect(field.value).toBe(family);
    expect(onchange).toHaveBeenLastCalledWith(family);
    const flag = "\u{1F1FA}\u{1F1E6}";
    await user.clear(field);
    await user.paste(`${flag}${flag}`);
    expect(field.value).toBe(flag);
  });

  it("clearing_the_field_reports_no_icon", async () => {
    const user = userEvent.setup();
    const onchange = vi.fn();
    render(EmojiPicker, { props: { value: "\u{1F3E0}", onchange } });
    await user.clear(screen.getByLabelText("Icon"));
    expect(onchange).toHaveBeenLastCalledWith(null);
  });
});
