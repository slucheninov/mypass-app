import { screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { navigation } from "../lib/stores/navigation.svelte";
import { backendWith, openEdit, openNew, user } from "../test/editorHelpers";
import { calls } from "../test/mainApp";

afterEach(() => vi.useRealTimers());

const pick = (...names: string[]) => names.map((name, i) => ({ token: `tok-${name}-${i}`, name, size: 1024 * (i + 1) }));
const savedOps = (fake: Parameters<typeof calls>[0]) => calls(fake, "entrySave")[0]?.args.model.attachmentOps;

describe("FilesForm", () => {
  it("files_add_uses_attachment_pick_and_queues_ops", async () => {
    const fake = backendWith();
    await openNew(fake, "files");
    fake.state.pickResult = pick("a.txt", "b.pdf");
    await user().click(screen.getByRole("button", { name: "Add files…" }));
    const list = await screen.findByRole("list", { name: "Attachments" });
    expect(within(list).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      expect.stringContaining("a.txt"),
      expect.stringContaining("b.pdf"),
    ]);
    expect(calls(fake, "attachmentPick")).toHaveLength(1);
    await user().type(screen.getByRole("textbox", { name: "Title" }), "Docs");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    // The webview never supplies a path: the ops carry the single-use tokens.
    expect(savedOps(fake)).toEqual([
      { kind: "add", path: "tok-a.txt-0" },
      { kind: "add", path: "tok-b.pdf-1" },
    ]);
    expect(fake.state.entries.at(-1)?.model.attachments.map((a) => a.name)).toEqual(["a.txt", "b.pdf"]);
  });

  it("a_cancelled_dialog_adds_nothing", async () => {
    const fake = backendWith();
    await openNew(fake, "files");
    await user().click(screen.getByRole("button", { name: "Add files…" }));
    await waitFor(() => expect(calls(fake, "attachmentPick")).toHaveLength(1));
    expect(screen.queryByRole("list", { name: "Attachments" })).toBeNull();
    expect((navigation.editModel?.attachmentOps ?? []).length).toBe(0);
  });

  it("remove_existing_attachment_queues_remove_op", async () => {
    const fake = backendWith([
      { id: "e-files", title: "Docs", entryType: "files", attachments: [{ name: "old.txt", size: 5 }, { name: "keep.txt", size: 7 }], modified: 1 },
    ]);
    await openEdit(fake, "e-files");
    const list = screen.getByRole("list", { name: "Attachments" });
    const item = within(list).getAllByRole("listitem").find((li) => li.textContent?.includes("old.txt"))!;
    await user().click(within(item).getByRole("button", { name: "Remove" }));
    expect(within(list).queryByText("old.txt")).toBeNull();
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(savedOps(fake)).toEqual([{ kind: "remove", name: "old.txt" }]);
  });

  it("removing_a_queued_file_drops_its_add_op", async () => {
    const fake = backendWith();
    await openNew(fake, "files");
    fake.state.pickResult = pick("a.txt");
    await user().click(screen.getByRole("button", { name: "Add files…" }));
    const item = (await screen.findAllByRole("listitem"))[0]!;
    await user().click(within(item).getByRole("button", { name: "Remove" }));
    expect(screen.queryByRole("list", { name: "Attachments" })).toBeNull();
    expect(navigation.editModel?.attachmentOps).toEqual([]);
  });

  it("the_hint_is_always_shown", async () => {
    await openNew(backendWith(), "files");
    expect(screen.getByText("8 files per item, 5 MB each")).toBeTruthy();
  });

  it("limit_exceeded_error_shown_with_message", async () => {
    const fake = backendWith();
    await openNew(fake, "files");
    fake.state.pickResult = pick(...Array.from({ length: 9 }, (_, i) => `f${i}.txt`));
    await user().click(screen.getByRole("button", { name: "Add files…" }));
    await screen.findByRole("list", { name: "Attachments" });
    await user().type(screen.getByRole("textbox", { name: "Title" }), "Docs");
    await user().click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Limit reached");
    expect(screen.getByText("8 files per item, 5 MB each")).toBeTruthy();
    expect(navigation.editModel).not.toBeNull();
  });
});
