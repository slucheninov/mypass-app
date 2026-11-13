import { screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
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
