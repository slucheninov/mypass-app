import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { calls } from "../../test/mainApp";
import { renderSettingsApp } from "../../test/settingsApp";

afterEach(() => {
  vi.useRealTimers();
});

const open = (opts: Parameters<typeof renderSettingsApp>[0] = {}, prepare?: Parameters<typeof renderSettingsApp>[1] extends infer O ? (O extends { prepare?: infer P } ? P : never) : never) =>
  renderSettingsApp(opts, { pane: "Vault", prepare });

const BACKUPS = [
  { id: "backup-1700000100.kdbx", created: 1_700_000_100_000, size: 2048 },
  { id: "backup-1700000000.kdbx", created: 1_700_000_000_000, size: 1_048_576 },
];

describe("vault card", () => {
  it("shows the vault name, its file status and no Open another vault button (R31)", async () => {
    await open();
    expect(await screen.findByText("Test vault")).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Open another/ })).toBeNull();
  });

  it("shows a missing file", async () => {
    await renderSettingsApp({}, {
      pane: "Vault",
      prepare: (fake) => (fake.state.session = { ...fake.state.session, fileStatus: { kind: "missing" } }),
    });
    expect(await screen.findByText("File not found")).toBeTruthy();
  });
});

describe("backups", () => {
  it("backups_show_in_finder_calls_app_reveal_backups", async () => {
    const user = userEvent.setup();
    const { fake } = await open();
    await user.click(await screen.findByRole("button", { name: "Show in Finder" }));
    expect(calls(fake, "appRevealBackups").length).toBe(1);
  });

  it("backups_reveal_label_is_explorer_on_windows", async () => {
    await open({ os: "windows" });
    expect(await screen.findByRole("button", { name: "Show in Explorer" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Show in Finder" })).toBeNull();
  });

  it("backup_count_stepper_clamps_1_50_and_debounces_updates", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { fake } = await open();
    const input = (await screen.findByRole("spinbutton", { name: "Keep" })) as HTMLInputElement;
    expect(input.value).toBe("10");
    for (let i = 0; i < 10; i += 1) await fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(input.value).toBe("20");
    expect(calls(fake, "settingsUpdate")).toEqual([]);
    await vi.advanceTimersByTimeAsync(301);
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([{ backupCount: 20 }]);
    await fireEvent.input(input, { target: { value: "99" } });
    await fireEvent.change(input);
    expect(input.value).toBe("50");
    await fireEvent.input(input, { target: { value: "0" } });
    await fireEvent.change(input);
    expect(input.value).toBe("1");
  });

  it("backup_interval_has_four_options", async () => {
    const user = userEvent.setup();
    const { fake } = await open();
    const select = (await screen.findByRole("combobox", { name: "Backup interval" })) as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => [o.value, o.textContent])).toEqual([
      ["everySave", "Every save"],
      ["tenMinutes", "Every 10 minutes"],
      ["hourly", "Hourly"],
      ["daily", "Daily"],
    ]);
    await user.selectOptions(select, "hourly");
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([{ backupInterval: "hourly" }]);
  });

  it("backup_list_shows_date_and_size", async () => {
    await open({}, (fake) => (fake.state.backups = structuredClone(BACKUPS)));
    const rows = await screen.findAllByRole("listitem");
    expect(rows.length).toBe(2);
    expect(within(rows[0]!).getByText("2 KB")).toBeTruthy();
    expect(within(rows[1]!).getByText("1 MB")).toBeTruthy();
    const date = new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" }).format(new Date(1_700_000_100_000));
    expect(within(rows[0]!).getByText(date)).toBeTruthy();
  });

  it("the_backup_list_reloads_after_an_unlock_or_a_vault_switch_but_not_a_save_state_change", async () => {
    const { fake } = await open({ session: "locked" });
    await screen.findByRole("combobox", { name: "Backup interval" });
    await waitFor(() => expect(calls(fake, "backupList").length).toBeGreaterThan(0));
    const n = () => calls(fake, "backupList").length;
    const start = n();
    await fake.emit("saveStateChanged", { state: "pending", message: null });
    await new Promise((r) => setTimeout(r, 20));
    expect(n()).toBe(start);
    fake.state.backups = structuredClone(BACKUPS);
    await fake.emit("sessionChanged", { ...fake.state.session, state: "unlocked", vaultPath: "/vaults/a.kdbx" });
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(2));
    const afterUnlock = n();
    expect(afterUnlock).toBeGreaterThan(start);
    fake.state.backups = [];
    await fake.emit("sessionChanged", { ...fake.state.session, state: "unlocked", vaultPath: "/vaults/b.kdbx" });
    await waitFor(() => expect(n()).toBeGreaterThan(afterUnlock));
    await waitFor(() => expect(screen.queryAllByRole("listitem")).toHaveLength(0));
  });

  it("backup_restore_requires_confirmation_then_calls_backup_restore", async () => {
    const user = userEvent.setup();
    const { fake } = await open({}, (f) => (f.state.backups = structuredClone(BACKUPS)));
    const rows = await screen.findAllByRole("listitem");
    await user.click(within(rows[1]!).getByRole("button", { name: "Restore…" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Restore this backup? Your current vault is backed up first.")).toBeTruthy();
    expect(calls(fake, "backupRestore")).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(calls(fake, "backupRestore")).toEqual([]);
    await user.click(within((await screen.findAllByRole("listitem"))[1]!).getByRole("button", { name: "Restore…" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Restore" }));
    await waitFor(() => expect(calls(fake, "backupRestore").map((c) => c.args)).toEqual([{ id: "backup-1700000000.kdbx" }]));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("backup_restore_password_mismatch_shows_localised_message", async () => {
    const user = userEvent.setup();
    const { fake } = await open({}, (f) => {
      f.state.backups = structuredClone(BACKUPS);
      f.state.restoreFailure = "invalidInput";
    });
    const rows = await screen.findAllByRole("listitem");
    await user.click(within(rows[0]!).getByRole("button", { name: "Restore…" }));
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Restore" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "The backup does not open with your current master password",
    );
    expect(calls(fake, "backupRestore").length).toBe(1);
  });

  it("backup_failure_banner_visible_when_last_failure_present", async () => {
    await open({}, (f) => (f.state.backupFailure = { message: "disk full", at: 1_700_000_100_000 }));
    const banner = await screen.findByRole("status");
    expect(banner.textContent).toContain("Last backup failed");
    expect(banner.textContent).not.toContain("disk full");
  });

  it("shows no banner without a failure and an empty list without backups", async () => {
    await open();
    await screen.findByRole("button", { name: "Show in Finder" });
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryAllByRole("listitem")).toEqual([]);
  });
});
