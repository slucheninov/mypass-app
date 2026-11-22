import { screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadPlatform } from "../lib/platform.svelte";
import { session } from "../lib/stores/session.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { createFakeBackend } from "../test/fakeBackend";
import { FIXTURE_MASTER_PASSWORD } from "../test/fixtures";
import { calls } from "../test/mainApp";
import { renderWithBackend } from "../test/render";
import OpenVaultSheet from "./OpenVaultSheet.svelte";

afterEach(() => session.stop());

const PATH = "/vaults/Family.kdbx";

describe("OpenVaultSheet", () => {
  it("open_vault_sheet_calls_vault_open_with_path_and_password", async () => {
    const user = userEvent.setup();
    const onclose = vi.fn();
    const fake = createFakeBackend({ session: "noVault" });
    renderWithBackend(OpenVaultSheet, { path: PATH, onclose }, fake);
    expect(await screen.findByRole("dialog", { name: "Open vault" })).toBeTruthy();
    expect(screen.getByText("Family.kdbx")).toBeTruthy();
    const input = screen.getByLabelText("Master password") as HTMLInputElement;
    await waitFor(() => expect(document.activeElement).toBe(input));
    await user.type(input, `${FIXTURE_MASTER_PASSWORD}{Enter}`);
    await waitFor(() => expect(onclose).toHaveBeenCalledTimes(1));
    expect(calls(fake, "vaultOpen")[0]?.args).toEqual({ path: PATH, password: FIXTURE_MASTER_PASSWORD });
    expect(session.status.state).toBe("unlocked");
    expect(session.status.vaultPath).toBe(PATH);
  });

  it("open_vault_sheet_wrong_password_clears_field_and_keeps_sheet", async () => {
    const user = userEvent.setup();
    const onclose = vi.fn();
    renderWithBackend(OpenVaultSheet, { path: PATH, onclose }, createFakeBackend({ session: "noVault" }));
    const input = (await screen.findByLabelText("Master password")) as HTMLInputElement;
    await user.type(input, "nope");
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(await screen.findByText("Wrong master password")).toBeTruthy();
    expect(input.value).toBe("");
    await waitFor(() => expect(document.activeElement).toBe(input));
    expect(onclose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Open vault" })).toBeTruthy();
  });

  it("open_button_needs_a_password_and_cancel_closes", async () => {
    const user = userEvent.setup();
    const onclose = vi.fn();
    const fake = createFakeBackend({ session: "noVault" });
    renderWithBackend(OpenVaultSheet, { path: PATH, onclose }, fake);
    const open = (await screen.findByRole("button", { name: "Open" })) as HTMLButtonElement;
    expect(open.disabled).toBe(true);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onclose).toHaveBeenCalledTimes(1);
    expect(calls(fake, "vaultOpen")).toHaveLength(0);
  });

  it("open_vault_sheet_shows_the_short_location_in_one_line_with_the_full_path_as_tooltip", async () => {
    const long = "/Users/someone/Library/Mobile Documents/com~apple~CloudDocs/Vaults/Family.kdbx";
    renderWithBackend(OpenVaultSheet, { path: long, onclose: () => {} }, createFakeBackend({ session: "noVault" }));
    await loadPlatform();
    const dialog = await screen.findByRole("dialog", { name: "Open vault" });
    const chip = dialog.querySelector(".folder .location");
    expect(chip?.getAttribute("title")).toBe(long);
    expect(chip?.querySelector('[aria-hidden="true"]')?.textContent).toBe("iCloud Drive/Vaults");
    // The file name is the subtitle of the header, next to the decorative vault tile.
    expect(dialog.querySelector(".subtitle")?.textContent).toBe("Family.kdbx");
    expect(dialog.querySelector(".tile svg")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("open_button_is_disabled_with_a_spinner_while_opening", async () => {
    const user = userEvent.setup();
    const onclose = vi.fn();
    const fake = createFakeBackend({ session: "noVault" });
    let release: () => void = () => {};
    const original = fake.handlers.vaultOpen;
    fake.handlers.vaultOpen = async (args) => {
      await new Promise<void>((resolve) => (release = resolve));
      return original(args);
    };
    renderWithBackend(OpenVaultSheet, { path: PATH, onclose }, fake);
    const input = (await screen.findByLabelText("Master password")) as HTMLInputElement;
    await user.type(input, `${FIXTURE_MASTER_PASSWORD}{Enter}`);
    const open = screen.getByRole("button", { name: "Open" }) as HTMLButtonElement;
    await waitFor(() => expect(open.disabled).toBe(true));
    expect(open.querySelector(".spinner")).not.toBeNull();
    expect(screen.getByRole("dialog").querySelector("form")?.getAttribute("aria-busy")).toBe("true");
    release();
    await waitFor(() => expect(onclose).toHaveBeenCalledTimes(1));
  });

  it("open_vault_sheet_passes_axe", async () => {
    renderWithBackend(OpenVaultSheet, { path: PATH, onclose: () => {} }, createFakeBackend({ session: "noVault" }));
    await expectNoAxeViolations(await screen.findByRole("dialog"));
  });
});

describe("session.openAnother", () => {
  it("open_another_locks_first_then_sets_pending_open_path", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.install();
    await session.init();
    expect(session.status.state).toBe("unlocked");
    await session.openAnother("/vaults/Next.kdbx");
    expect(calls(fake, "vaultLock")).toHaveLength(1);
    expect(session.status.state).toBe("locked");
    expect(session.pendingOpenPath).toBe("/vaults/Next.kdbx");
    session.clearPendingOpen();
    expect(session.pendingOpenPath).toBeNull();
  });

  it("open_another_from_a_locked_session_does_not_lock_again", async () => {
    const fake = createFakeBackend({ session: "locked" });
    fake.install();
    await session.init();
    await session.openAnother("/vaults/Next.kdbx");
    expect(calls(fake, "vaultLock")).toHaveLength(0);
    expect(session.pendingOpenPath).toBe("/vaults/Next.kdbx");
  });
});
