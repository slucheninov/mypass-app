import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { formatDate } from "../lib/i18n/format";
import ru from "../lib/i18n/locales/ru.json";
import { setLanguage } from "../lib/i18n/index.svelte";
import { session } from "../lib/stores/session.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { apiError, createFakeBackend, type FakeBackend } from "../test/fakeBackend";
import { FIXTURE_LOGIN_PASSWORD, makeEntry, sessionStatus, type EntrySpec } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";

afterEach(() => setLanguage("en"));

const T = 1_700_000_000_000;
const DAY = 86_400_000;
const OLD_SECRET = "hunter2-OLD-secret";
const MID_SECRET = "mid-secret-PW";

const ROUTER: EntrySpec = {
  id: "e-router",
  title: "Router",
  username: "admin",
  password: "current-secret",
  modified: T,
  history: [
    { modified: T - 3 * DAY, title: "Router (first)", password: OLD_SECRET, changedFields: ["Title"] },
    { modified: T - 2 * DAY, password: MID_SECRET, changedFields: ["Password"] },
    { modified: T - DAY, username: "root", changedFields: ["UserName", "MyPass.CardNumber", "Custom Thing"] },
  ],
};

function backend(extra: EntrySpec[] = [ROUTER]): FakeBackend {
  const fake = createFakeBackend({ session: "unlocked" });
  for (const spec of extra) fake.state.entries.push(makeEntry(spec));
  return fake;
}

async function select(fake: FakeBackend, id: string) {
  await renderMainApp(fake);
  await screen.findByRole("listbox");
  navigation.select(id);
}

async function openHistory(fake: FakeBackend, id = "e-router", link_name: string | RegExp = /^View history/) {
  await select(fake, id);
  const link = await screen.findByRole("button", { name: link_name });
  await userEvent.setup().click(link);
  const dialog = await screen.findByRole("dialog");
  await within(dialog).findByRole("listbox");
  return dialog;
}

const options = (dialog: HTMLElement) => within(dialog).getAllByRole("option");

async function pickRow(dialog: HTMLElement, at: number) {
  await userEvent.setup().click(options(dialog)[at]!);
  return userEvent.setup().click(within(dialog).getByRole("button", { name: "Restore this version" }));
}


