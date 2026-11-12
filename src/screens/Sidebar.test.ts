import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ENTRY_MIME } from "../lib/dnd";
import { entries } from "../lib/stores/entries.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { dragAndDrop, dropPayload } from "../test/dnd";
import {
  BANKING_GROUP_ID,
  PERSONAL_GROUP_ID,
  ROOT_GROUP_ID,
  WORK_GROUP_ID,
  fixtureSidebar,
} from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";

const CATEGORY_NAMES = ["All", "Favorites", "Codes", "Passkeys", "Addresses", "Cards", "Notes", "Files", "Security", "Deleted"];

async function openVault() {
  const app = await renderMainApp({ session: "unlocked" });
  await screen.findByRole("treeitem", { name: /^Personal/ });
  return app;
}

const row = (name: RegExp | string) => screen.getByRole("treeitem", { name });
const nameOf = (el: HTMLElement) => (el.textContent ?? "").trim().replace(/\s+(\d+|—)$/, "");

describe("categories", () => {
  it("sidebar_lists_ten_categories_in_spec_order", async () => {
    await openVault();
    const group = screen.getByRole("group", { name: "Categories" });
    expect(within(group).getAllByRole("treeitem").map(nameOf)).toEqual(CATEGORY_NAMES);
    expect(screen.getByRole("tree")).toBeTruthy();
  });

  it("counts_rendered_and_security_shows_em_dash", async () => {
    const { fake } = await openVault();
    const expected = fixtureSidebar(fake.state.entries).categories;
    const group = screen.getByRole("group", { name: "Categories" });
    const items = within(group).getAllByRole("treeitem");
    expected.forEach((c, i) => {
      const shown = (items[i]?.textContent ?? "").trim().split(/\s+/).pop();
      expect(shown).toBe(c.count === null ? "—" : String(c.count));
    });
    expect(expected.find((c) => c.category === "security")?.count).toBeNull();
  });
});

describe("group tree", () => {
  it("group_tree_shows_emoji_nested_groups_and_counts", async () => {
    const { fake } = await openVault();
    const groups = fixtureSidebar(fake.state.entries).groups;
    const personal = groups.find((g) => g.id === PERSONAL_GROUP_ID);
    const work = groups.find((g) => g.id === WORK_GROUP_ID);
    expect(row(/^Personal/).getAttribute("aria-level")).toBe("1");
    expect(row(/^Personal/).getAttribute("aria-expanded")).toBe("true");
    expect(row(/^Personal/).textContent).toContain(String(personal?.count));
    const banking = row(/Banking/);
    expect(banking.getAttribute("aria-level")).toBe("2");
    expect(banking.textContent).toContain("\u{1F3E6}");
    expect(row(/Work/).textContent).toContain("\u{1F4BC}");
    expect(row(/Work/).textContent).toContain(String(work?.count));
    // A group without an emoji shows the folder icon, never the word "null".
    expect(row(/^Personal/).textContent).not.toContain("null");
  });

  it("groups_collapse_and_expand_from_the_keyboard", async () => {
    const user = userEvent.setup();
    await openVault();
    row(/^Personal/).focus();
    await user.keyboard("{ArrowLeft}");
    expect(screen.queryByRole("treeitem", { name: /Banking/ })).toBeNull();
    expect(row(/^Personal/).getAttribute("aria-expanded")).toBe("false");
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("treeitem", { name: /Banking/ })).toBeTruthy();
  });

  it("recycle_bin_hidden_from_group_tree", async () => {
    await openVault();
    const groups = screen.getByRole("group", { name: "Groups" });
    expect(within(groups).queryByRole("treeitem", { name: /Bin/ })).toBeNull();
    const names = within(groups).getAllByRole("treeitem").map((el) => el.getAttribute("aria-level") + (el.textContent ?? ""));
    expect(names).toHaveLength(3);
    expect(names[0]).toMatch(/^1\s*Personal/);
    expect(names[1]).toMatch(/^2.*Banking/);
    expect(names[2]).toMatch(/^1.*Work/);
    expect(within(screen.getByRole("group", { name: "Categories" })).getByRole("treeitem", { name: /^Deleted/ })).toBeTruthy();
  });
});

