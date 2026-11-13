import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { expectNoAxeViolations } from "../test/axe";
import { apiError } from "../test/fakeBackend";
import { calls, renderMainApp } from "../test/mainApp";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
import { save } from "@tauri-apps/plugin-dialog";

const VAULT_PATH = "/vaults/My Vault.kdbx";

beforeEach(() => {
  vi.mocked(save).mockReset();
  vi.mocked(save).mockResolvedValue(VAULT_PATH);
});

async function openForm(opts: Parameters<typeof renderMainApp>[1] = undefined) {
  const user = userEvent.setup();
  const app = await renderMainApp({ session: "noVault" }, opts);
  await user.click(await screen.findByRole("button", { name: "Create vault" }));
  const dialog = await screen.findByRole("dialog", { name: "Create vault" });
  const fill = async (name: string, pw: string, confirm: string) => {
    await user.clear(within(dialog).getByLabelText("Vault name"));
    if (name) await user.type(within(dialog).getByLabelText("Vault name"), name);
    if (pw) await user.type(within(dialog).getByLabelText("Master password"), pw);
    if (confirm) await user.type(within(dialog).getByLabelText("Confirm password"), confirm);
  };
  const submit = () => user.click(within(dialog).getByRole("button", { name: "Create vault" }));
  return { user, dialog, fill, submit, ...app };
}

describe("CreateVault", () => {
  it("create_requires_matching_confirmation_and_skips_backend_call", async () => {
    const { fake, fill, submit, dialog } = await openForm();
    await fill("Family", "longenough1", "different1");
    await submit();
    expect(await within(dialog).findByText("Passwords do not match")).toBeTruthy();
    expect(save).not.toHaveBeenCalled();
    expect(calls(fake, "vaultCreate")).toHaveLength(0);
  });

  it("create_requires_a_password", async () => {
    const { fake, fill, submit, dialog } = await openForm();
    await fill("Family", "", "");
    await submit();
    expect(await within(dialog).findByText("Choose a password")).toBeTruthy();
    expect(calls(fake, "vaultCreate")).toHaveLength(0);
  });

  it("create_calls_vault_create_with_chosen_path_and_name", async () => {
    const { fake, fill, submit } = await openForm();
    await fill("Family", "longenough1", "longenough1");
    await submit();
    await waitFor(() => expect(calls(fake, "vaultCreate")).toHaveLength(1));
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ defaultPath: "MyPass.kdbx", filters: [{ name: "KeePass", extensions: ["kdbx"] }] }),
    );
    expect(calls(fake, "vaultCreate")[0]?.args).toEqual({ path: VAULT_PATH, password: "longenough1", name: "Family" });
    // The vault is open now: the form and the unlock screen are gone.
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByLabelText("Master password")).toBeNull();
  });

  it("a_cancelled_save_dialog_keeps_the_form_and_creates_nothing", async () => {
    vi.mocked(save).mockResolvedValue(null);
    const { fake, fill, submit, dialog } = await openForm();
    await fill("Family", "longenough1", "longenough1");
    await submit();
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(calls(fake, "vaultCreate")).toHaveLength(0);
    expect(calls(fake, "vaultOpenWarnings")).toHaveLength(0);
    expect(dialog.isConnected).toBe(true);
  });

  it("create_existing_file_error_alreadyExists_shown", async () => {
    const { fill, submit, dialog } = await openForm((f) => (f.state.existingFiles = [VAULT_PATH]));
    await fill("Family", "longenough1", "longenough1");
    await submit();
    expect(await within(dialog).findByText("A vault file already exists there")).toBeTruthy();
    // The password was fine: only the location has to change, so it stays typed.
    expect((within(dialog).getByLabelText("Master password") as HTMLInputElement).value).toBe("longenough1");
  });

  it("create_failure_other_than_path_or_io_clears_the_passwords", async () => {
    const { fill, submit, dialog } = await openForm((f) => {
      f.handlers.vaultCreate = () => {
        throw apiError("corrupt");
      };
    });
    await fill("Family", "longenough1", "longenough1");
    await submit();
    await within(dialog).findByText("The vault file looks damaged");
    expect((within(dialog).getByLabelText("Master password") as HTMLInputElement).value).toBe("");
    expect((within(dialog).getByLabelText("Confirm password") as HTMLInputElement).value).toBe("");
  });

  it("weak_password_hint_below_eight_chars_does_not_block", async () => {
    const { fake, fill, submit, dialog } = await openForm();
    await fill("Family", "abc", "abc");
    expect(within(dialog).getByText("That password is short. Longer is safer.")).toBeTruthy();
    await submit();
    await waitFor(() => expect(calls(fake, "vaultCreate")).toHaveLength(1));
  });

  it("no_weak_hint_for_an_empty_or_long_password", async () => {
    const { fill, dialog } = await openForm();
    expect(within(dialog).queryByText("That password is short. Longer is safer.")).toBeNull();
    await fill("Family", "longenough1", "");
    expect(within(dialog).queryByText("That password is short. Longer is safer.")).toBeNull();
  });

  it("create_form_passes_axe", async () => {
    const { dialog } = await openForm();
    await expectNoAxeViolations(dialog);
  });
});
