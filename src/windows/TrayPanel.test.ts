import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { formatDate } from "../lib/i18n/format";
import { toasts } from "../lib/stores/toasts.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { FIXTURE_LOGIN_PASSWORD, FIXTURE_MASTER_PASSWORD, makeEntry } from "../test/fixtures";
import { calls } from "../test/mainApp";
import { renderTrayPanel } from "../test/trayApp";
import type { FakeBackend } from "../test/fakeBackend";

const GITHUB_MODIFIED = 1_700_000_300_000;

async function unlockedTray(prepare?: (fake: FakeBackend) => void) {
  const app = await renderTrayPanel({}, prepare);
  await screen.findByRole("list", { name: "Recent items" });
  return { ...app, user: userEvent.setup() };
}

async function openGithub(prepare?: (fake: FakeBackend) => void) {
  const app = await unlockedTray(prepare);
  await app.user.click(await screen.findByRole("button", { name: "GitHub" }));
  const detail = await screen.findByRole("region", { name: "GitHub" });
  return { ...app, detail };
}

describe("tray panel, locked", () => {
  it("tray_locked_shows_password_field_and_unlock_calls_vault_unlock", async () => {
    const { fake } = await renderTrayPanel({ session: "locked" });
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(calls(fake, "vaultUnlock")).toHaveLength(1));
    expect(calls(fake, "vaultUnlock")[0]?.args).toEqual({ password: FIXTURE_MASTER_PASSWORD });
    expect(await screen.findByRole("list", { name: "Recent items" })).toBeTruthy();
  });

  it("a_wrong_password_shows_the_localised_error_and_stays_locked", async () => {
    await renderTrayPanel({ session: "locked" });
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText("Master password"), "nope");
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Wrong master password");
    expect(screen.queryByRole("list", { name: "Recent items" })).toBeNull();
  });

  it("tray_open_mypass_calls_app_show_main", async () => {
    const { fake } = await renderTrayPanel({ session: "locked" });
    await userEvent.setup().click(await screen.findByRole("button", { name: "Open MyPass" }));
    await waitFor(() => expect(calls(fake, "appShowMain")).toHaveLength(1));
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
    const list = screen.getByRole("list", { name: "Recent items" });
    expect(within(list).getAllByRole("button")).toHaveLength(5);
    expect(calls(fake, "entryRecent")[0]?.args).toEqual({ limit: 5 });
    // Titles only: no username line, no secret.
    expect(list.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("the_panel_never_fetches_the_entry_list", async () => {
    const { fake } = await unlockedTray();
    expect(calls(fake, "entryList")).toHaveLength(0);
    expect(calls(fake, "sidebarGet")).toHaveLength(0);
  });

  it("an_empty_vault_says_so_instead_of_an_empty_list", async () => {
    await renderTrayPanel({}, (f) => {
      f.state.entries = [];
    });
    expect(await screen.findByText("Nothing here yet")).toBeTruthy();
  });

  it("tray_lock_button_calls_vault_lock", async () => {
    const { fake, user } = await unlockedTray();
    await user.click(screen.getByRole("button", { name: "Lock" }));
    await waitFor(() => expect(calls(fake, "vaultLock")).toHaveLength(1));
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
  });

  it("tray_settings_and_quit_call_their_commands", async () => {
    const { fake, user } = await unlockedTray();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    await user.click(screen.getByRole("button", { name: "Quit MyPass" }));
    await waitFor(() => expect(calls(fake, "appOpenSettings")).toHaveLength(1));
    expect(calls(fake, "appQuit")).toHaveLength(1);
  });

  it("tray_open_mypass_calls_app_show_main_when_unlocked", async () => {
    const { fake, user } = await unlockedTray();
    await user.click(screen.getByRole("button", { name: "Open MyPass" }));
    await waitFor(() => expect(calls(fake, "appShowMain")).toHaveLength(1));
  });

  it("tray_update_row_slot_mounted_above_footer", async () => {
    await unlockedTray();
    const slot = screen.getByTestId("tray-update-row");
    expect(slot.textContent).toBe("");
    expect(slot.children).toHaveLength(0);
    const footerButton = screen.getByRole("button", { name: "Open MyPass" });
    expect(slot.compareDocumentPosition(footerButton) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("tray panel, compact detail", () => {
  it("tray_click_entry_shows_compact_detail_and_touches_access", async () => {
    const { fake, detail } = await openGithub();
    expect(calls(fake, "entryTouch")).toHaveLength(1);
    expect(calls(fake, "entryTouch")[0]?.args).toEqual({ id: "e-github" });
    expect(await within(detail).findByText("octo")).toBeTruthy();
  });

  it("tray_detail_shows_username_masked_password_website_and_modified", async () => {
    const { fake, detail } = await openGithub();
    await within(detail).findByText("octo");
    const password = within(detail).getByRole("group", { name: "Password" });
    expect(password.textContent).toContain("••••••••");
    expect(detail.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
    expect(calls(fake, "entryReveal")).toHaveLength(0);
    expect(within(detail).getByRole("group", { name: "Website" }).textContent).toContain("https://github.com");
    expect(within(detail).getByRole("group", { name: "Modified" }).textContent).toContain(formatDate(GITHUB_MODIFIED));
    expect(within(detail).getByRole("button", { name: "Edit" })).toBeTruthy();
  });

  it("tray_detail_reveal_password_then_back_clears_value", async () => {
    const { fake, detail, user } = await openGithub();
    await within(detail).findByText("octo");
    await user.click(within(within(detail).getByRole("group", { name: "Password" })).getByRole("button", { name: "Reveal" }));
    expect(await within(detail).findByText(FIXTURE_LOGIN_PASSWORD)).toBeTruthy();
    expect(calls(fake, "entryReveal")).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByRole("list", { name: "Recent items" })).toBeTruthy();
    expect(document.body.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
    // Back to the same entry: masked again.
    await user.click(screen.getByRole("button", { name: "GitHub" }));
    const again = await screen.findByRole("region", { name: "GitHub" });
    await within(again).findByText("octo");
    expect(again.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });

  it("tray_copy_password_calls_entry_copy_and_never_reveal", async () => {
    const { fake, detail, user } = await openGithub();
    await within(detail).findByText("octo");
    await user.click(within(within(detail).getByRole("group", { name: "Password" })).getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(calls(fake, "entryCopy")).toHaveLength(1));
    expect(calls(fake, "entryCopy")[0]?.args).toEqual({ id: "e-github", field: { kind: "password" } });
    expect(calls(fake, "entryReveal")).toHaveLength(0);
    expect(toasts.items.map((x) => x.kind)).toEqual(["success"]);
  });

  it("tray_copy_username_calls_entry_copy", async () => {
    const { fake, detail, user } = await openGithub();
    await within(detail).findByText("octo");
    await user.click(within(within(detail).getByRole("group", { name: "Username" })).getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(calls(fake, "entryCopy")[0]?.args).toEqual({ id: "e-github", field: { kind: "username" } }));
  });

  it("tray_detail_shows_totp_code_and_ring_only_for_totp_entries", async () => {
    const { detail, user } = await openGithub();
    const totp = await within(detail).findByRole("group", { name: "One-time code" });
    expect(await within(totp).findByText("123 456")).toBeTruthy();
    expect(within(totp).getByRole("img")).toBeTruthy();
    expect(within(totp).getByRole("button", { name: "Copy" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Back" }));
    await user.click(await screen.findByRole("button", { name: "GitLab" }));
    const gitlab = await screen.findByRole("region", { name: "GitLab" });
    await within(gitlab).findByText("octo-lab");
    expect(within(gitlab).queryByRole("group", { name: "One-time code" })).toBeNull();
    expect(within(gitlab).queryByRole("img")).toBeNull();
  });

  it("tray_website_is_text_with_copy_never_a_link", async () => {
    const { fake, detail, user } = await openGithub();
    await within(detail).findByText("octo");
    expect(within(detail).queryByRole("link")).toBeNull();
    await user.click(within(within(detail).getByRole("group", { name: "Website" })).getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(calls(fake, "entryCopy")[0]?.args).toEqual({ id: "e-github", field: { kind: "url" } }));
  });

  it.each(["http://example.com", "javascript:alert(1)"])("a_%s_website_is_plain_text_too", async (url) => {
    const { user } = await unlockedTray((f) => {
      f.state.entries.push(makeEntry({ id: "odd", title: "Odd", urls: [url], modified: 1_800_000_000_000 }));
    });
    await user.click(await screen.findByRole("button", { name: "Odd" }));
    const detail = await screen.findByRole("region", { name: "Odd" });
    expect(await within(detail).findByText(url)).toBeTruthy();
    expect(detail.querySelector("a")).toBeNull();
  });

  it("the_detail_renders_only_the_fields_present", async () => {
    const { user } = await unlockedTray((f) => {
      f.state.entries.push(makeEntry({ id: "bare", title: "Bare", username: "", password: "", modified: 1_800_000_000_000 }));
    });
    await user.click(await screen.findByRole("button", { name: "Bare" }));
    const detail = await screen.findByRole("region", { name: "Bare" });
    await within(detail).findByRole("group", { name: "Modified" });
    for (const name of ["Username", "Website", "One-time code"]) {
      expect(within(detail).queryByRole("group", { name })).toBeNull();
    }
  });

  it("tray_edit_button_calls_app_open_entry_with_edit_true", async () => {
    const { fake, detail, user } = await openGithub();
    await within(detail).findByText("octo");
    await user.click(within(detail).getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(calls(fake, "appOpenEntry")).toHaveLength(1));
    expect(calls(fake, "appOpenEntry")[0]?.args).toEqual({ id: "e-github", edit: true });
  });

  it("tray_back_returns_to_list", async () => {
    const { user } = await openGithub();
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByRole("list", { name: "Recent items" })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "GitHub" })).toBeNull();
  });

  it("tray_panel_clears_detail_on_lock_event", async () => {
    const { fake, detail, user } = await openGithub();
    await within(detail).findByText("octo");
    await user.click(within(within(detail).getByRole("group", { name: "Password" })).getByRole("button", { name: "Reveal" }));
    await within(detail).findByText(FIXTURE_LOGIN_PASSWORD);
    fake.handlers.vaultLock({});
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect(document.body.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
    expect(screen.queryByRole("region", { name: "GitHub" })).toBeNull();
    // The next unlock starts from the list, not from the old detail.
    fake.handlers.vaultUnlock({ password: FIXTURE_MASTER_PASSWORD });
    expect(await screen.findByRole("list", { name: "Recent items" })).toBeTruthy();
  });

  it("hiding_the_window_hides_the_revealed_password_and_returns_to_the_list", async () => {
    const { detail, user } = await openGithub();
    await within(detail).findByText("octo");
    await user.click(within(within(detail).getByRole("group", { name: "Password" })).getByRole("button", { name: "Reveal" }));
    await within(detail).findByText(FIXTURE_LOGIN_PASSWORD);
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    try {
      expect(await screen.findByRole("list", { name: "Recent items" })).toBeTruthy();
      expect(document.body.textContent).not.toContain(FIXTURE_LOGIN_PASSWORD);
    } finally {
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    }
  });
});

describe("tray panel, accessibility and toasts", () => {
  it("tray_and_import_pass_axe_locked_list_and_detail", async () => {
    const { container, user } = await unlockedTray();
    await expectNoAxeViolations(container);
    await user.click(screen.getByRole("button", { name: "GitHub" }));
    const detail = await screen.findByRole("region", { name: "GitHub" });
    await within(detail).findByText("123 456");
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
