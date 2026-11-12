import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OpenWarning } from "./bindings";
import { FIXTURE_MASTER_PASSWORD } from "../test/fixtures";
import { apiError } from "../test/fakeBackend";
import type { CommandName } from "../test/fakeBackend";
import { calls, renderMainApp } from "../test/mainApp";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
import { open, save } from "@tauri-apps/plugin-dialog";

const ICLOUD_TITLE = "This vault is in iCloud Drive";
const OLD_TITLE = "Another copy of MyPass is running";
const ICLOUD: OpenWarning = { kind: "icloud" };
const OLD: OpenWarning = { kind: "oldAppRunning", executable: "MyPass-Swift" };

beforeEach(() => {
  vi.mocked(open).mockReset();
  vi.mocked(save).mockReset();
});

async function startOpenExisting(warnings: OpenWarning[], path = "/vaults/Other.kdbx") {
  vi.mocked(open).mockResolvedValue(path);
  const user = userEvent.setup();
  const app = await renderMainApp({ session: "noVault" }, (f) => (f.state.openWarnings = warnings));
  await user.click(await screen.findByRole("button", { name: "Open existing vault" }));
  return { user, ...app };
}

describe("open guard", () => {
  it("open_guard_icloud_warning_blocks_until_continue", async () => {
    const { user, fake } = await startOpenExisting([ICLOUD]);
    const dialog = await screen.findByRole("dialog", { name: ICLOUD_TITLE });
    expect(within(dialog).getByText(/iCloud Drive can sync a vault file/)).toBeTruthy();
    // Nothing is asked for before the user decides.
    expect(screen.queryByRole("dialog", { name: "Open vault" })).toBeNull();
    expect(calls(fake, "vaultOpen")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    expect(await screen.findByRole("dialog", { name: "Open vault" })).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: ICLOUD_TITLE })).toBeNull();
  });

  it("open_guard_cancel_stops_the_flow", async () => {
    const { user, fake } = await startOpenExisting([ICLOUD]);
    const dialog = await screen.findByRole("dialog", { name: ICLOUD_TITLE });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls(fake, "vaultOpen")).toHaveLength(0);
  });

  it("open_guard_escape_cancels", async () => {
    const { user } = await startOpenExisting([ICLOUD]);
    await screen.findByRole("dialog", { name: ICLOUD_TITLE });
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("open_guard_old_app_warning_names_the_executable", async () => {
    await startOpenExisting([OLD]);
    const dialog = await screen.findByRole("dialog", { name: OLD_TITLE });
    expect(within(dialog).getByText(/An older MyPass \(MyPass-Swift\) is open\./)).toBeTruthy();
  });

  it("open_guard_two_warnings_ask_in_order_and_cancel_stops_the_rest", async () => {
    const { user } = await startOpenExisting([ICLOUD, OLD]);
    const first = await screen.findByRole("dialog", { name: ICLOUD_TITLE });
    await user.click(within(first).getByRole("button", { name: "Continue" }));
    const second = await screen.findByRole("dialog", { name: OLD_TITLE });
    await user.click(within(second).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("open_guard_second_warning_not_shown_when_the_first_is_cancelled", async () => {
    const { user } = await startOpenExisting([ICLOUD, OLD]);
    const first = await screen.findByRole("dialog", { name: ICLOUD_TITLE });
    await user.click(within(first).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("dialog", { name: OLD_TITLE })).toBeNull();
  });

  it("open_guard_no_warnings_shows_no_dialog", async () => {
    await startOpenExisting([]);
    expect(await screen.findByRole("dialog", { name: "Open vault" })).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: ICLOUD_TITLE })).toBeNull();
    expect(screen.queryByRole("dialog", { name: OLD_TITLE })).toBeNull();
  });

  it("open_guard_failure_does_not_block_opening", async () => {
    vi.mocked(open).mockResolvedValue("/vaults/Other.kdbx");
    const user = userEvent.setup();
    await renderMainApp({ session: "noVault" }, (f) => {
      f.handlers.vaultOpenWarnings = () => {
        throw apiError("io");
      };
    });
    await user.click(await screen.findByRole("button", { name: "Open existing vault" }));
    expect(await screen.findByRole("dialog", { name: "Open vault" })).toBeTruthy();
  });

  // Every entry point asks exactly once with the chosen path, before its backend call.
  const ENTRY_POINTS: [string, string, CommandName, (user: ReturnType<typeof userEvent.setup>) => Promise<void>][] = [
    [
      "create",
      "/vaults/New.kdbx",
      "vaultCreate",
      async (user) => {
        vi.mocked(save).mockResolvedValue("/vaults/New.kdbx");
        await user.click(await screen.findByRole("button", { name: "Create vault" }));
        const dialog = await screen.findByRole("dialog", { name: "Create vault" });
        await user.type(within(dialog).getByLabelText("Master password"), "longenough1");
        await user.type(within(dialog).getByLabelText("Confirm password"), "longenough1");
        await user.click(within(dialog).getByRole("button", { name: "Create vault" }));
      },
    ],
    [
      "open existing",
      "/vaults/Other.kdbx",
      "vaultOpen",
      async (user) => {
        vi.mocked(open).mockResolvedValue("/vaults/Other.kdbx");
        await user.click(await screen.findByRole("button", { name: "Open existing vault" }));
        await passwordStep(user);
      },
    ],
    [
      "find",
      "/vaults/found/Found.kdbx",
      "vaultOpen",
      async (user) => {
        await user.click(await screen.findByRole("button", { name: "Find vault files" }));
        const finder = await screen.findByRole("dialog", { name: "Find vault files" });
        await user.click(await within(finder).findByRole("button", { name: /Found\.kdbx/ }));
        await passwordStep(user);
      },
    ],
    [
      "recent",
      "/vaults/recent/Recent.kdbx",
      "vaultOpen",
      async (user) => {
        // Recents are on the locked screen; this entry starts from a locked session below.
        const list = await screen.findByRole("list", { name: "Recent vaults" });
        await user.click(within(list).getByRole("button", { name: /Recent\.kdbx/ }));
        await passwordStep(user);
      },
    ],
  ];

  async function passwordStep(user: ReturnType<typeof userEvent.setup>) {
    const sheet = await screen.findByRole("dialog", { name: "Open vault" });
    await user.type(within(sheet).getByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(within(sheet).getByRole("button", { name: "Open" }));
  }

  it.each(ENTRY_POINTS)("open_guard_applies_to_create_open_existing_find_and_recents (%s)", async (name, path, backendCall, drive) => {
    const user = userEvent.setup();
    const { fake } = await renderMainApp({ session: name === "recent" ? "locked" : "noVault" }, (f) => {
      f.state.foundFiles = [{ path, modified: null, size: null }];
      f.state.recents = [{ path, fileName: path.split("/").pop() ?? path, exists: true }];
    });
    await drive(user);
    await waitFor(() => expect(calls(fake, backendCall)).toHaveLength(1));
    const warnings = calls(fake, "vaultOpenWarnings");
    expect(warnings).toHaveLength(1);
    expect(warnings[0]?.args).toEqual({ path });
    const order = fake.calls.map((c) => c.command);
    expect(order.indexOf("vaultOpenWarnings")).toBeLessThan(order.indexOf(backendCall));
  });
});
