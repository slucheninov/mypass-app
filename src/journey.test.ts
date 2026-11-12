// End-to-end journeys: the real window roots on the fake backend, driven the way a user would
// (roles and labels), with the backend's events delivered the way the Rust side emits them.
import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setLanguage } from "./lib/i18n/index.svelte";
import ru from "./lib/i18n/locales/ru.json";
import { entries } from "./lib/stores/entries.svelte";
import { navigation } from "./lib/stores/navigation.svelte";
import { session } from "./lib/stores/session.svelte";
import { createFakeBackend, type FakeBackend } from "./test/fakeBackend";
import {
  FIXTURE_LOGIN_PASSWORD,
  FIXTURE_MASTER_PASSWORD,
  WORK_GROUP_ID,
  fixtureGroups,
  makeEntry,
  sessionStatus,
} from "./test/fixtures";
import { dragAndDrop } from "./test/dnd";
import { calls, renderMainApp } from "./test/mainApp";
import SettingsApp from "./windows/SettingsApp.svelte";
import TrayPanel from "./windows/TrayPanel.svelte";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
import { open, save } from "@tauri-apps/plugin-dialog";

beforeEach(() => {
  vi.mocked(open).mockReset();
  vi.mocked(save).mockReset();
});
afterEach(() => setLanguage("en"));

const body = () => document.body.textContent ?? "";
const options = () => {
  const list = screen.queryByRole("listbox");
  return list ? within(list).queryAllByRole("option") : [];
};
const titles = () => options().map((o) => o.querySelector("[data-part='title']")?.textContent?.trim());
const option = (name: RegExp | string) => screen.getByRole("option", { name });
const category = (name: RegExp | string) => screen.getByRole("treeitem", { name });
const pane = () => screen.getByRole("region", { name: "Details" });
const nonBin = (fake: FakeBackend) => fake.state.entries.filter((e) => !e.summary.inRecycleBin);

/** The unlocked main window with the list ready. */
async function unlocked(fake: FakeBackend = createFakeBackend({ session: "unlocked" })) {
  const app = await renderMainApp(fake);
  await screen.findByRole("listbox");
  await waitFor(() => expect(options().length).toBeGreaterThan(0));
  return { ...app, fake, user: userEvent.setup() };
}

async function menuItem(user: ReturnType<typeof userEvent.setup>, button: string, item: string) {
  await user.click(screen.getByRole("button", { name: button }));
  await user.click(await screen.findByRole("menuitem", { name: item }));
}

async function reveal(user: ReturnType<typeof userEvent.setup>, label: string) {
  await user.click(within(within(pane()).getByRole("group", { name: label })).getByRole("button", { name: "Reveal" }));
}

