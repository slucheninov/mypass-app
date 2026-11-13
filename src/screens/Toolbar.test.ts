import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ICON_PATHS } from "../lib/ui/icons";
import { entries } from "../lib/stores/entries.svelte";
import { session } from "../lib/stores/session.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { FIXTURE_MASTER_PASSWORD } from "../test/fixtures";
import { apiError } from "../test/fakeBackend";
import { calls, renderMainApp } from "../test/mainApp";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
import { open } from "@tauri-apps/plugin-dialog";

beforeEach(() => {
  vi.mocked(open).mockReset();
});

async function unlocked(opts: Parameters<typeof renderMainApp>[0] = { session: "unlocked" }) {
  const app = await renderMainApp(opts);
  await screen.findByRole("button", { name: "Lock" });
  return app;
}

describe("search", () => {
  it("search_shortcut_mod_f_focuses_field", async () => {
    await unlocked();
    const field = screen.getByRole("searchbox", { name: "Search" });
    expect(field.getAttribute("type")).toBe("search");
    expect(document.activeElement).not.toBe(field);
    await fireEvent.keyDown(window, { key: "f", metaKey: true });
    expect(document.activeElement).toBe(field);
    (field as HTMLElement).blur();
    await fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    expect(document.activeElement).toBe(field);
  });

  it("a_plain_f_does_not_steal_focus", async () => {
    await unlocked();
    await fireEvent.keyDown(window, { key: "f" });
    expect(document.activeElement).not.toBe(screen.getByRole("searchbox"));
  });

  it("typing_updates_the_search_text_of_the_list_store", async () => {
    const user = userEvent.setup();
    await unlocked();
    await user.type(screen.getByRole("searchbox"), "git");
    expect(entries.searchText).toBe("git");
  });
});

