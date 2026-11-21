import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PENDING_NEW_ENTRY_MS } from "../lib/stores/events";
import { navigation } from "../lib/stores/navigation.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { createFakeBackend, apiError } from "../test/fakeBackend";
import { FIXTURE_MASTER_PASSWORD } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";

afterEach(() => {
  navigation.reset();
  vi.restoreAllMocks();
});

async function unlock(user: ReturnType<typeof userEvent.setup>) {
  await user.type(await screen.findByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
  await user.click(screen.getByRole("button", { name: "Unlock" }));
  await screen.findByRole("listbox");
}

async function mainApp(session: "unlocked" | "locked" = "unlocked") {
  const fake = createFakeBackend({ session });
  const app = await renderMainApp(fake);
  if (session === "unlocked") await screen.findByRole("listbox");
  return { ...app, fake };
}

describe("main window event bridge", () => {
  it("clipboard_cleared_event_shows_toast", async () => {
    const { fake } = await mainApp();
    await fake.emit("clipboardClearedEvent", null);
    await waitFor(() => expect(toasts.items.map((x) => [x.kind, x.text])).toEqual([["info", "Clipboard cleared"]]));
    expect((await screen.findAllByRole("status")).some((el) => el.textContent?.includes("Clipboard cleared"))).toBe(true);
  });

  it("save_failed_event_shows_sticky_error_toast_with_details_collapsed", async () => {
    const { fake } = await mainApp();
    await fake.emit("saveStateChanged", { state: "failed", message: "disk is full" });
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Saving failed. MyPass will retry.");
    const details = alert.querySelector("details");
    expect(details).not.toBeNull();
    expect(details?.open).toBe(false);
    expect(toasts.items[0]?.sticky).toBe(true);
    // Recovery removes it again.
    await fake.emit("saveStateChanged", { state: "idle", message: null });
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("main_app_opens_editor_on_open_entry_event", async () => {
    const { fake } = await mainApp();
    await fake.emit("openEntryEvent", { id: "e-gitlab", edit: true });
    await waitFor(() => expect(navigation.presentation).toBe("edit"));
    expect(navigation.editModel?.id).toBe("e-gitlab");
    expect(navigation.selectedId).toBe("e-gitlab");
    expect(calls(fake, "entryEditModel")[0]?.args).toEqual({ id: "e-gitlab" });
  });

  it("open_entry_event_without_edit_only_selects", async () => {
    const { fake } = await mainApp();
    await fake.emit("openEntryEvent", { id: "e-gitlab", edit: false });
    await waitFor(() => expect(navigation.selectedId).toBe("e-gitlab"));
    expect(navigation.presentation).toBe("browse");
    expect(calls(fake, "entryEditModel")).toHaveLength(0);
  });

  it("open_entry_event_without_edit_leaves_the_selection_alone_while_an_editor_is_open", async () => {
    const { fake } = await mainApp();
    await fake.emit("openEntryEvent", { id: "e-github", edit: true });
    await waitFor(() => expect(navigation.presentation).toBe("edit"));
    await fake.emit("openEntryEvent", { id: "e-gitlab", edit: false });
    await new Promise((r) => setTimeout(r, 50));
    expect(navigation.selectedId).toBe("e-github");
    expect(navigation.editModel?.id).toBe("e-github");
  });

  it("new_entry_event_opens_the_editor_for_a_new_login", async () => {
    const { fake } = await mainApp();
    await fake.emit("newEntryEvent", { entryType: "login" });
    await waitFor(() => expect(navigation.presentation).toBe("create"));
    expect(navigation.editModel?.id ?? null).toBeNull();
    expect(navigation.editModel?.entryType).toBe("login");
    expect(calls(fake, "entryNewModel")[0]?.args).toEqual({ entryType: "login", groupId: null });
  });

  it("new_entry_event_while_locked_opens_nothing", async () => {
    const { fake } = await mainApp("locked");
    await fake.emit("newEntryEvent", { entryType: "login" });
    await new Promise((r) => setTimeout(r, 50));
    expect(navigation.presentation).toBe("browse");
    expect(calls(fake, "entryNewModel")).toHaveLength(0);
  });

  it("open_entry_event_while_locked_opens_nothing", async () => {
    const { fake } = await mainApp("locked");
    await fake.emit("openEntryEvent", { id: "e-gitlab", edit: true });
    await new Promise((r) => setTimeout(r, 50));
    expect(navigation.presentation).toBe("browse");
    expect(navigation.selectedId).toBeNull();
    expect(calls(fake, "entryEditModel")).toHaveLength(0);
    expect(screen.getByTestId("unlock-screen")).toBeTruthy();
  });

  it("open_entry_event_over_a_dirty_editor_asks_before_replacing_it", async () => {
    const user = userEvent.setup();
    const { fake } = await mainApp();
    await fake.emit("openEntryEvent", { id: "e-github", edit: true });
    await waitFor(() => expect(navigation.editModel?.id).toBe("e-github"));
    const title = await screen.findByRole("textbox", { name: "Title" });
    await user.type(title, " changed");
    await fake.emit("openEntryEvent", { id: "e-gitlab", edit: true });
    const dialog = await screen.findByRole("dialog", { name: "Discard changes?" });
    expect(navigation.editModel?.id).toBe("e-github");
    await user.click(within(dialog).getByRole("button", { name: "Keep editing" }));
    expect(navigation.editModel?.id).toBe("e-github");
    expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toContain("changed");
  });

  it("discarding_for_an_open_entry_event_opens_that_entry_and_selects_it", async () => {
    const user = userEvent.setup();
    const { fake } = await mainApp();
    await fake.emit("openEntryEvent", { id: "e-github", edit: true });
    await waitFor(() => expect(navigation.editModel?.id).toBe("e-github"));
    await user.type(await screen.findByRole("textbox", { name: "Title" }), " changed");
    await fake.emit("openEntryEvent", { id: "e-gitlab", edit: true });
    const dialog = await screen.findByRole("dialog", { name: "Discard changes?" });
    await user.click(within(dialog).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(navigation.editModel?.id).toBe("e-gitlab"));
    expect(navigation.selectedId).toBe("e-gitlab");
  });

  it.each(["address", "files"] as const)("new_entry_event_opens_a_new_%s_editor_like_the_toolbar", async (entryType) => {
    const { fake } = await mainApp();
    await fake.emit("newEntryEvent", { entryType });
    await waitFor(() => expect(navigation.presentation).not.toBe("browse"));
    expect(navigation.editModel?.entryType).toBe(entryType);
    expect(navigation.editModel?.id).toBeNull();
    expect(calls(fake, "entryNewModel").map((c) => c.args)).toEqual([{ entryType, groupId: null }]);
  });

  it("new_entry_event_lands_in_the_selected_group_like_the_toolbar", async () => {
    const { fake } = await mainApp();
    navigation.setScope({ kind: "group", groupId: "group-work" });
    await fake.emit("newEntryEvent", { entryType: "files" });
    await waitFor(() => expect(navigation.editModel?.entryType).toBe("files"));
    expect(calls(fake, "entryNewModel")[0]?.args).toEqual({ entryType: "files", groupId: "group-work" });
  });

  it("new_entry_event_over_a_dirty_editor_asks_before_replacing_it", async () => {
    const user = userEvent.setup();
    const { fake } = await mainApp();
    await fake.emit("openEntryEvent", { id: "e-github", edit: true });
    await waitFor(() => expect(navigation.editModel?.id).toBe("e-github"));
    await user.type(await screen.findByRole("textbox", { name: "Title" }), " changed");
    await fake.emit("newEntryEvent", { entryType: "address" });
    const dialog = await screen.findByRole("dialog", { name: "Discard changes?" });
    expect(navigation.editModel?.id).toBe("e-github");
    await user.click(within(dialog).getByRole("button", { name: "Keep editing" }));
    expect(navigation.editModel?.id).toBe("e-github");
    expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toContain("changed");
    // Asked again, Discard opens the new address.
    await fake.emit("newEntryEvent", { entryType: "address" });
    await user.click(within(await screen.findByRole("dialog", { name: "Discard changes?" })).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(navigation.editModel?.entryType).toBe("address"));
    expect(navigation.editModel?.id).toBeNull();
  });

  it("new_entry_event_while_locked_opens_the_editor_after_the_unlock", async () => {
    const user = userEvent.setup();
    const { fake } = await mainApp("locked");
    await fake.emit("newEntryEvent", { entryType: "address" });
    // The newest request wins.
    await fake.emit("newEntryEvent", { entryType: "files" });
    await new Promise((r) => setTimeout(r, 50));
    expect(calls(fake, "entryNewModel")).toHaveLength(0);
    expect(screen.getByTestId("unlock-screen")).toBeTruthy();
    await unlock(user);
    await waitFor(() => expect(navigation.editModel?.entryType).toBe("files"));
    expect(calls(fake, "entryNewModel")).toHaveLength(1);
  });

  it("new_entry_event_while_locked_is_dropped_after_its_wait_runs_out", async () => {
    const user = userEvent.setup();
    const { fake } = await mainApp("locked");
    const start = Date.now();
    const now = vi.spyOn(Date, "now").mockReturnValue(start);
    await fake.emit("newEntryEvent", { entryType: "files" });
    now.mockReturnValue(start + PENDING_NEW_ENTRY_MS + 1);
    await unlock(user);
    await new Promise((r) => setTimeout(r, 50));
    expect(calls(fake, "entryNewModel")).toHaveLength(0);
    expect(navigation.presentation).toBe("browse");
  });

  it("a_pending_new_entry_runs_once_and_not_again_on_a_later_unlock", async () => {
    const user = userEvent.setup();
    const { fake } = await mainApp("locked");
    await fake.emit("newEntryEvent", { entryType: "address" });
    await unlock(user);
    await waitFor(() => expect(navigation.editModel?.entryType).toBe("address"));
    await fake.handlers.vaultLock({});
    await screen.findByTestId("unlock-screen");
    await unlock(user);
    await new Promise((r) => setTimeout(r, 50));
    expect(calls(fake, "entryNewModel")).toHaveLength(1);
    expect(navigation.presentation).toBe("browse");
  });

  it("the_conflict_listener_is_registered_before_the_backlog_is_taken", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    // An event raised while the backlog is being taken is only seen by an already registered listener.
    fake.handlers.shortcutConflictsTake = () => {
      void fake.emit("shortcutConflictEvent", { kind: "show", shortcut: "Ctrl+K" });
      return [];
    };
    await renderMainApp(fake);
    await waitFor(() => expect(toasts.items.some((x) => x.text.includes("Ctrl+K"))).toBe(true));
  });

  it("shortcut_conflicts_are_taken_once_after_the_listener_is_registered", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.handlers.shortcutConflictsTake = () => [{ kind: "lock", shortcut: "Ctrl+L" }];
    await renderMainApp(fake);
    await waitFor(() => expect(toasts.items.some((x) => x.kind === "error" && x.text.includes("Ctrl+L"))).toBe(true));
    expect(calls(fake, "shortcutConflictsTake")).toHaveLength(1);
    await fake.emit("shortcutConflictEvent", { kind: "show", shortcut: "Ctrl+K" });
    await waitFor(() => expect(toasts.items.some((x) => x.text.includes("Ctrl+K"))).toBe(true));
  });

  it("a_failing_conflict_take_is_swallowed", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.handlers.shortcutConflictsTake = () => {
      throw apiError("internal");
    };
    await renderMainApp(fake);
    await screen.findByRole("listbox");
    expect(toasts.items).toHaveLength(0);
  });
});