describe("history list", () => {
  it("history_lists_versions_newest_first_with_localised_dates", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    const rows = options(dialog);
    expect(rows).toHaveLength(3);
    expect(rows[0]!.textContent).toContain(formatDate(T - DAY, "datetime"));
    expect(rows[1]!.textContent).toContain(formatDate(T - 2 * DAY, "datetime"));
    expect(rows[2]!.textContent).toContain(formatDate(T - 3 * DAY, "datetime"));
    expect(rows[2]!.textContent).toContain("Router (first)");
    expect(calls(fake, "entryHistory")[0]!.args).toEqual({ id: "e-router" });
  });

  it("history_dates_and_labels_follow_the_ui_language", async () => {
    const fake = backend();
    fake.state.settings.language = "ru";
    const dialog = await openHistory(fake, "e-router", new RegExp(`^${ru["View history (%lld)"].split(" (")[0]}`));
    const dialogName = ru["History"];
    expect(dialog.getAttribute("aria-labelledby")).toBeTruthy();
    expect(screen.getByRole("dialog", { name: dialogName })).toBe(dialog);
    expect(options(dialog)[0]!.textContent).toContain(formatDate(T - DAY, "datetime"));
    expect(options(dialog)[1]!.textContent).toContain(ru["Password"]);
  });

  it("changed_field_names_shown_without_values", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    const rows = options(dialog);
    expect(rows[1]!.textContent).toContain("Changed: Password");
    // Known KDBX keys are localised (R29), unknown custom keys are user data and shown as they are.
    expect(rows[0]!.textContent).toContain("Changed: Username, Card number, Custom Thing");
    expect(rows[2]!.textContent).toContain("Changed: Title");
    expect(document.body.textContent).not.toContain(OLD_SECRET);
    expect(document.body.textContent).not.toContain(MID_SECRET);
    expect(document.body.textContent).not.toContain("current-secret");
    expect(calls(fake, "entryReveal")).toHaveLength(0);
    expect(calls(fake, "entryCopy")).toHaveLength(0);
    expect(document.body.textContent).not.toContain("MyPass.CardNumber");
  });

  it("empty_history_shows_empty_state", async () => {
    const fake = backend();
    fake.handlers.entryHistory = () => [];
    await select(fake, "e-router");
    await userEvent.setup().click(await screen.findByRole("button", { name: /^View history/ }));
    const dialog = await screen.findByRole("dialog", { name: "History" });
    expect(await within(dialog).findByText("No earlier versions")).toBeTruthy();
    expect(within(dialog).queryByRole("listbox")).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "Restore this version" })).toBeNull();
  });

  it("a_failed_history_fetch_shows_a_toast_and_closes", async () => {
    const fake = backend();
    fake.handlers.entryHistory = () => {
      throw apiError("notFound");
    };
    await select(fake, "e-router");
    await userEvent.setup().click(await screen.findByRole("button", { name: /^View history/ }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "History" })).toBeNull());
    expect(await screen.findByText("Item not found", { selector: "[role=status] *, [role=alert] *, [role=status], [role=alert]" })).toBeTruthy();
  });

  it("history_passes_axe", async () => {
    const dialog = await openHistory(backend());
    await expectNoAxeViolations(dialog);
  });

  it("history_with_the_restore_confirmation_stacked_passes_axe", async () => {
    const dialog = await openHistory(backend());
    await userEvent.setup().click(within(dialog).getByRole("button", { name: "Restore this version" }));
    const confirm = await screen.findByRole("dialog", { name: "Restore this version" });
    await expectNoAxeViolations(confirm);
    await expectNoAxeViolations(document.body);
  });

  it("a_version_without_a_date_says_so_instead_of_repeating_its_title", async () => {
    const fake = backend();
    fake.handlers.entryHistory = () => [
      { index: 0, modified: null, title: "Router (undated)", changedFields: ["Title"] },
    ];
    const dialog = await openHistory(fake);
    const row = options(dialog)[0]!;
    expect(row.textContent).toContain("Unknown date");
    expect(row.textContent?.split("Router (undated)").length).toBe(2);
    await userEvent.setup().click(within(dialog).getByRole("button", { name: "Restore this version" }));
    const confirm = await screen.findByRole("dialog", { name: "Restore this version" });
    expect(confirm.textContent).toContain("Restore the version from Unknown date?");
  });
});

