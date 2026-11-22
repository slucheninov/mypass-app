import { act, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { formatShortcut } from "../lib/shortcuts";
import { toasts } from "../lib/stores/toasts.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { FIXTURE_LOGIN_PASSWORD, FIXTURE_MASTER_PASSWORD, makeEntry } from "../test/fixtures";
import { calls } from "../test/mainApp";
import { sourceFiles } from "../test/sourceScan";
import { renderTrayPanel } from "../test/trayApp";
import { apiError, type FakeBackend } from "../test/fakeBackend";

async function unlockedTray(prepare?: (fake: FakeBackend) => void) {
  const app = await renderTrayPanel({}, prepare);
  await screen.findByRole("group", { name: "Recent" });
  return { ...app, user: userEvent.setup() };
}

async function fiveRecents() {
  return unlockedTray((fake) => {
    fake.state.entries = fake.state.entries.filter((entry) => entry.summary.title.startsWith("Git"));
    for (const n of [1, 2, 3, 4]) {
      fake.state.entries.push(makeEntry({ id: `extra-${n}`, title: `Extra ${n}`, modified: 1_600_000_000_000 + n }));
    }
  });
}

const panelSource = sourceFiles(["svelte"]).find((file) => file.path.endsWith("/TrayPanel.svelte"))?.text ?? "";

async function openActions(entry = "GitHub") {
  const app = await unlockedTray();
  await app.user.click(await screen.findByRole("menuitem", { name: entry }));
  return app;
}

describe("tray panel, locked", () => {
  it("tray_locked_shows_password_field_and_unlock_calls_vault_unlock", async () => {
    const { fake } = await renderTrayPanel({ session: "locked" });
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(calls(fake, "vaultUnlock")).toHaveLength(1));
    expect(calls(fake, "vaultUnlock")[0]?.args).toEqual({ password: FIXTURE_MASTER_PASSWORD });
    expect(await screen.findByRole("group", { name: "Recent" })).toBeTruthy();
  });

  it("a_wrong_password_shows_the_localised_error_and_stays_locked", async () => {
    await renderTrayPanel({ session: "locked" });
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText("Master password"), "nope");
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Wrong master password");
    expect(screen.queryByRole("group", { name: "Recent" })).toBeNull();
  });

  it("the_locked_panel_shows_the_main_unlock_card_and_only_settings_and_quit", async () => {
    const { fake } = await renderTrayPanel({ session: "locked" });
    await screen.findByLabelText("Master password");
    const vault = fake.state.session.vaultPath ?? "";
    expect(screen.getByRole("heading", { name: vault.split("/").pop() })).toBeTruthy();
    expect(screen.getByTestId("vault-status").dataset.status).toBe("connected");
    expect(screen.getByRole("button", { name: "Show password" })).toBeTruthy();
    expect(screen.getAllByRole("menuitem").map((item) => item.getAttribute("aria-label"))).toEqual(["Settings…", "Quit"]);
    expect(screen.queryByRole("menuitem", { name: "Lock" })).toBeNull();
    const user = userEvent.setup();
    await user.click(screen.getByRole("menuitem", { name: "Settings…" }));
    await user.click(screen.getByRole("menuitem", { name: "Quit" }));
    await waitFor(() => expect(calls(fake, "appOpenSettings")).toHaveLength(1));
    expect(calls(fake, "appQuit")).toHaveLength(1);
  });

  it("tray_no_vault_shows_no_password_field", async () => {
    const { fake } = await renderTrayPanel({ session: "noVault" });
    expect(await screen.findByText("No vault is open")).toBeTruthy();
    expect(screen.queryByLabelText("Master password")).toBeNull();
    expect(screen.queryByRole("button", { name: "Unlock" })).toBeNull();
    expect(screen.getAllByRole("menuitem").map((item) => item.getAttribute("aria-label"))).toEqual(["Open MyPass", "Settings…", "Quit"]);
    await userEvent.setup().click(screen.getByRole("menuitem", { name: "Open MyPass" }));
    await waitFor(() => expect(calls(fake, "appShowMain")).toHaveLength(1));
  });

  it("hiding_the_window_empties_a_half_typed_password", async () => {
    await renderTrayPanel({ session: "locked" });
    const user = userEvent.setup();
    const field = (await screen.findByLabelText("Master password")) as HTMLInputElement;
    await user.type(field, "half-typed");
    expect((screen.getByRole("button", { name: "Unlock" }) as HTMLButtonElement).disabled).toBe(false);
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    try {
      await waitFor(() => expect(field.value).toBe(""));
      expect((screen.getByRole("button", { name: "Unlock" }) as HTMLButtonElement).disabled).toBe(true);
    } finally {
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    }
  });

  it("the_locked_panel_lists_no_entries", async () => {
    const { fake } = await renderTrayPanel({ session: "locked" });
    await screen.findByLabelText("Master password");
    expect(calls(fake, "entryRecent")).toHaveLength(0);
  });
});

