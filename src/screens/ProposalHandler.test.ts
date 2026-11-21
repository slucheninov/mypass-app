import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../lib/api";
import type { BrowserProposal_Serialize, EntryEditModel_Serialize, ProposalField } from "../lib/bindings";
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
  fake.state.proposals[id] = { model, extraFields: [], address: null };
  return { id, kind: "login" as const, url: "https://example.test", username: "me", entryId: null };
}

const FORM_FIELDS: ProposalField[] = [
  { kind: "givenName", label: "First name", value: "Ann" },
  { kind: "phone", label: "Mobile", value: "+380501234567" },
  { kind: "other", label: "Nickname", value: "annie" },
  { kind: "city", label: "Town", value: "Lviv" },
];

/** A registration form proposal: extra form values and, with `withAddress`, an Address entry to offer. */
async function proposeRegistration(fake: FakeBackend, id: string, withAddress: boolean) {
  const event = await propose(fake, id, "shop.example");
  const proposal = fake.state.proposals[id];
  if (!proposal) throw new Error("no proposal stored");
  proposal.extraFields = structuredClone(FORM_FIELDS);
  if (withAddress) {
    const address = await api.entryNewModel("address", null);
    address.title = "Ann";
    if (address.address) {
      address.address.givenName = "Ann";
      address.address.city = "Lviv";
    }
    proposal.address = address;
  }
  return event;
}

/** Wraps the take so the test sees the very object the webview received. */
function captureTake(fake: FakeBackend) {
  const seen: { taken?: BrowserProposal_Serialize } = {};
  const take = fake.handlers.browserProposalTake;
  fake.handlers.browserProposalTake = async (args) => {
    seen.taken = (await take(args)) as BrowserProposal_Serialize;
    return seen.taken;
  };
  return seen;
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
    let taken: BrowserProposal_Serialize | undefined;
    const take = fake.handlers.browserProposalTake;
    fake.handlers.browserProposalTake = async (args) => {
      await gate;
      taken = (await take(args)) as BrowserProposal_Serialize;
      return taken;
    };
    await fake.emit("proposalEvent", await propose(fake, "p1", "From the browser"));
    await waitFor(() => expect(calls(fake, "browserProposalTake")).toHaveLength(1));
    await typeTitle("typed meanwhile");
    release();
    await waitFor(() => expect(waiting()).toHaveLength(1));
    expect(taken?.model.password).toBe("pw-p1");
    await api.vaultLock();
    await waitFor(() => expect(waiting()).toHaveLength(0));
    expect(taken?.model.password).toBe("");
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

  describe("registration form review", () => {
    const review = () => screen.findByRole("dialog", { name: "Review form fields" });
    const box = (name: RegExp) => screen.getByRole("checkbox", { name }) as HTMLInputElement;

    it("ticks the identity and address values, leaves the page's own fields unticked, and keeps what is ticked", async () => {
      const { fake } = await openApp();
      await fake.emit("proposalEvent", await proposeRegistration(fake, "r1", false));
      await review();
      // No editor yet: the review comes first.
      expect(navigation.editModel).toBeNull();
      // The "other" value (a label the page chose) is opt-in.
      expect(screen.getAllByRole("checkbox").map((c) => (c as HTMLInputElement).checked)).toEqual([true, true, false, true]);
      expect(screen.queryByRole("checkbox", { name: /Also create an Address entry/ })).toBeNull();
      const user = userEvent.setup();
      await user.click(box(/Phone/));
      await user.click(screen.getByRole("button", { name: "Continue" }));
      await screen.findByTestId("proposal-banner");
      expect(navigation.editorSource).toBe("browser");
      expect(navigation.editModel?.password).toBe("pw-r1");
      expect(navigation.editModel?.customFields).toEqual([
        { key: "Given Name", value: "Ann", protected: false },
        { key: "City", value: "Lviv", protected: false },
      ]);
      expect(calls(fake, "entrySave")).toHaveLength(0);
    });

    it("keeps an other value only when the user ticks it", async () => {
      const { fake } = await openApp();
      await fake.emit("proposalEvent", await proposeRegistration(fake, "r1", false));
      await review();
      const user = userEvent.setup();
      await user.click(box(/Nickname/));
      await user.click(screen.getByRole("button", { name: "Continue" }));
      await screen.findByTestId("proposal-banner");
      expect(navigation.editModel?.customFields.map((f) => f.key)).toEqual(["Given Name", "Phone", "Nickname", "City"]);
    });

    it("offers the Address entry unticked and saves it only when ticked", async () => {
      const { fake } = await openApp();
      // A copy at the time of the call, like IPC: the webview wipes its own copy after the save.
      const saved: EntryEditModel_Serialize[] = [];
      const save = fake.handlers.entrySave;
      fake.handlers.entrySave = async (args) => {
        saved.push(structuredClone((args as { model: EntryEditModel_Serialize }).model));
        return save(args);
      };
      await fake.emit("proposalEvent", await proposeRegistration(fake, "r1", true));
      await review();
      const address = box(/Also create an Address entry/);
      expect(address.checked).toBe(false);
      const user = userEvent.setup();
      await user.click(address);
      await user.click(screen.getByRole("button", { name: "Continue" }));
      await screen.findByTestId("proposal-banner");
      expect(saved).toHaveLength(1);
      expect(saved[0]?.entryType).toBe("address");
      expect(saved[0]?.address?.city).toBe("Lviv");
      // The login itself is not saved: the editor's Save does that.
      expect(navigation.editModel?.entryType).toBe("login");
    });

    it("an unticked Address entry is never saved", async () => {
      const { fake } = await openApp();
      await fake.emit("proposalEvent", await proposeRegistration(fake, "r1", true));
      await review();
      await userEvent.setup().click(screen.getByRole("button", { name: "Continue" }));
      await screen.findByTestId("proposal-banner");
      expect(calls(fake, "entrySave")).toHaveLength(0);
    });

    it("cancel overwrites every taken value and opens nothing", async () => {
      const { fake } = await openApp();
      const seen = captureTake(fake);
      await fake.emit("proposalEvent", await proposeRegistration(fake, "r1", true));
      await review();
      await userEvent.setup().click(screen.getByRole("button", { name: "Cancel" }));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(navigation.editModel).toBeNull();
      expect(seen.taken?.model.password).toBe("");
      expect(seen.taken?.extraFields.map((f) => f.value)).toEqual(["", "", "", ""]);
      expect(calls(fake, "entrySave")).toHaveLength(0);
    });

    it("an open review makes a newer proposal wait", async () => {
      const { fake } = await openApp();
      await fake.emit("proposalEvent", await proposeRegistration(fake, "r1", false));
      await review();
      await fake.emit("proposalEvent", await propose(fake, "p2", "Second"));
      await waitFor(() => expect(waiting()).toHaveLength(1));
      expect(calls(fake, "browserProposalTake").map((c) => c.args.id)).toEqual(["r1"]);
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      await waitFor(() => expect(calls(fake, "browserProposalTake").map((c) => c.args.id)).toEqual(["r1", "p2"]));
      await waitFor(() => expect((screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("Second"));
    });

    it("lock overwrites a proposal under review", async () => {
      const { fake } = await openApp();
      const seen = captureTake(fake);
      await fake.emit("proposalEvent", await proposeRegistration(fake, "r1", true));
      await review();
      await api.vaultLock();
      await waitFor(() => expect(seen.taken?.model.password).toBe(""));
      expect(seen.taken?.extraFields.every((f) => f.value === "")).toBe(true);
      expect(seen.taken?.address?.address?.city).toBe("");
    });
  });
});