describe("restore a version", () => {
  it("restore_asks_confirmation_then_calls_entry_history_restore_with_index", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    await pickRow(dialog, 1);
    const confirm = await screen.findByRole("dialog", { name: "Restore this version" });
    expect(confirm.textContent).toContain(
      `Restore the version from ${formatDate(T - 2 * DAY, "datetime")}? The current version is kept in history.`,
    );
    expect(calls(fake, "entryHistoryRestore")).toHaveLength(0);
    await userEvent.setup().click(within(confirm).getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(calls(fake, "entryHistoryRestore")).toHaveLength(1));
    expect(calls(fake, "entryHistoryRestore")[0]!.args).toEqual({ id: "e-router", index: 1 });
  });

  it("cancelling_the_confirmation_restores_nothing_and_keeps_the_list", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    await pickRow(dialog, 0);
    const confirm = await screen.findByRole("dialog", { name: "Restore this version" });
    await userEvent.setup().click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Restore this version" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "History" })).toBeTruthy();
    expect(calls(fake, "entryHistoryRestore")).toHaveLength(0);
  });

  it("history_closed_and_detail_refetched_after_restore", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    const before = calls(fake, "entryGet").length;
    await pickRow(dialog, 0);
    await userEvent
      .setup()
      .click(within(await screen.findByRole("dialog", { name: "Restore this version" })).getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(calls(fake, "entryGet").length).toBeGreaterThan(before));
    // The restore is itself a version: the link now counts four.
    expect(await screen.findByRole("button", { name: "View history (4)" })).toBeTruthy();
    expect(within(screen.getByRole("region", { name: "Details" })).getByText("root")).toBeTruthy();
  });

  it("restore_failure_shows_localised_toast_and_keeps_modal", async () => {
    const fake = backend();
    fake.handlers.entryHistoryRestore = () => {
      throw apiError("notFound");
    };
    const dialog = await openHistory(fake);
    await pickRow(dialog, 0);
    await userEvent
      .setup()
      .click(within(await screen.findByRole("dialog", { name: "Restore this version" })).getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Restore this version" })).toBeNull());
    expect(screen.getByRole("dialog", { name: "History" })).toBeTruthy();
    expect(screen.getAllByText("Item not found").length).toBeGreaterThan(0);
  });

  it("history_modal_keyboard_navigation_up_down_enter", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    const list = within(dialog).getByRole("listbox");
    list.focus();
    const selected = () => options(dialog).findIndex((o) => o.getAttribute("aria-selected") === "true");
    expect(selected()).toBe(0);
    await userEvent.setup().keyboard("{ArrowDown}");
    expect(selected()).toBe(1);
    await userEvent.setup().keyboard("{ArrowDown}{ArrowDown}");
    expect(selected()).toBe(2);
    await userEvent.setup().keyboard("{ArrowUp}");
    expect(selected()).toBe(1);
    expect(list.getAttribute("aria-activedescendant")).toBe(options(dialog)[1]!.id);
    await userEvent.setup().keyboard("{Enter}");
    expect(await screen.findByRole("dialog", { name: "Restore this version" })).toBeTruthy();
    await userEvent.setup().keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Restore this version" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "History" })).toBeTruthy();
    expect(calls(fake, "entryHistoryRestore")).toHaveLength(0);
  });
});

describe("lifetime", () => {
  it("history_closes_when_the_selection_changes", async () => {
    const fake = backend([ROUTER, { id: "e-other", title: "Other", modified: 5 }]);
    await openHistory(fake);
    navigation.select("e-other");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "History" })).toBeNull());
  });

  it("history_closes_on_lock_and_stays_closed_after_unlock", async () => {
    const fake = backend();
    await openHistory(fake);
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "History" })).toBeNull());
    await fake.emit("sessionChanged", sessionStatus("unlocked"));
    await screen.findByRole("listbox");
    navigation.select("e-router");
    await screen.findByRole("heading", { name: "Router" });
    expect(screen.queryByRole("dialog", { name: "History" })).toBeNull();
  });

  it("an_editor_opening_closes_history_so_a_restore_never_lands_under_unsaved_edits", async () => {
    const fake = backend();
    await openHistory(fake);
    const model = await fake.handlers.entryEditModel({ id: "e-router" });
    navigation.requestOpenEditor(model);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "History" })).toBeNull());
    expect(calls(fake, "entryHistoryRestore")).toHaveLength(0);
  });

  it("a_reload_while_the_confirmation_is_open_keeps_the_chosen_version_not_the_index", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    // Choose the second row (T - 2 days), index 1 now.
    await pickRow(dialog, 1);
    const confirm = await screen.findByRole("dialog", { name: "Restore this version" });
    // A sync adds a newer version: the chosen one moves to index 2.
    fake.state.entries
      .find((e) => e.summary.id === "e-router")!
      .history.push({ modified: T - 1000, model: structuredClone(fake.state.entries[0]!.model) });
    await fake.emit("vaultChanged", null);
    await waitFor(() => expect(options(dialog)).toHaveLength(4));
    await userEvent.setup().click(within(confirm).getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(calls(fake, "entryHistoryRestore")).toHaveLength(1));
    expect(calls(fake, "entryHistoryRestore")[0]!.args).toEqual({ id: "e-router", index: 2 });
  });

  it("a_reload_that_removes_the_chosen_version_drops_the_confirmation", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    await pickRow(dialog, 2);
    await screen.findByRole("dialog", { name: "Restore this version" });
    fake.state.entries.find((e) => e.summary.id === "e-router")!.history.shift();
    await fake.emit("vaultChanged", null);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Restore this version" })).toBeNull());
    expect(calls(fake, "entryHistoryRestore")).toHaveLength(0);
  });

  it("switching_vaults_closes_history_and_a_same_id_selection_does_not_reopen_it", async () => {
    const fake = backend();
    await openHistory(fake);
    await fake.emit("sessionChanged", { ...sessionStatus("unlocked"), vaultPath: "/vaults/b.kdbx", vaultName: "Other" });
    await waitFor(() => expect(session.status.vaultPath).toBe("/vaults/b.kdbx"));
    // Same id is selected again in the new vault (ids can collide across copies of a vault).
    navigation.select("e-router");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "History" })).toBeNull());
  });

  it("the_history_list_refreshes_when_the_vault_changes", async () => {
    const fake = backend();
    const dialog = await openHistory(fake);
    expect(options(dialog)).toHaveLength(3);
    fake.state.entries.find((e) => e.summary.id === "e-router")!.history.shift();
    await fake.emit("vaultChanged", null);
    await waitFor(() => expect(options(dialog)).toHaveLength(2));
  });
});

