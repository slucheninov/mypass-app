import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { platform } from "../lib/platform.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { session } from "../lib/stores/session.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { FIXTURE_MASTER_PASSWORD } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import { apiError } from "../test/fakeBackend";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
import { open, save } from "@tauri-apps/plugin-dialog";

const field = () => screen.getByLabelText("Master password") as HTMLInputElement;

beforeEach(() => {
  vi.mocked(open).mockReset();
  vi.mocked(save).mockReset();
});

describe("no vault", () => {
  it("no_vault_shows_create_open_and_find_actions", async () => {
    await renderMainApp({ session: "noVault" });
    expect(await screen.findByRole("button", { name: "Create vault" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open existing vault" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Find vault files" })).toBeTruthy();
  });

  it("keyboard_layout_indicator_not_shown_on_no_vault_screen", async () => {
    const { fake } = await renderMainApp({ session: "noVault", keyboardLayout: "EN" });
    await screen.findByRole("button", { name: "Create vault" });
    expect(screen.queryByTestId("keyboard-layout")).toBeNull();
    expect(calls(fake, "platformKeyboardLayout")).toHaveLength(0);
  });

  it("open_existing_uses_dialog_with_kdbx_filter_then_prompts_for_password", async () => {
    vi.mocked(open).mockResolvedValue("/vaults/Other.kdbx");
    const user = userEvent.setup();
    const { fake } = await renderMainApp({ session: "noVault" });
    await user.click(await screen.findByRole("button", { name: "Open existing vault" }));
    const dialog = await screen.findByRole("dialog", { name: "Open vault" });
    expect(open).toHaveBeenCalledWith(
      expect.objectContaining({ filters: [{ name: "KeePass", extensions: ["kdbx"] }] }),
    );
    expect(within(dialog).getByText("Other.kdbx")).toBeTruthy();
    expect(calls(fake, "vaultOpen")).toHaveLength(0);
    await user.type(within(dialog).getByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(within(dialog).getByRole("button", { name: "Open" }));
    await waitFor(() => expect(calls(fake, "vaultOpen")).toHaveLength(1));
    expect(calls(fake, "vaultOpen")[0]?.args).toEqual({ path: "/vaults/Other.kdbx", password: FIXTURE_MASTER_PASSWORD });
  });

  it("a_cancelled_file_dialog_does_nothing", async () => {
    vi.mocked(open).mockResolvedValue(null);
    const user = userEvent.setup();
    const { fake } = await renderMainApp({ session: "noVault" });
    await user.click(await screen.findByRole("button", { name: "Open existing vault" }));
    await waitFor(() => expect(open).toHaveBeenCalled());
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls(fake, "vaultOpenWarnings")).toHaveLength(0);
  });
});

