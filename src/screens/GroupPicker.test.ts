import { render, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { entries } from "../lib/stores/entries.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { BANKING_GROUP_ID, PERSONAL_GROUP_ID, ROOT_GROUP_ID, WORK_GROUP_ID } from "../test/fixtures";
import { createFakeBackend } from "../test/fakeBackend";
import GroupPickerHarness from "../test/GroupPickerHarness.svelte";

afterEach(() => entries.stop());

async function openPicker(props: Partial<{ current: string | null; exclude: string | null; includeRoot: boolean }> = {}) {
  const fake = createFakeBackend({ session: "unlocked" });
  fake.install();
  await entries.refresh();
  const onpick = vi.fn();
  const onclose = vi.fn();
  render(GroupPickerHarness, { props: { onpick, onclose, ...props } });
  const dialog = await screen.findByRole("dialog", { name: "Restore to" });
  return { dialog, onpick, onclose, user: userEvent.setup() };
}

describe("group picker", () => {
  it("lists_the_top_level_and_every_group_but_not_the_recycle_bin", async () => {
    const { dialog } = await openPicker();
    const names = within(dialog).getAllByRole("treeitem").map((el) => (el.textContent ?? "").trim());
    expect(names[0]).toBe("Vault");
    expect(names).toHaveLength(4);
    expect(within(dialog).queryByRole("treeitem", { name: /Bin/ })).toBeNull();
    expect(within(dialog).getByRole("treeitem", { name: /Banking/ }).getAttribute("aria-level")).toBe("3");
  });

  it("activating_a_group_picks_it", async () => {
    const { dialog, onpick, user } = await openPicker();
    await user.click(within(dialog).getByRole("treeitem", { name: /Work/ }));
    expect(onpick).toHaveBeenCalledWith(WORK_GROUP_ID);
  });

  it("the_top_level_row_picks_the_root_group", async () => {
    const { dialog, onpick, user } = await openPicker();
    await user.click(within(dialog).getByRole("treeitem", { name: "Vault" }));
    expect(onpick).toHaveBeenCalledWith(ROOT_GROUP_ID);
    expect(within(dialog).queryByRole("treeitem", { name: "Vault" })).not.toBeNull();
  });

  it("the_current_group_and_an_excluded_subtree_cannot_be_picked", async () => {
    const { dialog, onpick, user } = await openPicker({ current: WORK_GROUP_ID, exclude: PERSONAL_GROUP_ID });
    for (const name of [/Work/, /^.*Personal/, /Banking/]) {
      const item = within(dialog).getByRole("treeitem", { name });
      expect(item.getAttribute("aria-disabled")).toBe("true");
      await user.click(item);
    }
    expect(onpick).not.toHaveBeenCalled();
  });

  it("the_root_can_be_left_out", async () => {
    const { dialog } = await openPicker({ includeRoot: false });
    expect(within(dialog).queryByRole("treeitem", { name: "Vault" })).toBeNull();
    expect(within(dialog).getByRole("treeitem", { name: /^Personal/ }).getAttribute("aria-level")).toBe("1");
  });

  it("arrow_keys_move_between_rows_and_enter_picks", async () => {
    const { dialog, onpick, user } = await openPicker();
    within(dialog).getByRole("treeitem", { name: "Vault" }).focus();
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(onpick).toHaveBeenCalledWith(BANKING_GROUP_ID);
  });

  it("cancel_closes_without_picking", async () => {
    const { dialog, onpick, onclose, user } = await openPicker();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(onclose).toHaveBeenCalled();
    expect(onpick).not.toHaveBeenCalled();
    await waitFor(() => expect(onclose).toHaveBeenCalledTimes(1));
  });

  it("group_picker_passes_axe", async () => {
    const { dialog } = await openPicker();
    await expectNoAxeViolations(dialog);
  });
});
