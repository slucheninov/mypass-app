import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { navigation } from "../lib/stores/navigation.svelte";
import { backendWith, openEdit, openNew, user } from "../test/editorHelpers";
import { sessionStatus } from "../test/fixtures";
import { calls } from "../test/mainApp";

afterEach(() => vi.useRealTimers());

const NOTE = { id: "e-note", title: "Memo", entryType: "notes" as const, notes: "original", modified: 1 };
const body = () => screen.getByRole("textbox", { name: "Notes" }) as HTMLTextAreaElement;

async function typeNote(text: string) {
  body().value = text;
  body().setSelectionRange(text.length, text.length);
  await fireEvent.input(body());
}

describe("draft autosave", () => {
  it("notes_editor_autosaves_draft_after_3s_idle", async () => {
    const fake = backendWith([NOTE]);
    await openEdit(fake, "e-note");
    vi.useFakeTimers();
    await typeNote("original one");
    await vi.advanceTimersByTimeAsync(2000);
    await typeNote("original one two");
    await vi.advanceTimersByTimeAsync(2999);
    expect(calls(fake, "draftSave")).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(2);
    const saves = calls(fake, "draftSave");
    expect(saves).toHaveLength(1);
    // Only the entry id and the note text cross; never the password or any other field.
    expect(saves[0]?.args).toEqual({ entryId: "e-note", notes: "original one two" });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(calls(fake, "draftSave")).toHaveLength(1);
  });

  it("no_draft_is_saved_for_an_unsaved_new_note_or_an_unchanged_text", async () => {
    const fake = backendWith();
    await openNew(fake, "notes");
    vi.useFakeTimers();
    await typeNote("brand new");
    await vi.advanceTimersByTimeAsync(5000);
    expect(calls(fake, "draftSave")).toHaveLength(0);
    expect(calls(fake, "draftVersions")).toHaveLength(0);
    vi.useRealTimers();
  });

  it("closing_the_editor_cancels_the_pending_draft_timer", async () => {
    const fake = backendWith([NOTE]);
    await openEdit(fake, "e-note");
    vi.useFakeTimers();
    await typeNote("changed");
    navigation.closeEditor();
    await vi.advanceTimersByTimeAsync(5000);
    expect(calls(fake, "draftSave")).toHaveLength(0);
  });

  it("a_successful_save_clears_the_drafts_of_the_entry", async () => {
    const fake = backendWith([NOTE]);
    fake.state.drafts["e-note"] = [{ id: "d1", savedAt: 1_700_000_000_000, notes: "x" }];
    await openEdit(fake, "e-note");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "draftClear")).toHaveLength(1));
    expect(calls(fake, "draftClear")[0]?.args).toEqual({ entryId: "e-note" });
    expect(fake.state.drafts["e-note"]).toBeUndefined();
  });
});

describe("draft write racing the save", () => {
  it("a_draft_write_in_flight_when_save_is_pressed_does_not_recreate_the_draft", async () => {
    const fake = backendWith([NOTE]);
    let release: () => void = () => undefined;
    const gate = new Promise<void>((r) => (release = r));
    const original = fake.handlers.draftSave;
    fake.handlers.draftSave = async (args) => {
      await gate;
      return original(args);
    };
    await openEdit(fake, "e-note");
    vi.useFakeTimers();
    await typeNote("changed");
    await vi.advanceTimersByTimeAsync(3100);
    vi.useRealTimers();
    expect(calls(fake, "draftSave")).toHaveLength(1);
    await user().click(screen.getByRole("button", { name: "Save" }));
    release();
    await waitFor(() => expect(navigation.editModel).toBeNull());
    await waitFor(() => expect(calls(fake, "draftClear").length).toBeGreaterThan(0));
    await new Promise((r) => setTimeout(r, 20));
    expect(fake.state.drafts["e-note"]).toBeUndefined();
  });
});

