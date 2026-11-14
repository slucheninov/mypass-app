import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../lib/api";
import type { EntryEditModel_Serialize } from "../lib/bindings";
import { navigation } from "../lib/stores/navigation.svelte";
import { toasts } from "../lib/stores/toasts.svelte";
import { calls, renderMainApp } from "../test/mainApp";
import type { FakeBackend } from "../test/fakeBackend";

afterEach(() => vi.useRealTimers());

const WAITING = "Saving from your browser is waiting";

async function openApp() {
  const app = await renderMainApp({ session: "unlocked" });
  await screen.findByRole("listbox");
  return app;
}

/** Stores a proposal on the fake and returns the event the backend would send. */
async function propose(fake: FakeBackend, id: string, title: string) {
  const model: EntryEditModel_Serialize = { ...(await api.entryNewModel("login", null)), title, password: `pw-${id}` };
  fake.state.proposals[id] = model;
  return { id, kind: "login" as const, url: "https://example.test", username: "me", entryId: null };
}

const waiting = () => toasts.items.filter((t) => t.text === WAITING);

describe("ProposalHandler", () => {
  it("proposal_event_opens_editor_with_taken_model_and_banner", async () => {
    const { fake } = await openApp();
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    const title = (await screen.findByRole("textbox", { name: "Title" })) as HTMLInputElement;
    expect(title.value).toBe("From the browser");
    expect(screen.getByTestId("proposal-banner").textContent).toContain("Saving from your browser");
    expect(calls(fake, "browserProposalTake").map((c) => c.args.id)).toEqual(["p1"]);
    expect(navigation.editorSource).toBe("browser");
  });

  it("proposal_dismiss_discards_without_saving", async () => {
    const { fake } = await openApp();
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await screen.findByTestId("proposal-banner");
    await userEvent.setup().click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(screen.queryByTestId("proposal-banner")).toBeNull());
    expect(navigation.editModel).toBeNull();
    expect(navigation.editorSource).toBeNull();
    expect(calls(fake, "entrySave")).toHaveLength(0);
  });

  it("proposal_never_replaces_a_dirty_editor_and_waits_behind_a_toast", async () => {
    const { fake } = await openApp();
    navigation.openEditor(await api.entryNewModel("login", null));
    const title = (await screen.findByRole("textbox", { name: "Title" })) as HTMLInputElement;
    await userEvent.setup().type(title, "my own draft");
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await waitFor(() => expect(waiting()).toHaveLength(1));
    // Nothing was taken, the editor is untouched and no "Discard changes?" was forced on the user.
    expect(calls(fake, "browserProposalTake")).toHaveLength(0);
    expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("my own draft");
    expect(navigation.editorSource).toBeNull();
    expect(navigation.pendingModel).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("a waiting proposal opens after the user closed the dirty editor", async () => {
    const { fake } = await openApp();
    navigation.openEditor(await api.entryNewModel("login", null));
    const user = userEvent.setup();
    await user.type((await screen.findByRole("textbox", { name: "Title" })) as HTMLInputElement, "draft");
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await waitFor(() => expect(waiting()).toHaveLength(1));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(await screen.findByRole("button", { name: "Discard" }));
    await waitFor(() => expect(calls(fake, "browserProposalTake").map((c) => c.args.id)).toEqual(["p1"]));
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("From the browser"));
    expect(waiting()).toHaveLength(0);
  });

  it("a_burst_keeps_only_the_latest_proposal_one_toast_one_editor", async () => {
    const { fake } = await openApp();
    // The first proposal opens; the editor then shows it, so the rest wait.
    await fake.emit("proposalEvent", await propose(fake, "p1", "First"));
    await screen.findByTestId("proposal-banner");
    await fake.emit("proposalEvent", await propose(fake, "p2", "Second"));
    await fake.emit("proposalEvent", await propose(fake, "p3", "Third"));
    await waitFor(() => expect(waiting()).toHaveLength(1));
    expect(screen.getAllByTestId("proposal-banner")).toHaveLength(1);
    expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("First");
    // Only after the user is done with the first one does the newest one show; p2 is never taken.
    await userEvent.setup().click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("Third"));
    expect(calls(fake, "browserProposalTake").map((c) => c.args.id)).toEqual(["p1", "p3"]);
    expect(waiting()).toHaveLength(0);
  });

  it("lock_closes_the_proposal_editor_and_the_waiting_toast", async () => {
    const { fake } = await openApp();
    await fake.emit("proposalEvent", await propose(fake, "p1", "First"));
    await screen.findByTestId("proposal-banner");
    await fake.emit("proposalEvent", await propose(fake, "p2", "Second"));
    await waitFor(() => expect(waiting()).toHaveLength(1));
    await api.vaultLock();
    await waitFor(() => expect(screen.queryByTestId("proposal-banner")).toBeNull());
    expect(navigation.editModel).toBeNull();
    expect(waiting()).toHaveLength(0);
    expect(JSON.stringify(navigation.editModel)).not.toContain("pw-p1");
  });

  const typeTitle = (text: string) => fireEvent.input(screen.getByRole("textbox", { name: "Title" }), { target: { value: text } });

  it("a_waiting_proposal_expires_with_the_backend_ttl_and_is_never_taken", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { fake } = await openApp();
    navigation.openEditor(await api.entryNewModel("login", null));
    await screen.findByRole("textbox", { name: "Title" });
    await typeTitle("my own draft");
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await waitFor(() => expect(waiting()).toHaveLength(1));
    await vi.advanceTimersByTimeAsync(121_000);
    expect(waiting()).toHaveLength(0);
    // Closing the editor later takes nothing and says nothing.
    navigation.closeEditor();
    await vi.advanceTimersByTimeAsync(1000);
    expect(calls(fake, "browserProposalTake")).toHaveLength(0);
    expect(toasts.items).toHaveLength(0);
  });

  it("a_take_that_answers_not_found_says_the_request_expired", async () => {
    const { fake } = await openApp();
    await fake.emit("proposalEvent", { id: "gone", kind: "login", url: "https://example.test", username: null, entryId: null });
    await waitFor(() => expect(toasts.items.map((x) => x.text)).toContain("This browser request has expired"));
    expect(navigation.editModel).toBeNull();
  });

  it("an_editor_that_turns_dirty_during_the_take_holds_the_model_and_asks_nothing", async () => {
    const { fake } = await openApp();
    navigation.openEditor(await api.entryNewModel("login", null));
    await screen.findByRole("textbox", { name: "Title" });
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    const take = fake.handlers.browserProposalTake;
    fake.handlers.browserProposalTake = async (args) => {
      await gate;
      return take(args);
    };
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await waitFor(() => expect(calls(fake, "browserProposalTake")).toHaveLength(1));
    await typeTitle("typed meanwhile");
    release();
    await waitFor(() => expect(waiting()).toHaveLength(1));
    // No "Discard changes?", no parked model, no banner, the draft is untouched.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(navigation.pendingModel).toBeNull();
    expect(navigation.editorSource).toBeNull();
    expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("typed meanwhile");
    // After the user acts it opens with the banner, and nothing is taken twice.
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(await screen.findByRole("button", { name: "Discard" }));
    await waitFor(() => expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("From the browser"));
    expect(screen.getByTestId("proposal-banner")).toBeTruthy();
    expect(calls(fake, "browserProposalTake")).toHaveLength(1);
    expect(waiting()).toHaveLength(0);
  });

  it("lock_overwrites_a_held_model", async () => {
    const { fake } = await openApp();
    navigation.openEditor(await api.entryNewModel("login", null));
    await screen.findByRole("textbox", { name: "Title" });
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    let taken: EntryEditModel_Serialize | undefined;
    const take = fake.handlers.browserProposalTake;
    fake.handlers.browserProposalTake = async (args) => {
      await gate;
      taken = (await take(args)) as EntryEditModel_Serialize;
      return taken;
    };
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await waitFor(() => expect(calls(fake, "browserProposalTake")).toHaveLength(1));
    await typeTitle("typed meanwhile");
    release();
    await waitFor(() => expect(waiting()).toHaveLength(1));
    expect(taken?.password).toBe("pw-p1");
    await api.vaultLock();
    await waitFor(() => expect(waiting()).toHaveLength(0));
    expect(taken?.password).toBe("");
    expect(navigation.editModel).toBeNull();
  });

  it("an_untouched_proposal_editor_counts_as_unsaved", async () => {
    const { fake } = await openApp();
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await screen.findByTestId("proposal-banner");
    expect(navigation.editorDirty?.()).toBe(true);
    await userEvent.setup().click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByRole("dialog", { name: "Discard changes?" })).toBeTruthy();
  });
});
