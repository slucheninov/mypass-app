import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { expectNoAxeViolations } from "../test/axe";
import { FIXTURE_MASTER_PASSWORD } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

const FOUND = [
  { path: "/vaults/home/Home.kdbx", modified: 1_700_000_000_000, size: 2048 },
  { path: "/vaults/work/Work.kdbx", modified: null, size: null },
];

describe("FindVaults", () => {
  it("find_files_lists_results_and_choosing_prefills_open", async () => {
    const user = userEvent.setup();
    const { fake } = await renderMainApp({ session: "noVault" }, (f) => (f.state.foundFiles = structuredClone(FOUND)));
    await user.click(await screen.findByRole("button", { name: "Find vault files" }));
    const finder = await screen.findByRole("dialog", { name: "Find vault files" });
    const list = await within(finder).findByRole("list");
    expect(within(list).getByText("Home.kdbx")).toBeTruthy();
    expect(within(list).getByText("Work.kdbx")).toBeTruthy();
    expect(within(list).getByText("2 KB")).toBeTruthy();
    expect(within(list).getByText("/vaults/home")).toBeTruthy();
    await user.click(within(list).getByRole("button", { name: /Home\.kdbx/ }));
    const sheet = await screen.findByRole("dialog", { name: "Open vault" });
    expect(screen.queryByRole("dialog", { name: "Find vault files" })).toBeNull();
    expect(within(sheet).getByText("Home.kdbx")).toBeTruthy();
    expect(calls(fake, "vaultOpen")).toHaveLength(0);
    await user.type(within(sheet).getByLabelText("Master password"), FIXTURE_MASTER_PASSWORD);
    await user.click(within(sheet).getByRole("button", { name: "Open" }));
    await waitFor(() => expect(calls(fake, "vaultOpen")).toHaveLength(1));
    expect(calls(fake, "vaultOpen")[0]?.args).toEqual({ path: "/vaults/home/Home.kdbx", password: FIXTURE_MASTER_PASSWORD });
  });

  it("find_files_shows_a_spinner_while_searching", async () => {
    const user = userEvent.setup();
    let release: (v: typeof FOUND) => void = () => {};
    await renderMainApp({ session: "noVault" }, (f) => {
      f.handlers.vaultFindFiles = () => new Promise((resolve) => (release = resolve));
    });
    await user.click(await screen.findByRole("button", { name: "Find vault files" }));
    const finder = await screen.findByRole("dialog", { name: "Find vault files" });
    expect(await within(finder).findByText("Searching…")).toBeTruthy();
    release(FOUND);
    expect(await within(finder).findByText("Home.kdbx")).toBeTruthy();
    expect(within(finder).queryByText("Searching…")).toBeNull();
  });

  it("find_files_empty_state_and_cancel", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "noVault" });
    await user.click(await screen.findByRole("button", { name: "Find vault files" }));
    const finder = await screen.findByRole("dialog", { name: "Find vault files" });
    expect(await within(finder).findByText("No vault files found")).toBeTruthy();
    await user.click(within(finder).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("find_files_failure_is_reported_and_the_dialog_stays_usable", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "noVault" }, (f) => {
      f.handlers.vaultFindFiles = () => {
        throw { code: "io", message: "denied" };
      };
    });
    await user.click(await screen.findByRole("button", { name: "Find vault files" }));
    const finder = await screen.findByRole("dialog", { name: "Find vault files" });
    expect(await within(finder).findByText("Something went wrong")).toBeTruthy();
    expect(within(finder).queryByText("Searching…")).toBeNull();
  });

  it("find_dialog_passes_axe", async () => {
    const user = userEvent.setup();
    await renderMainApp({ session: "noVault" }, (f) => (f.state.foundFiles = structuredClone(FOUND)));
    await user.click(await screen.findByRole("button", { name: "Find vault files" }));
    const finder = await screen.findByRole("dialog", { name: "Find vault files" });
    await within(finder).findByText("Home.kdbx");
    await expectNoAxeViolations(finder);
  });
});
