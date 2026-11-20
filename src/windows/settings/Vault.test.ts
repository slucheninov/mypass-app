import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DisplacedCopies_Serialize as DisplacedCopies } from "../../lib/bindings";
import { toasts } from "../../lib/stores/toasts.svelte";
import { apiError, type FakeBackend } from "../../test/fakeBackend";
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
    const { fake } = await open();
    const input = (await screen.findByRole("spinbutton", { name: "Keep" })) as HTMLInputElement;
    vi.useFakeTimers();
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
    const banner = await screen.findByText("Last backup failed", { exact: false });
    expect(banner.textContent).toContain("Last backup failed");
    expect(banner.textContent).not.toContain("disk full");
  });

  it("shows no banner without a failure and an empty list without backups", async () => {
    await open();
    await screen.findByRole("button", { name: "Show in Finder" });
    expect(screen.queryByText("Last backup failed", { exact: false })).toBeNull();
    expect(screen.queryAllByRole("listitem")).toEqual([]);
  });
});

describe("retained vault copies", () => {
  const COPIES: DisplacedCopies = {
    count: 2,
    copies: [
      { location: "primary", kind: "displaced", name: ".vault.kdbx.1.2.3.displaced", bytes: 2048, id: null },
      { location: "secondary", kind: "displaced", name: ".vault.kdbx.4.5.6.displaced", bytes: 12, id: null },
    ],
    unavailable: [],
  };
  const withCopies = (fake: FakeBackend) => void (fake.state.displaced = structuredClone(COPIES));

  it("shows_nothing_without_competing_copies", async () => {
    const { fake } = await open();
    await screen.findByText("Test vault");
    await waitFor(() => expect(calls(fake, "syncDisplacedStatus").length).toBeGreaterThan(0));
    expect(screen.queryByTestId("displaced-copies")).toBeNull();
  });

  it("lists_the_copies_by_name_and_says_they_are_never_deleted_automatically", async () => {
    await open({}, withCopies);
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByText("Retained copies: 2")).toBeTruthy();
    expect(within(section).getByText(".vault.kdbx.1.2.3.displaced")).toBeTruthy();
    expect(within(section).getByText(".vault.kdbx.4.5.6.displaced")).toBeTruthy();
    expect(within(section).getByText("Vault folder")).toBeTruthy();
    expect(within(section).getByText("Sync folder")).toBeTruthy();
    expect(within(section).getByText("2 KB")).toBeTruthy();
    expect(within(section).getByText(/never deletes competing copies automatically/)).toBeTruthy();
    expect(within(section).getByText(/Review these copies in the folder shown/)).toBeTruthy();
    // Copies the backend gives no id cannot be deleted from here.
    expect(within(section).queryByRole("button")).toBeNull();
  });

  it("shows the retained working copy alongside a successful sync until it is gone", async () => {
    const name = ".vault.kdbx.1.2.3.aside";
    const { fake } = await open({}, (f) => {
      f.state.sync = { state: "idle", lastSync: 5000, recoveryFile: name };
      f.state.displaced = {
        count: 1,
        copies: [{ location: "secondary", kind: "aside", name, bytes: 1024, id: null }],
        unavailable: [],
      };
    });

    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByText("Replacement working copy")).toBeTruthy();
    expect(within(section).getByText("Sync folder")).toBeTruthy();
    expect(within(section).getByText(name)).toBeTruthy();
    expect((await screen.findByTestId("sync-recovery-file")).textContent).toContain(name);
    expect(screen.getByText(/Last synced/)).toBeTruthy();

    fake.state.displaced = { count: 0, copies: [], unavailable: [] };
    await fake.emit("syncStatusChanged", { state: "idle", lastSync: 5000 });
    await waitFor(() => expect(screen.queryByTestId("displaced-copies")).toBeNull());
    expect(screen.queryByTestId("sync-recovery-file")).toBeNull();
  });

  it("labels_a_retained_exchange_copy_as_a_replacement_working_copy", async () => {
    const name = ".vault.kdbx.1.2.3.exchanged";
    await open({}, (f) => {
      f.state.displaced = { count: 1, copies: [{ location: "primary", kind: "exchanged", name, bytes: 10, id: null }], unavailable: [] };
    });
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByText("Replacement working copy")).toBeTruthy();
    expect(within(section).getByText(name)).toBeTruthy();
    expect(within(section).queryByText("Competing copy")).toBeNull();
  });

  it("refreshes_when_a_save_of_the_primary_vault_alone_settles", async () => {
    // No secondary folder, so no sync status change: the settled save alone must refresh it.
    const { fake } = await open();
    await screen.findByText("Test vault");
    await waitFor(() => expect(calls(fake, "syncDisplacedStatus").length).toBeGreaterThan(0));
    expect(screen.queryByTestId("displaced-copies")).toBeNull();
    fake.state.displaced = structuredClone(COPIES);
    await fake.emit("saveStateChanged", { state: "saving", message: null });
    await fake.emit("saveStateChanged", { state: "idle", message: null });
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByText(".vault.kdbx.1.2.3.displaced")).toBeTruthy();
    // A failed save settles too (a failed replacement can retain a copy).
    fake.state.displaced = { count: 0, copies: [], unavailable: [] };
    await fake.emit("saveStateChanged", { state: "pending", message: null });
    await fake.emit("saveStateChanged", { state: "failed", message: "disk full" });
    await waitFor(() => expect(screen.queryByTestId("displaced-copies")).toBeNull());
  });

  it("an_unreadable_folder_is_reported_not_shown_as_no_copies", async () => {
    await open({}, (fake) => {
      fake.state.displaced = { count: 0, copies: [], unavailable: ["secondary"] };
    });
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByRole("alert").textContent).toBe("Retained copies could not be checked in: Sync folder");
    expect(within(section).queryByText(/Retained copies:/)).toBeNull();
  });

  it("a_failed_status_read_is_reported", async () => {
    await open({}, (fake) => {
      fake.handlers.syncDisplacedStatus = () => {
        throw new Error("boom");
      };
    });
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByRole("alert").textContent).toBe("Retained copies could not be checked.");
  });

  it("a_failed_refresh_after_a_vault_switch_shows_no_filenames_of_the_previous_vault", async () => {
    const { fake } = await open({}, withCopies);
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByText(".vault.kdbx.1.2.3.displaced")).toBeTruthy();
    fake.handlers.syncDisplacedStatus = () => {
      throw new Error("boom");
    };
    await fake.emit("sessionChanged", { ...fake.state.session, vaultPath: "/other/vault.kdbx" });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Retained copies could not be checked."));
    expect(screen.queryByText(".vault.kdbx.1.2.3.displaced")).toBeNull();
    expect(screen.queryByText(/Retained copies:/)).toBeNull();
  });

  const DELETABLE: DisplacedCopies = {
    count: 2,
    copies: [
      { location: "primary", kind: "displaced", name: ".vault.kdbx.1.2.3.displaced", bytes: 2048, id: "id-1" },
      { location: "secondary", kind: "aside", name: ".vault.kdbx.4.5.6.aside", bytes: 12, id: null },
    ],
    unavailable: [],
  };
  const withDeletable = (fake: FakeBackend) => void (fake.state.displaced = structuredClone(DELETABLE));
  const askToDelete = async () => {
    const section = await screen.findByTestId("displaced-copies");
    await userEvent.click(within(section).getByRole("button", { name: "Delete: .vault.kdbx.1.2.3.displaced" }));
    return screen.findByTestId("displaced-delete-confirm");
  };

  it("offers_delete_only_for_copies_with_an_id_and_names_the_file_before_deleting", async () => {
    const { fake } = await open({}, withDeletable);
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getAllByRole("button", { name: /^Delete/ })).toHaveLength(1);
    const confirm = await askToDelete();
    expect(confirm.textContent).toContain(".vault.kdbx.1.2.3.displaced");
    expect(confirm.textContent).toContain("cannot be undone");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByTestId("displaced-delete-confirm")).toBeNull());
    expect(calls(fake, "syncDisplacedDelete")).toEqual([]);
    expect(within(section).getByText(".vault.kdbx.1.2.3.displaced")).toBeTruthy();
  });

  it("hides_delete_for_iCloud_primary_copies_and_keeps_secondary_delete_available", async () => {
    const { fake } = await open({}, (backend) => {
      backend.state.displaced = {
        ...structuredClone(DELETABLE),
        copies: DELETABLE.copies.map((copy) => ({ ...copy, id: copy.name })),
      };
    });
    await askToDelete();
    await fake.emit("storageStatusChanged", { ...fake.state.storage, location: "iCloudDrive" });
    await waitFor(() => expect(screen.queryByTestId("displaced-delete-confirm")).toBeNull());
    const section = await screen.findByTestId("displaced-copies");
    expect(within(section).getByText(".vault.kdbx.1.2.3.displaced")).toBeTruthy();
    expect(within(section).queryByRole("button", { name: "Delete: .vault.kdbx.1.2.3.displaced" })).toBeNull();
    expect(within(section).getByRole("button", { name: "Delete: .vault.kdbx.4.5.6.aside" })).toBeTruthy();
    expect(calls(fake, "syncDisplacedDelete")).toEqual([]);
  });

  it("deletes_the_confirmed_copy_by_its_id_and_refreshes_the_list", async () => {
    const { fake } = await open({}, withDeletable);
    await askToDelete();
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(calls(fake, "syncDisplacedDelete").map((c) => c.args)).toEqual([{ id: "id-1" }]));
    await waitFor(() => expect(screen.queryByText(".vault.kdbx.1.2.3.displaced")).toBeNull());
    expect(screen.getByText(".vault.kdbx.4.5.6.aside")).toBeTruthy();
    expect(screen.getByText("Retained copies: 1")).toBeTruthy();
    expect(screen.queryByTestId("displaced-delete-confirm")).toBeNull();
  });

  it("a_refused_delete_says_nothing_was_deleted_and_lists_again", async () => {
    const { fake } = await open({}, (f) => {
      withDeletable(f);
      f.handlers.syncDisplacedDelete = () => {
        throw apiError("retainedCopyRefused", "changed");
      };
    });
    await askToDelete();
    const before = calls(fake, "syncDisplacedStatus").length;
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() =>
      expect(toasts.items.map((toast) => toast.text)).toContain(
        "The copy changed or could not be verified, so it was not deleted. The list was refreshed.",
      ),
    );
    await waitFor(() => expect(calls(fake, "syncDisplacedStatus").length).toBeGreaterThan(before));
    expect(screen.getByText(".vault.kdbx.1.2.3.displaced")).toBeTruthy();
  });

  it("a_lock_while_confirming_closes_the_confirmation_without_deleting", async () => {
    const { fake } = await open({}, withDeletable);
    await askToDelete();
    // Locked: the backend offers no ids any more.
    fake.state.displaced = {
      ...structuredClone(DELETABLE),
      copies: DELETABLE.copies.map((copy) => ({ ...copy, id: null })),
    };
    await fake.emit("sessionChanged", { ...fake.state.session, state: "locked" });
    await waitFor(() => expect(screen.queryByTestId("displaced-delete-confirm")).toBeNull());
    expect(calls(fake, "syncDisplacedDelete")).toEqual([]);
  });
});