describe("recycle bin restore", () => {
  it("restore_from_bin_passes_null_so_the_backend_restores_to_the_root", async () => {
    const fake = backend([]);
    await select(fake, "e-old");
    const pane = await screen.findByRole("region", { name: "Details" });
    await within(pane).findByRole("heading", { name: "Old forum" });
    // The default restore: no group, the backend picks (the root, as it remembers no origin).
    await userEvent.setup().click(within(pane).getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(calls(fake, "entryRestore")).toHaveLength(1));
    expect(calls(fake, "entryRestore")[0]!.args).toEqual({ id: "e-old", groupId: null });
  });

  it("restore_to_opens_a_group_picker_and_restores_into_the_chosen_group", async () => {
    const fake = backend([]);
    await select(fake, "e-old");
    const pane = await screen.findByRole("region", { name: "Details" });
    await within(pane).findByRole("heading", { name: "Old forum" });
    await userEvent.setup().click(within(pane).getByRole("button", { name: "Restore to…" }));
    const picker = await screen.findByRole("dialog", { name: "Choose a group" });
    expect(within(picker).queryByRole("treeitem", { name: /Bin/ })).toBeNull();
    await userEvent.setup().click(within(picker).getByRole("treeitem", { name: /Work/ }));
    await waitFor(() => expect(calls(fake, "entryRestore")).toHaveLength(1));
    expect(calls(fake, "entryRestore")[0]!.args).toEqual({ id: "e-old", groupId: "group-work" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("the_restore_to_group_picker_passes_axe_from_the_detail_pane_and_from_the_list", async () => {
    const fake = backend([]);
    await select(fake, "e-old");
    const pane = await screen.findByRole("region", { name: "Details" });
    await within(pane).findByRole("heading", { name: "Old forum" });
    await userEvent.setup().click(within(pane).getByRole("button", { name: "Restore to…" }));
    await expectNoAxeViolations(await screen.findByRole("dialog", { name: "Choose a group" }));
    await userEvent.setup().keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    navigation.setScope({ kind: "category", category: "deleted" });
    await fireEvent.contextMenu(await screen.findByRole("option", { name: /Old forum/ }));
    await userEvent.setup().click(screen.getByRole("menuitem", { name: "Restore to…" }));
    await expectNoAxeViolations(await screen.findByRole("dialog", { name: "Choose a group" }));
  });

  it("the_list_context_menu_restores_a_bin_entry_and_offers_restore_to", async () => {
    const fake = backend([]);
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    navigation.setScope({ kind: "category", category: "deleted" });
    const row = await screen.findByRole("option", { name: /Old forum/ });
    await fireEvent.contextMenu(row);
    const menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: "Move to…" })).toBeTruthy();
    expect(within(menu).getByRole("menuitem", { name: "Move to…" }).getAttribute("aria-disabled")).toBe("true");
    await userEvent.setup().click(within(menu).getByRole("menuitem", { name: "Restore" }));
    await waitFor(() => expect(calls(fake, "entryRestore")).toHaveLength(1));
    expect(calls(fake, "entryRestore")[0]!.args).toEqual({ id: "e-old", groupId: null });
  });

  it("the_list_context_menu_restore_to_picks_a_group", async () => {
    const fake = backend([]);
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    navigation.setScope({ kind: "category", category: "deleted" });
    await fireEvent.contextMenu(await screen.findByRole("option", { name: /Old forum/ }));
    await userEvent.setup().click(await screen.findByRole("menuitem", { name: "Restore to…" }));
    const picker = await screen.findByRole("dialog", { name: "Choose a group" });
    await userEvent.setup().click(within(picker).getByRole("treeitem", { name: /Banking/ }));
    await waitFor(() => expect(calls(fake, "entryRestore")).toHaveLength(1));
    expect(calls(fake, "entryRestore")[0]!.args).toEqual({ id: "e-old", groupId: "group-banking" });
  });

  it("live_entries_have_no_restore_actions_in_the_context_menu", async () => {
    const fake = backend([]);
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    await fireEvent.contextMenu(await screen.findByRole("option", { name: /GitHub/ }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: "Restore" })).toBeNull();
    expect(within(menu).queryByRole("menuitem", { name: "Restore to…" })).toBeNull();
  });
});