describe("tray panel, unlocked list", () => {
  it("tray_unlocked_lists_five_recents", async () => {
    const { fake } = await unlockedTray((f) => {
      for (const n of [1, 2, 3, 4]) {
        f.state.entries.push(makeEntry({ id: `extra-${n}`, title: `Extra ${n}`, modified: 1_600_000_000_000 + n }));
      }
    });
    const list = screen.getByRole("group", { name: "Recent" });
    expect(within(list).getAllByRole("menuitem")).toHaveLength(5);
    expect(calls(fake, "entryRecent")[0]?.args).toEqual({ limit: 5 });
    // Titles only: no username line, no secret.
    expect(list.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("the_panel_has_no_search_field", async () => {
    await unlockedTray();
    expect(screen.queryByRole("searchbox")).toBeNull();
  });

  it("the_panel_never_fetches_the_general_entry_list", async () => {
    const { fake } = await unlockedTray();
    expect(calls(fake, "entryList")).toHaveLength(0);
    expect(calls(fake, "sidebarGet")).toHaveLength(0);
  });

  it("each_recent_row_shows_the_letter_avatar_title_and_user_name", async () => {
    await unlockedTray();
    const row = screen.getByRole("menuitem", { name: "GitHub" });
    expect(row.querySelector(".avatar")?.textContent).toBe("G");
    expect(row.textContent).toContain("octo");
    expect(row.getAttribute("aria-describedby")).toBeTruthy();
  });

  it("an_empty_vault_says_so_instead_of_an_empty_list", async () => {
    await renderTrayPanel({}, (f) => {
      f.state.entries = [];
    });
    expect(await screen.findByText("Nothing here yet")).toBeTruthy();
  });

  it("tray_lock_button_calls_vault_lock", async () => {
    const { fake, user } = await unlockedTray();
    await user.click(screen.getByRole("menuitem", { name: "Lock" }));
    await waitFor(() => expect(calls(fake, "vaultLock")).toHaveLength(1));
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
  });

  it("the_unlocked_panel_lists_recents_then_open_lock_settings_quit", async () => {
    await fiveRecents();
    const menu = screen.getByRole("menu", { name: "MyPass" });
    const names = within(menu).getAllByRole("menuitem").map((el) =>
      el.getAttribute("aria-label"),
    );
    expect(names.slice(0, 5)).toEqual(["GitHub", "GitLab", "Extra 4", "Extra 3", "Extra 2"]);
    expect(names.slice(5)).toEqual(["Open MyPass", "Lock", "Settings…", "Quit"]);
    expect(screen.getByText("Recent")).toBeTruthy();
  });

  it.each([
    ["macos", "⌘,", "⌘Q"],
    ["windows", "Ctrl+,", "Ctrl+Q"],
  ])("only real shortcuts are shown on %s", async (os, settingsKey, quitKey) => {
    await renderTrayPanel({ os }, (fake) => { fake.state.settings.shortcutLock = ""; });
    expect(screen.getByRole("menuitem", { name: "Lock" }).querySelector(".shortcut")).toBeNull();
    expect(screen.getByRole("menuitem", { name: "Settings…" }).textContent).toContain(settingsKey);
    expect(screen.getByRole("menuitem", { name: "Quit" }).textContent).toContain(quitKey);
  });

  it("arrow keys wrap and Enter activates the highlighted row", async () => {
    const { user, fake } = await fiveRecents();
    await user.keyboard("{ArrowDown}{ArrowUp}");
    expect(screen.getByRole("menuitem", { name: "Quit" }).hasAttribute("data-active")).toBe(true);
    await user.keyboard("{ArrowDown>6/}");
    expect(screen.getByRole("menuitem", { name: "Open MyPass" }).hasAttribute("data-active")).toBe(true);
    await user.keyboard("{Enter}");
    expect(calls(fake, "appShowMain")).toHaveLength(1);
  });

  it("the highlight clamps within recents when the list shrinks", async () => {
    const { user, fake } = await fiveRecents();
    await user.keyboard("{ArrowDown>5/}");
    fake.state.entries = fake.state.entries.filter((entry) => !entry.summary.id.startsWith("extra-"));
    await fake.emit("vaultChanged", null);
    await waitFor(() => expect(screen.queryByRole("menuitem", { name: "Extra 2" })).toBeNull());
    expect(screen.getByRole("menuitem", { name: "GitLab" }).hasAttribute("data-active")).toBe(true);
    await user.keyboard("{Enter}");
    expect(calls(fake, "entryTouch")[0]?.args).toEqual({
      id: fake.state.entries.find((entry) => entry.summary.title === "GitLab")?.summary.id,
    });
  });

  it("Cmd+, opens settings, Cmd+Q quits and Escape hides", async () => {
    const { user, fake } = await unlockedTray();
    await user.keyboard("{Meta>},{/Meta}{Meta>}q{/Meta}{Escape}");
    expect(calls(fake, "appOpenSettings")).toHaveLength(1);
    expect(calls(fake, "appQuit")).toHaveLength(1);
    expect(calls(fake, "trayPanelHide")).toHaveLength(1);
  });

  it("the panel fits its height and applies the returned material", async () => {
    const { fake } = await unlockedTray((f) => { f.handlers.trayPanelFit = () => "solid"; });
    await waitFor(() => {
      expect(calls(fake, "trayPanelFit")[0]?.args.height).toBeGreaterThan(0);
      expect(document.documentElement.dataset.material).toBe("solid");
    });
  });

  it("the panel uses an opaque solid fallback, system font and system accent", () => {
    expect(panelSource).toMatch(/html\[data-material="solid"\][^{]+\{[^}]*background: Canvas;/);
    expect(panelSource).toContain("font: 13px system-ui");
    expect(panelSource).toContain("AccentColor");
    expect(panelSource).toContain("#0a64d6");
  });

  it("the_menu_shows_the_configured_global_shortcuts_beside_open_and_lock", async () => {
    await unlockedTray((f) => {
      f.state.settings.shortcutLock = "CommandOrControl+Shift+L";
      f.state.settings.shortcutShow = "";
    });
    const lock = screen.getByRole("menuitem", { name: "Lock" });
    expect(lock.querySelector(".shortcut")?.textContent).toBe(formatShortcut("CommandOrControl+Shift+L", "macos"));
    expect(screen.getByRole("menuitem", { name: "Open MyPass" }).querySelector(".shortcut")).toBeNull();
  });

  it("tray_settings_and_quit_call_their_commands", async () => {
    const { fake, user } = await unlockedTray();
    await user.click(screen.getByRole("menuitem", { name: "Settings…" }));
    await user.click(screen.getByRole("menuitem", { name: "Quit" }));
    await waitFor(() => expect(calls(fake, "appOpenSettings")).toHaveLength(1));
    expect(calls(fake, "appQuit")).toHaveLength(1);
  });

  it("tray_open_mypass_calls_app_show_main_when_unlocked", async () => {
    const { fake, user } = await unlockedTray();
    await user.click(screen.getByRole("menuitem", { name: "Open MyPass" }));
    await waitFor(() => expect(calls(fake, "appShowMain")).toHaveLength(1));
  });

  it("the_update_row_appears_above_open_mypass_and_opens_the_release", async () => {
    const { fake, user } = await unlockedTray();
    const slot = screen.getByTestId("tray-update-row");
    expect(slot.textContent).toBe("");
    expect(slot.children).toHaveLength(0);
    await fake.emit("updateChanged", {
      status: { state: "idle" },
      banner: { version: "10.0.0", url: "https://github.com/release" },
    });
    const row = await within(slot).findByRole("menuitem", { name: "MyPass 10.0.0 available…" });
    await user.click(row);
    expect(calls(fake, "updateOpenRelease")).toHaveLength(1);
    const footerButton = screen.getByRole("menuitem", { name: "Open MyPass" });
    expect(slot.compareDocumentPosition(footerButton) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("tray panel, entry actions", () => {
  it("a_save_state_change_does_not_refetch_the_recents", async () => {
    const { fake } = await unlockedTray();
    expect(calls(fake, "entryRecent")).toHaveLength(1);
    await fake.emit("saveStateChanged", { state: "saving", message: null });
    await fake.emit("saveStateChanged", { state: "idle", message: null });
    await new Promise((r) => setTimeout(r, 50));
    expect(calls(fake, "entryRecent")).toHaveLength(1);
  });

  it("clicking_an_entry_replaces_the_list_with_its_actions", async () => {
    const { fake } = await openActions();
    const menu = await screen.findByRole("menu", { name: "GitHub" });
    expect(within(menu).getAllByRole("menuitem").map((row) => row.getAttribute("aria-label")))
      .toEqual(["Back", "Copy user name", "Copy password", "Copy code"]);
    expect(screen.queryByText("Recent")).toBeNull();
    expect(calls(fake, "entryTouch").map((call) => call.args)).toEqual([{ id: "e-github" }]);
  });

  it("copy_code_shows_only_for_totp_entries", async () => {
    await openActions("GitLab");
    await screen.findByRole("menuitem", { name: "Copy password" });
    expect(screen.queryByRole("menuitem", { name: "Copy code" })).toBeNull();
  });

  it.each([
    ["Copy user name", "username"],
    ["Copy password", "password"],
    ["Copy code", "totp"],
  ])("%s copies through Rust and hides the panel", async (name, kind) => {
    const { fake, user } = await openActions();
    await user.click(await screen.findByRole("menuitem", { name }));
    expect(calls(fake, "entryCopy").map((call) => call.args)).toEqual([{ id: "e-github", field: { kind } }]);
    await screen.findByRole("group", { name: "Recent" });
    expect(calls(fake, "trayPanelHide")).toHaveLength(1);
    for (const command of ["entryGet", "entryReveal", "entryTotp"] as const) {
      expect(calls(fake, command)).toHaveLength(0);
    }
  });

  it("a_copy_finishing_after_blur_does_not_hide_the_reopened_action_view", async () => {
    const { fake, user } = await openActions();
    let finish = () => {};
    const pending = new Promise<null>((resolve) => {
      finish = () => resolve(null);
    });
    fake.handlers.entryCopy = () => pending;
    await user.click(await screen.findByRole("menuitem", { name: "Copy password" }));
    expect(calls(fake, "entryCopy")).toHaveLength(1);

    window.dispatchEvent(new Event("blur"));
    await user.click(await screen.findByRole("menuitem", { name: "GitHub" }));
    await screen.findByRole("menu", { name: "GitHub" });
    await act(async () => {
      finish();
      await pending;
    });
    expect(screen.getByRole("menu", { name: "GitHub" })).toBeTruthy();
    expect(calls(fake, "trayPanelHide")).toHaveLength(0);
  });

  it("a_copy_failure_reports_the_error_and_keeps_the_actions_open", async () => {
    const { fake, user } = await openActions();
    fake.handlers.entryCopy = () => { throw apiError("notFound"); };
    await user.click(await screen.findByRole("menuitem", { name: "Copy password" }));
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("menu", { name: "GitHub" })).toBeTruthy();
    expect(calls(fake, "trayPanelHide")).toHaveLength(0);
  });

  it("right_arrow_opens_and_left_or_escape_goes_back", async () => {
    const { fake, user } = await unlockedTray();
    await user.keyboard("{ArrowDown}{ArrowRight}");
    await screen.findByRole("menu", { name: "GitHub" });
    await user.keyboard("{ArrowLeft}");
    await screen.findByRole("group", { name: "Recent" });
    await user.keyboard("{ArrowRight}");
    await screen.findByRole("menu", { name: "GitHub" });
    await user.keyboard("{Escape}");
    await screen.findByRole("group", { name: "Recent" });
    expect(calls(fake, "trayPanelHide")).toHaveLength(0);
  });

  it("a_lock_closes_the_action_view", async () => {
    const { fake } = await openActions();
    await screen.findByRole("menuitem", { name: "Copy password" });
    fake.handlers.vaultLock({});
    await screen.findByLabelText("Master password");
    expect(screen.queryByRole("menuitem", { name: "Copy password" })).toBeNull();
  });

  it("a_window_blur_closes_the_action_view", async () => {
    await openActions();
    await screen.findByRole("menuitem", { name: "Copy password" });
    window.dispatchEvent(new Event("blur"));
    await screen.findByRole("group", { name: "Recent" });
    expect(screen.queryByRole("menuitem", { name: "Copy password" })).toBeNull();
  });
});

describe("tray panel, hide without visibilitychange", () => {
  it("a_window_blur_empties_a_half_typed_master_password", async () => {
    const user = userEvent.setup();
    await renderTrayPanel({ session: "locked" });
    const input = (await screen.findByLabelText("Master password")) as HTMLInputElement;
    await user.type(input, "half-typed");
    window.dispatchEvent(new Event("blur"));
    await waitFor(() => expect(input.value).toBe(""));
  });
});

describe("tray panel, vault switch while unlocked", () => {
  it("a_switch_closes_the_action_view_and_loads_the_new_recents", async () => {
    const { fake } = await openActions();
    await screen.findByRole("menuitem", { name: "Copy password" });
    fake.state.session = { ...fake.state.session, vaultPath: "/vaults/Other.kdbx", vaultName: "Other" };
    await fake.emit("sessionChanged", fake.state.session);
    expect(await screen.findByRole("group", { name: "Recent" })).toBeTruthy();
    expect(screen.queryByRole("menu", { name: "GitHub" })).toBeNull();
    expect(calls(fake, "entryRecent").length).toBeGreaterThanOrEqual(2);
  });
});

describe("tray panel, accessibility and toasts", () => {
  it("tray_list_and_entry_actions_pass_axe", async () => {
    const { container, user } = await unlockedTray();
    await expectNoAxeViolations(container);
    await user.click(screen.getByRole("menuitem", { name: "GitHub" }));
    await screen.findByRole("menu", { name: "GitHub" });
    await expectNoAxeViolations(container);
  });

  it("tray_locked_passes_axe", async () => {
    const { container } = await renderTrayPanel({ session: "locked" });
    await screen.findByLabelText("Master password");
    await expectNoAxeViolations(container);
  });

  it("the_toast_host_is_mounted_in_the_panel", async () => {
    await unlockedTray();
    toasts.push("info", "Hello");
    expect(await screen.findByRole("region", { name: "Notifications" })).toBeTruthy();
    expect((await screen.findAllByRole("status")).some((el) => el.textContent?.includes("Hello"))).toBe(true);
  });

  it("the_panel_does_not_toast_a_failed_save", async () => {
    const { fake } = await unlockedTray();
    await fake.emit("saveStateChanged", { state: "failed", message: "disk is full" });
    await new Promise((r) => setTimeout(r, 50));
    expect(toasts.items).toHaveLength(0);
  });
});