describe("selection", () => {
  it("selecting_category_clears_group_selection_and_vice_versa", async () => {
    const user = userEvent.setup();
    await openVault();
    expect(row(/^All/).getAttribute("aria-selected")).toBe("true");
    await user.click(row(/^Personal/));
    expect(navigation.scope).toEqual({ kind: "group", groupId: PERSONAL_GROUP_ID });
    expect(row(/^Personal/).getAttribute("aria-selected")).toBe("true");
    expect(row(/^All/).getAttribute("aria-selected")).toBe("false");
    await user.click(row(/^Favorites/));
    expect(navigation.scope).toEqual({ kind: "category", category: "favorites" });
    expect(row(/^Personal/).getAttribute("aria-selected")).toBe("false");
    expect(row(/^Favorites/).getAttribute("aria-selected")).toBe("true");
  });

  it("selecting_new_scope_keeps_draft_editor_open", async () => {
    const user = userEvent.setup();
    const { fake } = await openVault();
    await user.click(screen.getByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Login" }));
    await waitFor(() => expect(navigation.presentation).toBe("create"));
    await user.click(row(/^Work/));
    expect(navigation.presentation).toBe("create");
    expect(navigation.editModel).not.toBeNull();
    expect(calls(fake, "entryNewModel")).toHaveLength(1);
  });

  it("selecting_new_scope_clears_browse_selection", async () => {
    const user = userEvent.setup();
    await openVault();
    navigation.select("e-github");
    await user.click(row(/^Work/));
    expect(navigation.selectedId).toBeNull();
  });

  it("sidebar_keyboard_arrow_navigation_and_enter_selects", async () => {
    const user = userEvent.setup();
    await openVault();
    row(/^All/).focus();
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(row(/^Favorites/));
    await user.keyboard("{End}");
    expect(document.activeElement).toBe(row(/^Work/));
    await user.keyboard("{Home}{ArrowDown}{ArrowDown}{Enter}");
    expect(navigation.scope).toEqual({ kind: "category", category: "codes" });
    // One tab stop: the focused row.
    expect(screen.getAllByRole("treeitem").filter((el) => el.tabIndex === 0)).toHaveLength(1);
  });
});

describe("plus menu", () => {
  it("plus_menu_creates_each_entry_type_with_current_group_default", async () => {
    const user = userEvent.setup();
    const { fake } = await openVault();
    await user.click(row(/^Personal/));
    const cases = [
      ["New Login", "login"],
      ["New Address", "address"],
      ["New Card", "card"],
      ["New Note", "notes"],
      ["New Files item", "files"],
    ] as const;
    for (const [label, type] of cases) {
      await user.click(screen.getByRole("button", { name: "New Entry" }));
      await user.click(await screen.findByRole("menuitem", { name: label }));
      await waitFor(() => expect(navigation.editModel?.entryType).toBe(type));
      expect(navigation.presentation).toBe("create");
      navigation.closeEditor();
    }
    expect(calls(fake, "entryNewModel").map((c) => c.args)).toEqual(
      cases.map(([, type]) => ({ entryType: type, groupId: PERSONAL_GROUP_ID })),
    );
  });

  it("plus_menu_without_a_group_selected_lets_the_backend_choose", async () => {
    const user = userEvent.setup();
    const { fake } = await openVault();
    await user.click(screen.getByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Card" }));
    await waitFor(() => expect(calls(fake, "entryNewModel")).toHaveLength(1));
    expect(calls(fake, "entryNewModel")[0]?.args).toEqual({ entryType: "card", groupId: null });
  });

  it("new_group_opens_editor_and_creates_with_parent_of_selection", async () => {
    const user = userEvent.setup();
    const { fake } = await openVault();
    await user.click(row(/^Work/));
    await user.click(screen.getByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Group" }));
    const dialog = await screen.findByRole("dialog", { name: "New Group" });
    await user.type(within(dialog).getByLabelText("Group name"), "Projects");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "groupCreate")).toHaveLength(1));
    expect(calls(fake, "groupCreate")[0]?.args).toEqual({ parentId: WORK_GROUP_ID, name: "Projects", emoji: null });
    // The vault-changed event refreshes the tree; the new group shows under Work.
    expect(await screen.findByRole("treeitem", { name: /Projects/ })).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("new_group_with_a_category_selected_goes_to_the_top_level", async () => {
    const user = userEvent.setup();
    const { fake } = await openVault();
    await user.click(screen.getByRole("button", { name: "New Entry" }));
    await user.click(await screen.findByRole("menuitem", { name: "New Group" }));
    const dialog = await screen.findByRole("dialog", { name: "New Group" });
    await user.type(within(dialog).getByLabelText("Group name"), "Top");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "groupCreate")).toHaveLength(1));
    expect(calls(fake, "groupCreate")[0]?.args).toEqual({ parentId: null, name: "Top", emoji: null });
  });
});