describe("draft banner and versions", () => {
  const older = [
    { id: "d1", savedAt: 1_700_000_000_000, notes: "first draft" },
    { id: "d2", savedAt: 1_700_000_100_000, notes: "newest draft" },
  ];

  it("draft_banner_offered_when_versions_exist_and_restore_fills_editor", async () => {
    const fake = backendWith([NOTE]);
    fake.state.drafts["e-note"] = structuredClone(older);
    await openEdit(fake, "e-note");
    const banner = await screen.findByRole("region", { name: "Draft found" });
    await user().click(within(banner).getByRole("button", { name: "Restore draft" }));
    expect(body().value).toBe("newest draft");
    expect(screen.queryByRole("region", { name: "Draft found" })).toBeNull();
    // The restored text is a change: leaving asks first.
    await user().click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByRole("dialog", { name: "Discard changes?" })).toBeTruthy();
  });

  it("dismiss_clears_the_drafts_and_hides_the_banner", async () => {
    const fake = backendWith([NOTE]);
    fake.state.drafts["e-note"] = structuredClone(older);
    await openEdit(fake, "e-note");
    await user().click(await screen.findByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("region", { name: "Draft found" })).toBeNull();
    expect(calls(fake, "draftClear")).toHaveLength(1);
    expect(body().value).toBe("original");
  });

  it("the_versions_sheet_lists_drafts_with_their_date_and_restores_the_chosen_one", async () => {
    const fake = backendWith([NOTE]);
    fake.state.drafts["e-note"] = structuredClone(older);
    await openEdit(fake, "e-note");
    await user().click(await screen.findByRole("button", { name: "Drafts" }));
    const sheet = await screen.findByRole("dialog", { name: "Drafts" });
    const items = within(sheet).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    // Newest first.
    expect(items[0]?.textContent).toContain("newest draft");
    await user().click(within(items[1]!).getByRole("button", { name: "Restore draft" }));
    expect(body().value).toBe("first draft");
    expect(screen.queryByRole("dialog", { name: "Drafts" })).toBeNull();
  });

  it("no_banner_without_versions", async () => {
    const fake = backendWith([NOTE]);
    await openEdit(fake, "e-note");
    await waitFor(() => expect(calls(fake, "draftVersions")).toHaveLength(1));
    expect(screen.queryByRole("region", { name: "Draft found" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Drafts" })).toBeNull();
  });

  it("drafts_unavailable_hides_banner_and_save_still_works", async () => {
    const fake = backendWith([NOTE]);
    fake.state.drafts["e-note"] = structuredClone(older);
    fake.state.draftsAvailable = false;
    await openEdit(fake, "e-note");
    await screen.findByRole("region", { name: "Draft found" });
    vi.useFakeTimers();
    await typeNote("changed");
    await vi.advanceTimersByTimeAsync(3100);
    vi.useRealTimers();
    expect(calls(fake, "draftSave")).toHaveLength(1);
    await waitFor(() => expect(screen.queryByRole("region", { name: "Draft found" })).toBeNull());
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(navigation.editModel).toBeNull());
    expect(fake.state.entries.find((e) => e.summary.id === "e-note")?.model.notes).toBe("changed");
  });

  it("draft_versions_are_dropped_when_the_vault_locks", async () => {
    const fake = backendWith([NOTE]);
    fake.state.drafts["e-note"] = structuredClone(older);
    await openEdit(fake, "e-note");
    await screen.findByRole("region", { name: "Draft found" });
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await waitFor(() => expect(navigation.editModel).toBeNull());
    expect(document.body.textContent).not.toContain("newest draft");
    expect(JSON.stringify(navigation)).not.toContain("newest draft");
  });
});

describe("notes form", () => {
  it("the_note_body_is_edited_with_the_markdown_editor_and_saved", async () => {
    const fake = backendWith([NOTE]);
    await openEdit(fake, "e-note");
    expect(screen.getByRole("button", { name: "Bold" })).toBeTruthy();
    await typeNote("edited body");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(calls(fake, "entrySave")[0]?.args.model.notes).toBe("edited body");
  });
});