describe("import menu", () => {
  it("import_menu_lists_csv_and_open_existing", async () => {
    const user = userEvent.setup();
    await unlocked();
    await user.click(screen.getByRole("button", { name: "Import" }));
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((i) => i.textContent?.trim())).toEqual(["Import CSV…", "Open Existing .kdbx…"]);
  });

  it("open_existing_kdbx_flushes_and_locks_then_prompts_for_password", async () => {
    vi.mocked(open).mockResolvedValue("/vaults/Other.kdbx");
    const user = userEvent.setup();
    const { fake } = await unlocked();
    await user.click(screen.getByRole("button", { name: "Import" }));
    await user.click(await screen.findByRole("menuitem", { name: "Open Existing .kdbx…" }));
    const dialog = await screen.findByRole("dialog", { name: "Open vault" });
    expect(open).toHaveBeenCalledWith(expect.objectContaining({ filters: [{ name: "KeePass", extensions: ["kdbx"] }] }));
    const order = fake.calls.map((c) => c.command);
    expect(order.indexOf("vaultOpenWarnings")).toBeGreaterThan(-1);
    expect(order.indexOf("vaultLock")).toBeGreaterThan(order.indexOf("vaultOpenWarnings"));
    expect(session.status.state).toBe("locked");
    expect(calls(fake, "vaultOpen")).toHaveLength(0);
    await user.type(within(dialog).getByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(within(dialog).getByRole("button", { name: "Open" }));
    await waitFor(() => expect(calls(fake, "vaultOpen")).toHaveLength(1));
    expect(calls(fake, "vaultOpen")[0]?.args).toEqual({ path: "/vaults/Other.kdbx", password: FIXTURE_MASTER_PASSWORD });
  });

  it("open_existing_kdbx_dialog_cancel_does_not_lock", async () => {
    vi.mocked(open).mockResolvedValue(null);
    const user = userEvent.setup();
    const { fake } = await unlocked();
    await user.click(screen.getByRole("button", { name: "Import" }));
    await user.click(await screen.findByRole("menuitem", { name: "Open Existing .kdbx…" }));
    await waitFor(() => expect(open).toHaveBeenCalled());
    expect(calls(fake, "vaultLock")).toHaveLength(0);
    expect(session.status.state).toBe("unlocked");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("cancelling_the_open_warning_keeps_the_current_vault_unlocked", async () => {
    vi.mocked(open).mockResolvedValue("/vaults/Other.kdbx");
    const user = userEvent.setup();
    const { fake } = await unlocked({ session: "unlocked" });
    fake.state.openWarnings = [{ kind: "icloud" }];
    await user.click(screen.getByRole("button", { name: "Import" }));
    await user.click(await screen.findByRole("menuitem", { name: "Open Existing .kdbx…" }));
    const warning = await screen.findByRole("dialog");
    await user.click(within(warning).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls(fake, "vaultLock")).toHaveLength(0);
    expect(session.status.state).toBe("unlocked");
  });

  it("import_csv_item_calls_the_wired_flow", async () => {
    // The CSV flow itself arrives with the import task; the shell passes its opener down.
    const user = userEvent.setup();
    await unlocked();
    await user.click(screen.getByRole("button", { name: "Import" }));
    await user.click(await screen.findByRole("menuitem", { name: "Import CSV…" }));
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("lock and settings", () => {
  it("lock_button_calls_vault_lock", async () => {
    const user = userEvent.setup();
    const { fake } = await unlocked();
    await user.click(screen.getByRole("button", { name: "Lock" }));
    await waitFor(() => expect(calls(fake, "vaultLock")).toHaveLength(1));
    // The shell gives way to the unlock screen; nothing of the vault stays in the DOM.
    await waitFor(() => expect(screen.queryByTestId("vault-shell")).toBeNull());
    expect(screen.getByLabelText("Master password")).toBeTruthy();
  });

  it("a_failed_lock_shows_a_toast", async () => {
    const user = userEvent.setup();
    const { fake } = await unlocked();
    fake.handlers.vaultLock = () => {
      throw apiError("internal", "disk on fire");
    };
    await user.click(screen.getByRole("button", { name: "Lock" }));
    await waitFor(() => expect(toasts.items).toHaveLength(1));
    expect(session.status.state).toBe("unlocked");
  });

  it("settings_gear_opens_the_settings_window", async () => {
    const user = userEvent.setup();
    const { fake } = await unlocked();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    await waitFor(() => expect(calls(fake, "appOpenSettings")).toHaveLength(1));
  });
});

describe("save indicator", () => {
  it("save_indicator_states", async () => {
    const { fake } = await unlocked();
    const indicator = screen.getByTestId("save-indicator");
    expect(indicator.textContent?.trim()).toBe("");
    // The entry list rows carry their own badges, so only the indicator is looked at.
    expect(within(indicator).queryByRole("img")).toBeNull();

    await fake.emit("saveStateChanged", { state: "pending", message: null });
    await waitFor(() => expect(within(indicator).getByText("Saving…")).toBeTruthy());

    await fake.emit("saveStateChanged", { state: "failed", message: "disk full" });
    const icon = await within(indicator).findByRole("img", { name: "Saving failed. MyPass will retry." });
    expect(icon.querySelector("path")?.getAttribute("d")).toBe(ICON_PATHS.warning);
    expect(icon.getAttribute("title")).toBe("Saving failed. MyPass will retry.");
    expect(within(indicator).queryByText("Saving…")).toBeNull();

    await fake.emit("saveStateChanged", { state: "idle", message: null });
    await waitFor(() => expect(within(indicator).queryByRole("img")).toBeNull());
    expect(indicator.textContent?.trim()).toBe("");
  });
});

describe("drag region", () => {
  it.each(["macos", "windows", "linux"])("the_toolbar_is_a_drag_region_on_%s", async (os) => {
    await unlocked({ session: "unlocked", os });
    expect(screen.getByTestId("toolbar").hasAttribute("data-tauri-drag-region")).toBe(true);
  });
});