describe("journeys", () => {
  it("journey_unlock_browse_edit_lock", async () => {
    const fake = createFakeBackend();
    const user = userEvent.setup();
    await renderMainApp(fake);
    await user.type(await screen.findByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    await screen.findByRole("listbox");
    const names = within(screen.getByRole("group", { name: "Categories" })).getAllByRole("treeitem");
    expect(names).toHaveLength(10);
    await waitFor(() => expect(options()).toHaveLength(nonBin(fake).length));
    expect(titles()).toContain("GitHub");
    await user.click(option(/GitHub/));
    await screen.findByRole("heading", { name: "GitHub" });
    expect(body()).not.toContain(FIXTURE_LOGIN_PASSWORD);
    await reveal(user, "Password");
    await waitFor(() => expect(body()).toContain(FIXTURE_LOGIN_PASSWORD));
    await user.click(within(pane()).getByRole("button", { name: "Edit" }));
    const title = await screen.findByLabelText("Title");
    await user.clear(title);
    await user.type(title, "GitHub Work");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(navigation.editModel).toBeNull());
    await waitFor(() => expect(titles()).toContain("GitHub Work"));
    expect(calls(fake, "entrySave")).toHaveLength(1);

    await fake.emit("sessionChanged", sessionStatus("locked"));
    await screen.findByLabelText("Master password");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(body()).not.toContain(FIXTURE_LOGIN_PASSWORD);
    expect(body()).not.toContain("GitHub Work");
    expect(navigation.editModel).toBeNull();
    expect(entries.items).toEqual([]);
  });

  it("journey_create_vault_then_add_login_then_generate_password", async () => {
    vi.mocked(save).mockResolvedValue("/vaults/Fresh.kdbx");
    const fake = createFakeBackend({ session: "noVault" });
    const user = userEvent.setup();
    await renderMainApp(fake);
    await user.click(await screen.findByRole("button", { name: "Create vault" }));
    const dialog = await screen.findByRole("dialog", { name: "Create vault" });
    await user.type(within(dialog).getByLabelText("Master password"), "longenough1");
    await user.type(within(dialog).getByLabelText("Confirm password"), "longenough1");
    await user.click(within(dialog).getByRole("button", { name: "Create vault" }));
    await screen.findByTestId("entry-list");
    expect(await screen.findByTestId("empty-state")).toBeTruthy();

    await menuItem(user, "New Entry", "New Login");
    await user.type(await screen.findByLabelText("Title"), "Bank");
    await user.click(screen.getByRole("button", { name: "Generate" }));
    const tabs = await screen.findByRole("tablist", { name: "Password Generator…" });
    await user.click(within(tabs.parentElement as HTMLElement).getByRole("button", { name: "Generate" }));
    await user.click(await screen.findByRole("button", { name: "Use password" }));
    const generated = (screen.getByLabelText("Password") as HTMLInputElement).value;
    expect(generated).toHaveLength(20);
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(titles()).toEqual(["Bank"]));
    expect(fake.state.entries[0]?.model.password).toBe(generated);
  });

  it("journey_search_and_category_switch", async () => {
    const { user } = await unlocked();
    const search = screen.getByRole("searchbox", { name: "Search" });
    await user.type(search, "git");
    await waitFor(() => expect(options()).toHaveLength(2));
    // GitHub and GitLab both match; narrow to one.
    await user.type(search, "hub");
    await waitFor(() => expect(options()).toHaveLength(1));
    expect(screen.getByTestId("list-count").textContent).toBe("1 matching item");
    await user.click(category(/^Cards/));
    await waitFor(() => expect(titles()).toEqual([]));
    await user.clear(search);
    await waitFor(() => expect(titles()).toEqual(["Visa"]));
    await user.click(category(/^All/));
    await waitFor(() => expect(options().length).toBeGreaterThan(1));
    expect(screen.getByTestId("list-count").textContent).toMatch(/^\d+ items$/);
  });

  it("journey_drag_entry_to_group", async () => {
    const { fake } = await unlocked();
    const workCount = () => Number(/(\d+)$/.exec(category(/^Work/).textContent?.trim() ?? "")?.[1]);
    const before = workCount();
    await dragAndDrop(option(/GitHub/), category(/^Work/));
    await waitFor(() => expect(calls(fake, "entryMove")).toHaveLength(1));
    expect(calls(fake, "entryMove")[0]?.args).toEqual({ id: "e-github", groupId: WORK_GROUP_ID });
    await waitFor(() => expect(workCount()).toBe(before + 1));
  });

  it("journey_language_switch_ru_updates_sidebar_labels", async () => {
    const { user } = await unlocked();
    render(SettingsApp);
    await user.selectOptions(await screen.findByRole("combobox", { name: "Language" }), "ru");
    const labels = ["All", "Favorites", "Codes", "Passkeys", "Addresses", "Cards", "Notes", "Files", "Security", "Deleted"];
    const group = screen.getByRole("group", { name: ru["Categories"] });
    await waitFor(() => {
      const shown = within(group)
        .getAllByRole("treeitem")
        .map((el) => (el.textContent ?? "").trim().replace(/\s+(\d+|—)$/, ""));
      expect(shown).toEqual(labels.map((l) => (ru as Record<string, string>)[l]));
    });
  });

  it("journey_import_csv_then_visible", async () => {
    const { fake, user } = await unlocked();
    fake.state.csvPreview = { total: 2, sampleTitles: ["Acme, Inc", "Globex"], skipped: [] };
    await menuItem(user, "Import", "Import CSV…");
    const dialog = await screen.findByRole("dialog", { name: "Import CSV" });
    expect(await within(dialog).findByText("2 items found")).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Import" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Import CSV" })).toBeNull());
    await waitFor(() => expect(titles()).toContain("Acme, Inc"));
    expect(calls(fake, "importCsvCommit")).toHaveLength(1);
  });

  it("journey_idle_lock_event_during_editing_discards_edit_model", async () => {
    const { fake, user } = await unlocked();
    await user.click(option(/GitHub/));
    await user.click(await within(pane()).findByRole("button", { name: "Edit" }));
    const password = (await screen.findByLabelText("Password")) as HTMLInputElement;
    await user.clear(password);
    await user.type(password, "typed-secret-xyz");
    expect(password.value).toBe("typed-secret-xyz");
    await fake.emit("sessionChanged", { ...sessionStatus("locked"), lockReason: "idle" });
    await screen.findByLabelText("Master password");
    const inputs = Array.from(document.querySelectorAll("input"));
    expect(inputs.some((i) => i.value.includes("typed-secret-xyz"))).toBe(false);
    expect(body()).not.toContain("typed-secret-xyz");
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(navigation.editModel).toBeNull();
  });

  it("journey_external_change_event_refreshes_list", async () => {
    const { fake } = await unlocked();
    expect(titles()).not.toContain("Added elsewhere");
    fake.state.entries.push(makeEntry({ id: "e-ext", title: "Added elsewhere", modified: 1_700_000_900_000 }));
    await fake.emit("vaultChanged", null);
    await waitFor(() => expect(titles()).toContain("Added elsewhere"));
  });

  it("journey_delete_restore_from_deleted", async () => {
    const { fake, user } = await unlocked();
    await user.click(option(/GitHub/));
    await user.click(await within(pane()).findByRole("button", { name: "Delete" }));
    await waitFor(() => expect(titles()).not.toContain("GitHub"));
    await user.click(category(/^Deleted/));
    await waitFor(() => expect(titles()).toContain("GitHub"));
    await user.click(option(/GitHub/));
    await user.click(await within(pane()).findByRole("button", { name: "Restore" }));
    await waitFor(() => expect(calls(fake, "entryRestore")).toHaveLength(1));
    await waitFor(() => expect(titles()).not.toContain("GitHub"));
    await user.click(category(/^All/));
    await waitFor(() => expect(titles()).toContain("GitHub"));
    expect(fake.state.entries.find((e) => e.summary.id === "e-github")?.summary.inRecycleBin).toBe(false);
  });

  it("journey_history_restore", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.state.entries.push(
      makeEntry({
        id: "e-router",
        title: "Router",
        password: "current-secret",
        modified: 1_700_000_500_000,
        history: [{ modified: 1_700_000_100_000, title: "Router (first)", password: "old-secret" }],
      }),
    );
    const { user } = await unlocked(fake);
    await user.click(option(/Router/));
    await user.click(await screen.findByRole("button", { name: /^View history/ }));
    const dialog = await screen.findByRole("dialog", { name: "History" });
    await user.click(await within(dialog).findByRole("option"));
    await user.click(within(dialog).getByRole("button", { name: "Restore this version" }));
    const confirm = await screen.findByRole("dialog", { name: "Restore this version" });
    await user.click(within(confirm).getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(calls(fake, "entryHistoryRestore")[0]?.args).toEqual({ id: "e-router", index: 0 }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await screen.findByRole("heading", { name: "Router (first)" });
    await waitFor(() => expect(titles()).toContain("Router (first)"));
  });

  it("journey_open_existing_vault_from_toolbar_locks_first", async () => {
    vi.mocked(open).mockResolvedValue("/vaults/B.kdbx");
    const fake = createFakeBackend({ session: "unlocked" });
    fake.state.vaults["/vaults/B.kdbx"] = {
      entries: [makeEntry({ id: "b-1", title: "Vault B login", modified: 1 })],
      groups: fixtureGroups().filter((g) => g.isRecycleBin),
    };
    const { user } = await unlocked(fake);
    expect(fake.state.openWarnings).toEqual([]);
    // Vault A: a search, a selection, an open editor with typed text.
    const search = screen.getByRole("searchbox", { name: "Search" });
    await user.type(search, "git");
    await waitFor(() => expect(options()).toHaveLength(2));
    await user.click(option(/GitHub/));
    await user.click(await within(pane()).findByRole("button", { name: "Edit" }));
    await user.type(await screen.findByLabelText("Notes"), "typed-in-vault-a");
    await menuItem(user, "Import", "Open Existing .kdbx\u2026");
    // The editor is dirty: the toolbar asks before it locks.
    await user.click(await screen.findByRole("button", { name: "Discard" }));
    const sheet = await screen.findByRole("dialog", { name: "Open vault" });
    expect(session.status.state).toBe("locked");
    expect(body()).not.toContain("GitHub");
    expect(body()).not.toContain("typed-in-vault-a");
    await user.type(within(sheet).getByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(within(sheet).getByRole("button", { name: "Open" }));
    await waitFor(() => expect(session.status.state).toBe("unlocked"));
    expect(calls(fake, "vaultOpen")[0]?.args).toEqual({ path: "/vaults/B.kdbx", password: FIXTURE_MASTER_PASSWORD });
    await waitFor(() => expect(titles()).toEqual(["Vault B login"]));
    expect(body()).not.toContain("GitHub");
    expect(body()).not.toContain("GitLab");
    expect(body()).not.toContain("typed-in-vault-a");
    // Nothing of vault A's navigation survives into vault B.
    expect(screen.queryByRole("heading", { name: "GitHub" })).toBeNull();
    expect(navigation.selectedId).toBeNull();
    expect(navigation.editModel).toBeNull();
    expect((screen.getByRole("searchbox", { name: "Search" }) as HTMLInputElement).value).toBe("");
    expect(entries.searchText).toBe("");
  });

  it("journey_vault_switch_without_a_lock_drops_everything_of_the_old_vault", async () => {
    // The backend can swap the open vault without passing through `locked`; the window must not
    // carry the old vault's selection, revealed secret or typed text across.
    const fake = createFakeBackend({ session: "unlocked" });
    fake.state.vaults["/vaults/B.kdbx"] = {
      entries: [makeEntry({ id: "b-1", title: "Vault B login", modified: 1 })],
      groups: fixtureGroups().filter((g) => g.isRecycleBin),
    };
    const { user } = await unlocked(fake);
    await user.click(option(/GitHub/));
    await reveal(user, "Password");
    await waitFor(() => expect(body()).toContain(FIXTURE_LOGIN_PASSWORD));
    await user.click(within(pane()).getByRole("button", { name: "Edit" }));
    await user.type(await screen.findByLabelText("Notes"), "typed-in-vault-a");
    await user.type(screen.getByRole("searchbox", { name: "Search" }), "git");
    const switched = { ...fake.state.session, vaultPath: "/vaults/B.kdbx", vaultName: "B" };
    fake.state.entries = structuredClone(fake.state.vaults["/vaults/B.kdbx"]!.entries);
    fake.state.groups = structuredClone(fake.state.vaults["/vaults/B.kdbx"]!.groups);
    await fake.emit("sessionChanged", switched);
    await waitFor(() => expect(titles()).toEqual(["Vault B login"]));
    expect(body()).not.toContain("typed-in-vault-a");
    expect(body()).not.toContain(FIXTURE_LOGIN_PASSWORD);
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "GitHub" })).toBeNull();
  });

  it("journey_large_type_and_exclusive_reveal", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.state.entries.push(
      makeEntry({ id: "e-lt", title: "Wifi", password: "pw-large-1", notes: "notes-large-1", modified: 1_700_000_999_000 }),
    );
    const { user } = await unlocked(fake);
    await user.click(option(/Wifi/));
    await screen.findByRole("heading", { name: "Wifi" });
    await user.click(within(pane()).getByRole("button", { name: "Reveal notes" }));
    await waitFor(() => expect(body()).toContain("notes-large-1"));
    await reveal(user, "Password");
    await waitFor(() => expect(body()).toContain("pw-large-1"));
    expect(body()).not.toContain("notes-large-1");
    await fireEvent.contextMenu(within(pane()).getByRole("group", { name: "Password" }));
    await user.click(await screen.findByRole("menuitem", { name: "Show Large Type" }));
    const large = await screen.findByRole("dialog", { name: "Large Type" });
    await waitFor(() => expect(large.textContent).toContain("pw-large-1"));
    expect(body()).not.toContain("notes-large-1");

    await fake.emit("sessionChanged", sessionStatus("locked"));
    await screen.findByLabelText("Master password");
    expect(screen.queryByRole("dialog", { name: "Large Type" })).toBeNull();
    expect(body()).not.toContain("pw-large-1");
    expect(body()).not.toContain("notes-large-1");
  });

  it("journey_tray_recent_detail_edit_opens_main_editor", async () => {
    const { fake, user } = await unlocked();
    // Rendered beside the main window without a second `install()`, which would drop its event listeners.
    const tray = render(TrayPanel);
    const recent = await tray.findByRole("button", { name: "GitHub" });
    await user.click(recent);
    const edit = await tray.findAllByRole("button", { name: "Edit" });
    await user.click(edit[edit.length - 1] as HTMLElement);
    await waitFor(() => expect(calls(fake, "appOpenEntry")[0]?.args).toEqual({ id: "e-github", edit: true }));
    await waitFor(() => expect(navigation.editModel?.id).toBe("e-github"));
    expect(navigation.selectedId).toBe("e-github");
    await waitFor(() => expect(screen.getAllByRole("button", { name: "Save" }).length).toBeGreaterThan(0));
  });
});

