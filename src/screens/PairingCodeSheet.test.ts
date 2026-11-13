import { screen, waitFor, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { api } from "../lib/api";
import { browser } from "../lib/stores/browser.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { calls, renderMainApp } from "../test/mainApp";

const challenge = (over: Partial<{ code: string; browser: string; expiresAt: number | null }> = {}) => ({
  code: "123456",
  browser: "Chrome (Work)",
  expiresAt: Date.now() + 60_000,
  ...over,
});

async function openApp() {
  const app = await renderMainApp({ session: "unlocked" });
  await screen.findByRole("listbox");
  return app;
}

describe("PairingCodeSheet", () => {
  it("pairing_sheet_opens_on_challenge_event_shows_grouped_code_and_closes_on_clear", async () => {
    const { fake } = await openApp();
    expect(screen.queryByRole("dialog")).toBeNull();
    await fake.emit("pairingChallengeEvent", challenge());
    const dialog = await screen.findByRole("dialog", { name: "Pairing code" });
    expect(within(dialog).getByText("123 456")).toBeTruthy();
    expect(within(dialog).getByText("Chrome (Work)")).toBeTruthy();
    expect(within(dialog).getByText(/It expires in \d+ seconds\./)).toBeTruthy();
    // The code is shown, not announced.
    expect(dialog.querySelector("[aria-live]")).toBeNull();
    await fake.emit("pairingChallengeEvent", null);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("a challenge that is already active at mount shows the sheet", async () => {
    const { fake } = await renderMainApp({ session: "unlocked" }, (f) => void (f.state.browser.pairing = challenge()));
    expect(await screen.findByRole("dialog", { name: "Pairing code" })).toBeTruthy();
    expect(fake.state.browser.pairing?.code).toBe("123456");
  });

  it("pairing_sheet_cancel_calls_browser_pairing_cancel", async () => {
    const { fake } = await openApp();
    fake.state.browser.pairing = challenge();
    await fake.emit("pairingChallengeEvent", challenge());
    await userEvent.setup().click(await screen.findByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(calls(fake, "browserPairingCancel")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("pairing_code_not_present_in_any_store_after_close", async () => {
    const { fake } = await openApp();
    fake.state.browser.pairing = challenge();
    await fake.emit("pairingChallengeEvent", challenge());
    await screen.findByRole("dialog", { name: "Pairing code" });
    // The status a window fetches meanwhile never keeps the code either.
    await browser.refresh();
    expect(JSON.stringify(browser.status)).not.toContain("123456");
    await userEvent.setup().click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const stores = JSON.stringify([browser.status, browser.pairing, navigation.editModel, navigation.pendingModel]);
    expect(stores).not.toContain("123456");
    expect(document.body.innerHTML).not.toMatch(/123\s?456/);
  });

  it("the countdown reaching zero closes a stale sheet by itself", async () => {
    const { fake } = await openApp();
    // The backend still lists the challenge, but its own deadline has passed (the clear event is late).
    const soon = challenge({ expiresAt: Date.now() + 1200 });
    fake.state.browser.pairing = soon;
    await fake.emit("pairingChallengeEvent", soon);
    await screen.findByRole("dialog", { name: "Pairing code" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull(), { timeout: 4000 });
    expect(calls(fake, "browserStatus").length).toBeGreaterThanOrEqual(2);
    expect(browser.pairing).toBeNull();
  });

  it("the countdown reaching zero shows a refreshed challenge", async () => {
    const { fake } = await openApp();
    const soon = challenge({ expiresAt: Date.now() + 1200 });
    fake.state.browser.pairing = challenge({ code: "999888", expiresAt: Date.now() + 60_000 });
    await fake.emit("pairingChallengeEvent", soon);
    expect(await screen.findByText("123 456")).toBeTruthy();
    expect(await screen.findByText("999 888", undefined, { timeout: 4000 })).toBeTruthy();
  });

  it("lock_event_closes_pairing_sheet_and_proposal_editor", async () => {
    const { fake } = await openApp();
    navigation.openEditor(await api.entryNewModel("login", null), "browser");
    fake.state.browser.pairing = challenge();
    await fake.emit("pairingChallengeEvent", challenge());
    await screen.findByRole("dialog", { name: "Pairing code" });
    await api.vaultLock();
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Pairing code" })).toBeNull());
    expect(navigation.editModel).toBeNull();
    expect(navigation.editorSource).toBeNull();
    expect(browser.pairing).toBeNull();
    expect(document.body.innerHTML).not.toMatch(/123\s?456/);
  });
});
