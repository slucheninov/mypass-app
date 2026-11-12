import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CsvPreview } from "../lib/bindings";
import { setLanguage } from "../lib/i18n/index.svelte";
import { entries } from "../lib/stores/entries.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { session } from "../lib/stores/session.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { expectNoAxeViolations } from "../test/axe";
import { createFakeBackend, apiError, type FakeBackend } from "../test/fakeBackend";
import { ROOT_GROUP_ID, WORK_GROUP_ID } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import { renderWithBackend } from "../test/render";
import ImportDialog from "./ImportDialog.svelte";

afterEach(() => {
  entries.stop();
  session.stop();
  navigation.reset();
  toasts.clear();
  setLanguage("en");
});

const PREVIEW: CsvPreview = {
  total: 7,
  sampleTitles: ["Alpha", "Beta", "Gamma", "Delta", "Epsilon"],
  skipped: [
    { line: 3, reason: "fieldCountMismatch" },
    { line: 9, reason: "unterminatedQuote" },
  ],
};

async function openDialog(preview: CsvPreview | null = PREVIEW, prepare?: (fake: FakeBackend) => void) {
  const fake = createFakeBackend({ session: "unlocked" });
  fake.state.csvPreview = preview;
  prepare?.(fake);
  fake.install();
  await session.init();
  await entries.refresh();
  const onclose = vi.fn();
  const utils = renderWithBackend(ImportDialog, { onclose }, fake);
  return { ...utils, fake, onclose, user: userEvent.setup() };
}

/** The dialog once the preview has arrived (the picker answers asynchronously). */
async function ready(title = "Import CSV", button = "Import") {
  const dialog = await screen.findByRole("dialog", { name: title });
  await within(dialog).findByRole("button", { name: button });
  return dialog;
}

