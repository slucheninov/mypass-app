import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { navigation } from "../lib/stores/navigation.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { createFakeBackend, apiError } from "../test/fakeBackend";
import { calls, renderMainApp } from "../test/mainApp";

afterEach(() => navigation.reset());

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