describe("fake backend parity", () => {
  it("history_restore_snapshots_the_current_state_and_keeps_group_and_favorite", async () => {
    const fake = backend();
    fake.install();
    const entry = fake.state.entries.find((e) => e.summary.id === "e-router")!;
    entry.model.favorite = true;
    await fake.handlers.entryHistoryRestore({ id: "e-router", index: 2 });
    expect(entry.history).toHaveLength(4);
    expect(entry.history[3]!.model.password).toBe("current-secret");
    expect(entry.model.password).toBe(OLD_SECRET);
    expect(entry.model.title).toBe("Router (first)");
    expect(entry.model.favorite).toBe(true);
    expect(entry.model.groupId).toBe("group-personal");
    const listed = await fake.handlers.entryHistory({ id: "e-router" });
    expect(listed.map((h) => h.index)).toEqual([0, 1, 2, 3]);
    expect(listed[0]!.changedFields).toEqual(["Title", "Password"]);
  });

  it("history_restore_with_an_unknown_index_is_not_found", async () => {
    const fake = backend();
    for (const index of [3, 99, -1]) {
      await expect(Promise.resolve().then(() => fake.handlers.entryHistoryRestore({ id: "e-router", index }))).rejects.toMatchObject({
        code: "notFound",
      });
    }
  });

  it("entry_restore_refuses_an_entry_that_is_not_in_the_bin", async () => {
    const fake = backend();
    await expect(
      Promise.resolve().then(() => fake.handlers.entryRestore({ id: "e-router", groupId: null })),
    ).rejects.toMatchObject({ code: "invalidInput" });
    await expect(
      Promise.resolve().then(() => fake.handlers.entryRestore({ id: "e-old", groupId: "no-such-group" })),
    ).rejects.toMatchObject({ code: "notFound" });
  });

  it("saving_an_existing_entry_snapshots_it_into_history", async () => {
    const fake = backend();
    fake.install();
    const entry = fake.state.entries.find((e) => e.summary.id === "e-router")!;
    const model = { ...structuredClone(entry.model), username: "changed" };
    await fake.handlers.entrySave({ model });
    expect(entry.history).toHaveLength(4);
    expect(entry.history[3]!.model.username).toBe("admin");
  });

  it("a_default_login_password_is_not_a_history_value", () => {
    // Guards the fixture assumption of the no-values tests above.
    expect(FIXTURE_LOGIN_PASSWORD).not.toBe(OLD_SECRET);
  });
});