describe("import dialog", () => {
  it("import_preview_lists_skipped_rows_with_line_numbers_and_localised_reasons", async () => {
    await openDialog();
    const dialog = await ready();
    expect(within(dialog).getByText("7 items found")).toBeTruthy();
    expect(within(dialog).getByText("Row 3: the number of columns does not match")).toBeTruthy();
    expect(within(dialog).getByText("Row 9: a quoted value is not closed")).toBeTruthy();
  });

  it("import_preview_shows_no_passwords", async () => {
    const { fake } = await openDialog();
    const dialog = await ready();
    for (const title of PREVIEW.sampleTitles) expect(within(dialog).getByText(title)).toBeTruthy();
    // The dialog only ever receives titles and counts; nothing resembling a secret is rendered.
    expect(dialog.querySelector("input[type='password']")).toBeNull();
    expect(dialog.textContent).not.toMatch(/hunter2|gitlab-secret|password/i);
    expect(JSON.stringify(fake.state.csvPreview)).not.toMatch(/password/i);
  });

  it("the_dialog_opens_the_native_picker_through_the_backend_and_passes_no_path", async () => {
    const { fake } = await openDialog();
    await screen.findByRole("dialog");
    expect(calls(fake, "importCsvPreview")).toHaveLength(1);
    expect(calls(fake, "importCsvPreview")[0]?.args).toEqual({});
  });

  it("import_commit_reports_plural_toast_and_refreshes_list", async () => {
    const { fake, onclose, user } = await openDialog({ total: 2, sampleTitles: ["Alpha", "Beta"], skipped: [] });
    const dialog = await ready();
    await user.click(within(dialog).getByRole("button", { name: "Import" }));
    await waitFor(() => expect(onclose).toHaveBeenCalled());
    expect(toasts.items.map((x) => x.text)).toEqual(["2 items imported"]);
    expect(calls(fake, "importCsvCommit")).toHaveLength(1);
    await waitFor(() => expect(entries.items.map((e) => e.title)).toEqual(expect.arrayContaining(["Alpha", "Beta"])));
  });

  it("the_imported_toast_uses_the_russian_plural_forms", async () => {
    setLanguage("ru");
    const { user } = await openDialog({ total: 5, sampleTitles: ["A", "B", "C", "D", "E"], skipped: [] });
    // The Russian dialog title and Import button.
    const importLabel = "\u0418\u043c\u043f\u043e\u0440\u0442\u0438\u0440\u043e\u0432\u0430\u0442\u044c";
    const dialog = await ready("\u0418\u043c\u043f\u043e\u0440\u0442 CSV", importLabel);
    await user.click(within(dialog).getByRole("button", { name: importLabel }));
    await waitFor(() => expect(toasts.items).toHaveLength(1));
    expect(toasts.items[0]?.text).toBe("\u0418\u043c\u043f\u043e\u0440\u0442\u0438\u0440\u043e\u0432\u0430\u043d\u043e 5 \u0437\u0430\u043f\u0438\u0441\u0435\u0439");
  });

  it.each([
    ["missing required column title", "The file must have title, username and password columns"],
    ["file is not valid UTF-8", "The file is not UTF-8 text"],
    ["file is empty", "The file is empty"],
  ])("import_missing_column_error_maps_to_message (%s)", async (message, expected) => {
    const { onclose } = await openDialog(PREVIEW, (fake) => {
      fake.state.csvFailure = apiError("invalidInput", message);
    });
    const dialog = await screen.findByRole("dialog", { name: "Import CSV" });
    expect((await within(dialog).findByRole("alert")).textContent).toBe(expected);
    expect(within(dialog).queryByRole("button", { name: "Import" })).toBeNull();
    expect(onclose).not.toHaveBeenCalled();
  });

  it("an_unknown_failure_shows_the_generic_localised_message", async () => {
    await openDialog(PREVIEW, (fake) => {
      fake.state.csvFailure = apiError("limitExceeded", "the file is too large");
    });
    const dialog = await screen.findByRole("dialog");
    expect((await within(dialog).findByRole("alert")).textContent).toBe("Limit reached");
  });

  it("choose_a_csv_file_retries_the_picker_after_a_failure", async () => {
    const { fake, user } = await openDialog(PREVIEW, (f) => {
      f.state.csvFailure = apiError("invalidInput", "file is empty");
    });
    const dialog = await screen.findByRole("dialog");
    await within(dialog).findByRole("alert");
    await user.click(within(dialog).getByRole("button", { name: "Choose a CSV file" }));
    expect(await within(dialog).findByText("7 items found")).toBeTruthy();
    expect(calls(fake, "importCsvPreview")).toHaveLength(2);
  });

  it("a_cancelled_native_dialog_closes_without_a_toast", async () => {
    const { fake, onclose } = await openDialog(null);
    await waitFor(() => expect(onclose).toHaveBeenCalled());
    expect(calls(fake, "importCsvCommit")).toHaveLength(0);
    expect(toasts.items).toHaveLength(0);
  });

  it("import_group_defaults_to_current_group", async () => {
    navigation.setScope({ kind: "group", groupId: WORK_GROUP_ID });
    const { user, fake } = await openDialog();
    const select = (await screen.findByRole("combobox", { name: "Import into" })) as HTMLSelectElement;
    expect(select.value).toBe(WORK_GROUP_ID);
    await user.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(calls(fake, "importCsvCommit")).toHaveLength(1));
    expect(calls(fake, "importCsvCommit")[0]?.args).toEqual({ groupId: WORK_GROUP_ID });
  });

  it("the_group_defaults_to_the_vault_root_outside_a_group_and_never_offers_the_bin", async () => {
    const { fake } = await openDialog();
    const select = (await screen.findByRole("combobox", { name: "Import into" })) as HTMLSelectElement;
    expect(select.value).toBe(ROOT_GROUP_ID);
    const labels = within(select).getAllByRole("option").map((o) => o.textContent ?? "");
    expect(labels[0]).toBe("Vault");
    expect(labels.some((l) => /bin|deleted/i.test(l))).toBe(false);
    expect(fake.state.groups.length).toBeGreaterThan(1);
  });

  it("import_dialog_cancel_makes_no_backend_commit", async () => {
    const { fake, onclose, user } = await openDialog();
    const dialog = await ready();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(onclose).toHaveBeenCalled();
    expect(calls(fake, "importCsvCommit")).toHaveLength(0);
    expect(toasts.items).toHaveLength(0);
  });

  it("a_failed_commit_keeps_the_dialog_open_and_offers_choosing_the_file_again", async () => {
    const { fake, onclose, user } = await openDialog();
    // The backend spends the remembered file on every commit, so Import cannot be pressed again.
    fake.handlers.importCsvCommit = () => {
      throw apiError("invalidInput", "no CSV file was previewed");
    };
    const dialog = await ready();
    await user.click(within(dialog).getByRole("button", { name: "Import" }));
    expect((await within(dialog).findByRole("alert")).textContent).toBe("Invalid input");
    expect(onclose).not.toHaveBeenCalled();
    expect(within(dialog).queryByRole("button", { name: "Import" })).toBeNull();
    expect(within(dialog).queryByText("7 items found")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Choose a CSV file" }));
    expect(await within(dialog).findByText("7 items found")).toBeTruthy();
    expect(calls(fake, "importCsvPreview")).toHaveLength(2);
  });

  it("nested_groups_are_indented_with_non_breaking_spaces", async () => {
    await openDialog();
    const select = (await screen.findByRole("combobox", { name: "Import into" })) as HTMLSelectElement;
    const labels = within(select).getAllByRole("option").map((o) => o.textContent ?? "");
    expect(labels.some((l) => l.startsWith("\u00a0\u00a0\u00a0\u00a0"))).toBe(true);
    expect(labels.every((l) => !l.startsWith(" "))).toBe(true);
  });

  it("a_long_skipped_list_is_capped_with_an_and_more_line", async () => {
    const skipped = Array.from({ length: 100 }, (_, i) => ({ line: i + 2, reason: "fieldCountMismatch" as const }));
    await openDialog({ total: 3, sampleTitles: ["A", "B", "C"], skipped });
    const dialog = await ready();
    expect(within(dialog).getAllByText(/^Row \d+:/)).toHaveLength(20);
    expect(within(dialog).getByText("and 80 more")).toBeTruthy();
  });

  it("the_toolbar_import_csv_item_opens_the_dialog_in_the_main_window", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.state.csvPreview = { total: 1, sampleTitles: ["Solo"], skipped: [] };
    const user = userEvent.setup();
    await renderMainApp(fake);
    await user.click(await screen.findByRole("button", { name: "Import" }));
    await user.click(await screen.findByRole("menuitem", { name: "Import CSV…" }));
    const dialog = await screen.findByRole("dialog", { name: "Import CSV" });
    await user.click(within(dialog).getByRole("button", { name: "Import" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Import CSV" })).toBeNull());
    expect(toasts.items.map((x) => x.text)).toEqual(["1 item imported"]);
  });

  it("import_passes_axe", async () => {
    await openDialog();
    const dialog = await screen.findByRole("dialog");
    await within(dialog).findByText("7 items found");
    await expectNoAxeViolations(dialog);
  });
});