describe("context menu", () => {
  it("context_menu_offers_new_subgroup_edit_and_delete", async () => {
    const user = userEvent.setup();
    await openVault();
    await fireEvent.contextMenu(row(/^Work/), { clientX: 30, clientY: 40 });
    const menu = await screen.findByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((i) => i.textContent?.trim())).toEqual([
      "New subgroup",
      "Edit group…",
      "Delete group",
    ]);
    await user.click(within(menu).getByRole("menuitem", { name: "New subgroup" }));
    const dialog = await screen.findByRole("dialog", { name: "New Group" });
    expect(dialog).toBeTruthy();
  });

  it("new_subgroup_uses_the_clicked_group_as_parent", async () => {
    const user = userEvent.setup();
    const { fake } = await openVault();
    await fireEvent.contextMenu(row(/Banking/));
    await user.click(await screen.findByRole("menuitem", { name: "New subgroup" }));
    const dialog = await screen.findByRole("dialog", { name: "New Group" });
    await user.type(within(dialog).getByLabelText("Group name"), "Savings");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "groupCreate")[0]?.args).toEqual({ parentId: BANKING_GROUP_ID, name: "Savings", emoji: null }));
  });

  it("context_menu_escape_returns_focus_to_the_group_row", async () => {
    const user = userEvent.setup();
    await openVault();
    await fireEvent.contextMenu(row(/^Work/));
    await screen.findByRole("menu");
    expect(document.activeElement).not.toBe(row(/Work/));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    expect(document.activeElement).toBe(row(/Work/));
  });

  it("context_menu_opens_from_the_keyboard", async () => {
    const user = userEvent.setup();
    await openVault();
    row(/^Work/).focus();
    await user.keyboard("{Shift>}{F10}{/Shift}");
    expect(await screen.findByRole("menu")).toBeTruthy();
  });

  it("closing_the_edit_dialog_returns_focus_to_the_group_row", async () => {
    const user = userEvent.setup();
    await openVault();
    await fireEvent.contextMenu(row(/^Work/));
    await user.click(await screen.findByRole("menuitem", { name: "Edit group…" }));
    await screen.findByRole("dialog", { name: "Edit group…" });
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(row(/Work/));
  });
});

describe("delete group", () => {
  async function askToDelete() {
    const user = userEvent.setup();
    const app = await openVault();
    await user.click(row(/^Work/));
    await fireEvent.contextMenu(row(/^Work/));
    await user.click(await screen.findByRole("menuitem", { name: "Delete group" }));
    const dialog = await screen.findByRole("dialog", { name: "Delete group" });
    return { user, dialog, ...app };
  }

  it("delete_group_asks_confirmation_then_calls_group_delete", async () => {
    const { user, dialog, fake } = await askToDelete();
    expect(within(dialog).getByText("Delete this group and move its items to Deleted?")).toBeTruthy();
    expect(calls(fake, "groupDelete")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(calls(fake, "groupDelete")).toHaveLength(1));
    expect(calls(fake, "groupDelete")[0]?.args).toEqual({ id: WORK_GROUP_ID });
    await waitFor(() => expect(screen.queryByRole("treeitem", { name: /Work/ })).toBeNull());
    // The selected group is gone, so the scope falls back to All items.
    expect(navigation.scope).toEqual({ kind: "category", category: "all" });
  });

  it("cancelling_the_delete_confirmation_does_nothing", async () => {
    const { user, dialog, fake } = await askToDelete();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls(fake, "groupDelete")).toHaveLength(0);
    expect(row(/Work/)).toBeTruthy();
    expect(navigation.scope).toEqual({ kind: "group", groupId: WORK_GROUP_ID });
  });

  it("a_failed_delete_shows_a_toast_and_keeps_the_scope", async () => {
    const { user, dialog, fake } = await askToDelete();
    const { apiError } = await import("../test/fakeBackend");
    fake.handlers.groupDelete = () => {
      throw apiError("notFound");
    };
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(toasts.items.length).toBe(1));
    expect(navigation.scope).toEqual({ kind: "group", groupId: WORK_GROUP_ID });
  });
});

