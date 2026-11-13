import { afterEach, describe, expect, it, vi } from "vitest";
import { createFakeBackend } from "../../test/fakeBackend";
import { FIXTURE_LOGIN_PASSWORD, sessionStatus } from "../../test/fixtures";
import { api } from "../api";
import { entries } from "./entries.svelte";
import { startStores } from "./index";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { settings } from "./settings.svelte";
import { toasts } from "./toasts.svelte";

let stop: (() => void) | undefined;
afterEach(() => {
  stop?.();
  stop = undefined;
  toasts.clear();
});

/** Rune class fields are private with prototype accessors, so JSON.stringify of the instance would
 *  be empty: read every accessor explicitly. */
function dump(store: object): unknown {
  const proto = Object.getPrototypeOf(store) as object;
  const out: Record<string, unknown> = {};
  for (const [key, d] of Object.entries(Object.getOwnPropertyDescriptors(proto))) {
    if (d.get) out[key] = (store as Record<string, unknown>)[key];
  }
  return out;
}
const everything = () =>
  JSON.stringify([session, navigation, entries, settings, toasts].map((s) => dump(s)));

describe("secrets in the stores", () => {
  it("no_secret_in_any_store_state", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.install();
    stop = await startStores();
    await vi.waitFor(() => expect(entries.items.length).toBe(4));

    // The list and the detail never carry a password.
    await api.entryGet("e-github");
    expect(everything()).not.toContain(FIXTURE_LOGIN_PASSWORD);

    // An open editor holds the secret by design (the one store-held secret) ...
    navigation.openEditor(await api.entryEditModel("e-github"));
    expect(everything()).toContain(FIXTURE_LOGIN_PASSWORD);
    // ... and closing it removes it.
    navigation.closeEditor();
    expect(everything()).not.toContain(FIXTURE_LOGIN_PASSWORD);

    // A lock removes it as well.
    navigation.openEditor(await api.entryEditModel("e-github"));
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await vi.waitFor(() => expect(session.status.state).toBe("locked"));
    expect(everything()).not.toContain(FIXTURE_LOGIN_PASSWORD);
  });
});