describe("locked", () => {
  it("locked_screen_shows_file_name_and_connected_status_card", async () => {
    await renderMainApp({ session: "locked" });
    expect(await screen.findByText("test.kdbx")).toBeTruthy();
    expect(screen.getByText("/vaults")).toBeTruthy();
    expect(screen.getByText("4 KB")).toBeTruthy();
    expect(screen.getByText(/^Modified /)).toBeTruthy();
    const dot = screen.getByTestId("vault-status");
    expect(dot.getAttribute("data-status")).toBe("connected");
    expect(within(dot).getByText("Connected")).toBeTruthy();
  });

  it("missing_file_status_shows_warning_and_disables_unlock", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" }, (fake) => {
      fake.state.session = { ...fake.state.session, fileStatus: { kind: "missing" } };
    });
    const status = await screen.findByTestId("vault-status");
    expect(status.getAttribute("data-status")).toBe("missing");
    expect(within(status).getByText("File not found")).toBeTruthy();
    await user.type(field(), "pw");
    expect((screen.getByRole("button", { name: "Unlock" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("unreadable_file_status_is_labelled", async () => {
    await renderMainApp({ session: "locked" }, (fake) => {
      fake.state.session = { ...fake.state.session, fileStatus: { kind: "unreadable", message: "denied" } };
    });
    const status = await screen.findByTestId("vault-status");
    expect(status.getAttribute("data-status")).toBe("unreadable");
    expect(within(status).getByText("File is not readable")).toBeTruthy();
  });

  it("unlock_button_disabled_when_password_empty", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" });
    await screen.findByLabelText("Master password");
    const button = screen.getByRole("button", { name: "Unlock" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    await user.type(field(), "x");
    expect(button.disabled).toBe(false);
    await user.clear(field());
    expect(button.disabled).toBe(true);
  });

  it("password_field_is_focused_on_arrival", async () => {
    await renderMainApp({ session: "locked" });
    await waitFor(() => expect(document.activeElement).toBe(field()));
  });

  it("enter_key_submits_unlock", async () => {
    const user = userEvent.setup();
    const { fake } = await renderMainApp({ session: "locked" });
    await screen.findByLabelText("Master password");
    await user.type(field(), "pw{Enter}");
    await waitFor(() => expect(calls(fake, "vaultUnlock")).toHaveLength(1));
    expect(calls(fake, "vaultUnlock")[0]?.args).toEqual({ password: "pw" });
  });

  it("wrong_password_shows_message_clears_field_and_refocuses", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" });
    await screen.findByLabelText("Master password");
    await user.type(field(), "nope{Enter}");
    expect(await screen.findByText("Wrong master password")).toBeTruthy();
    expect(field().value).toBe("");
    await waitFor(() => expect(document.activeElement).toBe(field()));
    expect((screen.getByRole("button", { name: "Unlock" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("another_backend_error_is_shown_by_code", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" }, (fake) => {
      fake.handlers.vaultUnlock = () => {
        throw apiError("corrupt");
      };
    });
    await screen.findByLabelText("Master password");
    await user.type(field(), "pw{Enter}");
    expect(await screen.findByText("The vault file looks damaged")).toBeTruthy();
  });

  it("unreadable_file_still_allows_unlock_attempt", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" }, (fake) => {
      fake.state.session = { ...fake.state.session, fileStatus: { kind: "unreadable", message: "denied" } };
    });
    await screen.findByTestId("vault-status");
    await user.type(field(), "pw");
    expect((screen.getByRole("button", { name: "Unlock" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("field_is_emptied_at_submit_while_the_backend_is_still_working", async () => {
    const user = userEvent.setup();
    let release: () => void = () => {};
    await renderMainApp({ session: "locked" }, (fake) => {
      fake.handlers.vaultUnlock = () => new Promise((_, reject) => (release = () => reject(apiError("wrongPassword"))));
    });
    await screen.findByLabelText("Master password");
    await user.type(field(), "secret-pw{Enter}");
    expect(field().value).toBe("");
    release();
    await screen.findByText("Wrong master password");
  });

  it("password_not_kept_after_success_and_not_in_store", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" });
    await screen.findByLabelText("Master password");
    await user.type(field(), FIXTURE_MASTER_PASSWORD);
    await user.click(screen.getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(screen.queryByLabelText("Master password")).toBeNull());
    expect(session.status.state).toBe("unlocked");
    expect(Object.keys(session.status).some((k) => /pass|secret/i.test(k))).toBe(false);
    expect(JSON.stringify(session.status)).not.toContain(FIXTURE_MASTER_PASSWORD);
    expect(JSON.stringify(toasts.items)).not.toContain(FIXTURE_MASTER_PASSWORD);
    expect(JSON.stringify([session.status, session.pendingOpenPath, session.loaded])).not.toContain(FIXTURE_MASTER_PASSWORD);
  });

  it("reveal_does_not_survive_a_failed_attempt", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" });
    await screen.findByLabelText("Master password");
    await user.click(screen.getByRole("button", { name: "Show password" }));
    expect(field().type).toBe("text");
    await user.type(field(), "nope{Enter}");
    await screen.findByText("Wrong master password");
    expect(field().type).toBe("password");
  });

  it("show_hide_password_toggle_switches_input_type", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" });
    await screen.findByLabelText("Master password");
    expect(field().type).toBe("password");
    await user.click(screen.getByRole("button", { name: "Show password" }));
    expect(field().type).toBe("text");
    await user.click(screen.getByRole("button", { name: "Hide password" }));
    expect(field().type).toBe("password");
  });

  it("unlock_extras_slot_mounted_with_availability_flag", async () => {
    try {
      for (const available of [false, true]) {
        const { unmount } = await renderMainApp({ session: "locked", deviceUnlockAvailable: available });
        const slot = await screen.findByTestId("unlock-extras");
        expect(slot.getAttribute("data-available")).toBe(String(available));
        expect(within(slot).queryByRole("button")).toBeNull();
        unmount();
      }
    } finally {
      platform.deviceUnlockAvailable = false;
    }
  });
});

describe("recent vaults", () => {
  const recents = [
    { path: "/vaults/test.kdbx", fileName: "test.kdbx", exists: true },
    { path: "/other/Family.kdbx", fileName: "Family.kdbx", exists: true },
    { path: "/gone/Old.kdbx", fileName: "Old.kdbx", exists: false },
  ];

  it("recent_vaults_switch_vault_calls_vault_open_flow_with_that_path", async () => {
    const user = userEvent.setup();
    const { fake } = await renderMainApp({ session: "locked" }, (f) => (f.state.recents = structuredClone(recents)));
    const list = await screen.findByRole("list", { name: "Recent vaults" });
    // The vault on the screen is not offered again.
    expect(within(list).queryByText("test.kdbx")).toBeNull();
    await user.click(within(list).getByRole("button", { name: /Family\.kdbx/ }));
    const dialog = await screen.findByRole("dialog", { name: "Open vault" });
    await user.type(within(dialog).getByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(within(dialog).getByRole("button", { name: "Open" }));
    await waitFor(() => expect(calls(fake, "vaultOpen")).toHaveLength(1));
    expect(calls(fake, "vaultOpen")[0]?.args.path).toBe("/other/Family.kdbx");
  });

  it("a_missing_recent_is_marked_and_cannot_be_opened", async () => {
    await renderMainApp({ session: "locked" }, (f) => (f.state.recents = structuredClone(recents)));
    const list = await screen.findByRole("list", { name: "Recent vaults" });
    const item = within(list).getByText("Old.kdbx").closest("li") as HTMLElement;
    expect(within(item).getByText("File not found")).toBeTruthy();
    expect(within(item).queryByRole("button", { name: /Old\.kdbx/ })).toBeNull();
    expect(within(item).getByRole("button", { name: "Remove from list" })).toBeTruthy();
  });

  it("forget_recent_calls_vault_forget", async () => {
    const user = userEvent.setup();
    const { fake } = await renderMainApp({ session: "locked" }, (f) => (f.state.recents = structuredClone(recents)));
    const list = await screen.findByRole("list", { name: "Recent vaults" });
    const item = within(list).getByText("Family.kdbx").closest("li") as HTMLElement;
    await user.click(within(item).getByRole("button", { name: "Remove from list" }));
    await waitFor(() => expect(calls(fake, "vaultForget")).toHaveLength(1));
    expect(calls(fake, "vaultForget")[0]?.args).toEqual({ path: "/other/Family.kdbx" });
    await waitFor(() => expect(within(list).queryByText("Family.kdbx")).toBeNull());
  });

  it("open_another_vault_uses_the_file_dialog", async () => {
    vi.mocked(open).mockResolvedValue("/vaults/Next.kdbx");
    const user = userEvent.setup();
    await renderMainApp({ session: "locked" });
    await user.click(await screen.findByRole("button", { name: "Open another vault…" }));
    const dialog = await screen.findByRole("dialog", { name: "Open vault" });
    expect(within(dialog).getByText("Next.kdbx")).toBeTruthy();
  });
});

describe("keyboard layout indicator", () => {
  afterEach(() => vi.useRealTimers());

  it("keyboard_layout_indicator_shows_current_layout_name", async () => {
    await renderMainApp({ session: "locked", keyboardLayout: "UK" });
    const code = await screen.findByText("UK");
    expect(code.closest("[aria-live]")?.getAttribute("aria-live")).toBe("polite");
    expect(screen.getByText("Keyboard layout: UK")).toBeTruthy();
  });

  it("keyboard_layout_indicator_hidden_when_unsupported", async () => {
    const { fake } = await renderMainApp({ session: "locked", keyboardLayout: null });
    await screen.findByLabelText("Master password");
    await waitFor(() => expect(calls(fake, "platformKeyboardLayout").length).toBeGreaterThan(0));
    expect(screen.queryByTestId("keyboard-layout")).toBeNull();
  });

  it("keyboard_layout_refreshes_on_window_focus", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    let layout = "EN";
    const { fake } = await renderMainApp({ session: "locked" }, (f) => {
      f.handlers.platformKeyboardLayout = () => layout;
    });
    await screen.findByText("EN");
    const reads = () => calls(fake, "platformKeyboardLayout").length;
    expect(reads()).toBe(1);
    layout = "RU";
    // Inside the 500 ms window of the first read nothing is asked again.
    window.dispatchEvent(new Event("focus"));
    expect(reads()).toBe(1);
    vi.advanceTimersByTime(500);
    window.dispatchEvent(new Event("focus"));
    window.dispatchEvent(new Event("focus"));
    expect(await screen.findByText("RU")).toBeTruthy();
    expect(reads()).toBe(2);
  });

  it("keyboard_layout_refreshes_on_non_printable_keys_only", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    const { fake } = await renderMainApp({ session: "locked", keyboardLayout: "EN" });
    await screen.findByText("EN");
    vi.advanceTimersByTime(500);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
    expect(calls(fake, "platformKeyboardLayout")).toHaveLength(1);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "CapsLock" }));
    await waitFor(() => expect(calls(fake, "platformKeyboardLayout")).toHaveLength(2));
  });
});

describe("keyboard layout after a switch key", () => {
  afterEach(() => vi.useRealTimers());

  it("keyboard_layout_badge_shows_the_layout_that_took_effect_after_the_key_event", async () => {
    vi.useFakeTimers({ toFake: ["Date", "setTimeout", "clearTimeout"] });
    let layout = "EN";
    await renderMainApp({ session: "locked" }, (f) => {
      f.handlers.platformKeyboardLayout = () => layout;
    });
    await screen.findByText("EN");
    // The key arrives inside the throttle window; the OS applies the switch a moment later.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "CapsLock" }));
    setTimeout(() => (layout = "UK"), 50);
    await vi.advanceTimersByTimeAsync(200);
    expect(await screen.findByText("UK")).toBeTruthy();
  });
});

describe("accessibility", () => {
  it("screens_pass_axe", async () => {
    const first = await renderMainApp({ session: "noVault", keyboardLayout: "EN" });
    await screen.findByRole("button", { name: "Create vault" });
    await expectNoAxeViolations(first.container);
    first.unmount();
    const locked = await renderMainApp({ session: "locked", keyboardLayout: "EN" }, (f) => {
      f.state.recents = [{ path: "/other/Family.kdbx", fileName: "Family.kdbx", exists: true }];
    });
    await screen.findByLabelText("Master password");
    await screen.findByRole("list", { name: "Recent vaults" });
    await expectNoAxeViolations(locked.container);
  });
});