describe("drag and drop", () => {
  it("drag_entry_onto_group_calls_entry_move", async () => {
    const { fake } = await openVault();
    await dropPayload(row(/Work/), ENTRY_MIME, "e-github");
    await waitFor(() => expect(calls(fake, "entryMove")).toHaveLength(1));
    expect(calls(fake, "entryMove")[0]?.args).toEqual({ id: "e-github", groupId: WORK_GROUP_ID });
    // The backend event refreshes the counts.
    await waitFor(() => expect(entries.sidebar?.groups.find((g) => g.id === WORK_GROUP_ID)?.count).toBe(2));
  });

  it("drag_group_onto_group_calls_group_move", async () => {
    const { fake } = await openVault();
    await dragAndDrop(row(/Work/), row(/Banking/));
    await waitFor(() => expect(calls(fake, "groupMove")).toHaveLength(1));
    expect(calls(fake, "groupMove")[0]?.args).toEqual({ id: WORK_GROUP_ID, parentId: BANKING_GROUP_ID });
  });

  it("drag_group_onto_the_groups_heading_moves_it_to_the_top_level", async () => {
    const { fake } = await openVault();
    await dragAndDrop(row(/Banking/), screen.getByText("Groups", { selector: "[aria-hidden]" }));
    await waitFor(() => expect(calls(fake, "groupMove")).toHaveLength(1));
    expect(calls(fake, "groupMove")[0]?.args).toEqual({ id: BANKING_GROUP_ID, parentId: ROOT_GROUP_ID });
  });

  it("drag_group_into_own_descendant_blocked_client_side", async () => {
    const { fake } = await openVault();
    await dragAndDrop(row(/^Personal/), row(/Banking/));
    await dragAndDrop(row(/^Personal/), row(/^Personal/));
    expect(calls(fake, "groupMove")).toHaveLength(0);
    expect(toasts.items).toHaveLength(0);
  });

  it("backend_invalid_input_on_group_move_shows_toast", async () => {
    const { fake } = await openVault();
    const { apiError } = await import("../test/fakeBackend");
    fake.handlers.groupMove = () => {
      throw apiError("invalidInput");
    };
    await dragAndDrop(row(/Work/), row(/Banking/));
    await waitFor(() => expect(toasts.items).toHaveLength(1));
    expect(toasts.items[0]?.text).toBe("Invalid input");
  });

  it("drop_entry_on_deleted_calls_entry_delete", async () => {
    const { fake } = await openVault();
    await dropPayload(row(/^Deleted/), ENTRY_MIME, "e-github");
    await waitFor(() => expect(calls(fake, "entryDelete")).toHaveLength(1));
    expect(calls(fake, "entryDelete")[0]?.args).toEqual({ id: "e-github" });
    await waitFor(() => expect(entries.sidebar?.categories.find((c) => c.category === "deleted")?.count).toBe(2));
  });

  it("dropping_a_bin_entry_on_deleted_purges_nothing", async () => {
    const { fake } = await openVault();
    // Bin rows are only ever listed (and so dragged) in Deleted.
    navigation.setScope({ kind: "category", category: "deleted" });
    await waitFor(() => expect(entries.items.map((e) => e.id)).toContain("e-old"));
    const before = entries.sidebar?.categories.find((c) => c.category === "deleted")?.count;
    await dropPayload(row(/^Deleted/), ENTRY_MIME, "e-old");
    expect(calls(fake, "entryDelete")).toHaveLength(0);
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(fake.state.entries.some((e) => e.summary.id === "e-old")).toBe(true);
    expect(entries.sidebar?.categories.find((c) => c.category === "deleted")?.count).toBe(before);
  });

  it("drop_on_deleted_asks_before_purging_when_the_recycle_bin_is_disabled", async () => {
    const { fake } = await renderMainApp({ session: "unlocked" }, (f) => {
      f.state.recycleBinEnabled = false;
    });
    await screen.findByRole("treeitem", { name: /^Personal/ });
    const user = userEvent.setup();
    await dropPayload(row(/^Deleted/), ENTRY_MIME, "e-github");
    const dialog = await screen.findByRole("dialog");
    expect(calls(fake, "entryDelete")).toHaveLength(0);
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(calls(fake, "entryPurge")).toHaveLength(0);
    expect(fake.state.entries.some((e) => e.summary.id === "e-github")).toBe(true);
    await dropPayload(row(/^Deleted/), ENTRY_MIME, "e-github");
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete permanently" }));
    await waitFor(() => expect(calls(fake, "entryPurge")[0]?.args).toEqual({ id: "e-github" }));
    await waitFor(() => expect(fake.state.entries.some((e) => e.summary.id === "e-github")).toBe(false));
  });

  it("dropping_a_group_on_deleted_does_nothing", async () => {
    const { fake } = await openVault();
    await dragAndDrop(row(/Work/), row(/^Deleted/));
    expect(calls(fake, "groupDelete")).toHaveLength(0);
    expect(calls(fake, "groupMove")).toHaveLength(0);
  });

  it("drag_data_carries_only_the_group_id", async () => {
    await openVault();
    const dt = await dragAndDrop(row(/Work/), row(/Banking/));
    expect(dt.types).toEqual(["application/x-mypass-group"]);
    expect(dt.getData("application/x-mypass-group")).toBe(WORK_GROUP_ID);
  });
});

describe("review fixes", () => {
  it("roving_tab_stop_survives_the_focused_row_disappearing", async () => {
    const user = userEvent.setup();
    await openVault();
    await user.click(row(/Work/));
    await fireEvent.contextMenu(row(/Work/));
    await user.click(await screen.findByRole("menuitem", { name: "Delete group" }));
    await user.click(within(await screen.findByRole("dialog", { name: "Delete group" })).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("treeitem", { name: /Work/ })).toBeNull());
    const stops = screen.getAllByRole("treeitem").filter((el) => el.tabIndex === 0);
    expect(stops).toHaveLength(1);
  });

  it("a_group_is_not_a_drop_target_over_itself_or_its_descendants", async () => {
    const { FakeDataTransfer } = await import("../test/dnd");
    await openVault();
    const dataTransfer = new FakeDataTransfer();
    await fireEvent.dragStart(row(/^Personal/), { dataTransfer });
    // fireEvent returns false when the handler called preventDefault (a drop is allowed).
    expect(await fireEvent.dragOver(row(/Banking/), { dataTransfer })).toBe(true);
    expect(await fireEvent.dragOver(row(/^Personal/), { dataTransfer })).toBe(true);
    expect(await fireEvent.dragOver(row(/Work/), { dataTransfer })).toBe(false);
    await fireEvent.dragEnd(row(/^Personal/), { dataTransfer });
    expect(await fireEvent.dragOver(row(/Banking/), { dataTransfer })).toBe(false);
  });

  it("left_moves_to_the_parent_and_right_to_the_first_child", async () => {
    const user = userEvent.setup();
    await openVault();
    row(/Banking/).focus();
    await user.keyboard("{ArrowLeft}");
    expect(document.activeElement).toBe(row(/^Personal/));
    await user.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(row(/Banking/));
  });

  it("the_categories_label_is_used_once", async () => {
    await openVault();
    expect(screen.getAllByLabelText("Categories")).toHaveLength(1);
    expect(screen.getByRole("tree", { name: "Sidebar" })).toBeTruthy();
  });
});
