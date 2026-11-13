import { afterEach, describe, expect, it, vi } from "vitest";
import { createFakeBackend } from "../../test/fakeBackend";
import { FIXTURE_MASTER_PASSWORD, sessionStatus } from "../../test/fixtures";
import { api } from "../api";
import { __resetCatalogsForTest, setLanguage, t } from "../i18n/index.svelte";
import { createRevealed } from "../secret.svelte";
import { entries } from "./entries.svelte";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { startStores } from "./index";
import { toasts } from "./toasts.svelte";

let stop: (() => void) | undefined;
afterEach(() => {
  stop?.();
  stop = undefined;
  toasts.clear();
  vi.useRealTimers();
  __resetCatalogsForTest();
  setLanguage("en");
});

async function boot(state: "noVault" | "locked" | "unlocked") {
  const fake = createFakeBackend({ session: state });
  fake.install();
  stop = await startStores();
  return fake;
}

describe("session store", () => {
  it("session_store_initialises_from_vault_status_and_follows_events", async () => {
    const fake = await boot("locked");
    expect(session.loaded).toBe(true);
    expect(session.status.state).toBe("locked");
    await fake.emit("sessionChanged", sessionStatus("unlocked"));
    await vi.waitFor(() => expect(session.status.state).toBe("unlocked"));
  });

  it("unlock_and_lock_go_through_the_backend", async () => {
    await boot("locked");
    await expect(session.unlock("wrong")).rejects.toMatchObject({ code: "wrongPassword" });
    expect(session.status.state).toBe("locked");
    await session.unlock(FIXTURE_MASTER_PASSWORD);
    expect(session.status.state).toBe("unlocked");
    await session.lock();
    expect(session.status.state).toBe("locked");
  });

  it("unlock_loads_entries_and_sidebar", async () => {
    await boot("locked");
    await session.unlock(FIXTURE_MASTER_PASSWORD);
    await vi.waitFor(() => expect(entries.items.length).toBe(4));
    expect(entries.sidebar?.rootGroupId).toBe("group-root");
  });

  it("lock_event_clears_entries_and_selection_and_edit_models", async () => {
    const fake = await boot("unlocked");
    await vi.waitFor(() => expect(entries.items.length).toBe(4));
    navigation.select("e-github");
    navigation.openEditor(await api.entryEditModel("e-github"));
    const revealed = createRevealed();
    await revealed.reveal(() => api.entryReveal("e-github", { kind: "password" }));
    expect(revealed.value).toBe("hunter2");
    expect(navigation.editModel?.password).toBe("hunter2");
    const epoch = session.lockEpoch;

    await fake.emit("sessionChanged", sessionStatus("locked"));
    await vi.waitFor(() => expect(session.status.state).toBe("locked"));

    expect(entries.items).toEqual([]);
    expect(entries.sidebar).toBeNull();
    expect(navigation.selectedId).toBeNull();
    expect(navigation.editModel).toBeNull();
    expect(navigation.presentation).toBe("browse");
    expect(revealed.value).toBeNull();
    expect(session.lockEpoch).toBe(epoch + 1);

    await fake.emit("sessionChanged", sessionStatus("locked"));
    await vi.waitFor(() => expect(session.lockEpoch).toBe(epoch + 1));
  });

  it("a_late_status_answer_does_not_overwrite_a_newer_event", async () => {
    const fake = createFakeBackend({ session: "locked" });
    let release: (() => void) | undefined;
    fake.handlers.vaultStatus = () =>
      new Promise((resolve) => {
        release = () => resolve(sessionStatus("locked"));
      });
    fake.install();
    const starting = session.start();
    await vi.waitFor(() => expect(release).toBeDefined());
    await fake.emit("sessionChanged", sessionStatus("unlocked"));
    await vi.waitFor(() => expect(session.status.state).toBe("unlocked"));
    release?.();
    await starting;
    expect(session.status.state).toBe("unlocked");
    session.stop();
  });

  it("save_state_changed_failed_pushes_sticky_error_toast", async () => {
    const fake = await boot("unlocked");
    await fake.emit("saveStateChanged", { state: "failed", message: "disk full: /Users/x/vault.kdbx" });
    await vi.waitFor(() => expect(toasts.items).toHaveLength(1));
    const toast = toasts.items[0];
    expect(toast).toMatchObject({ kind: "error", sticky: true, text: t("Saving failed. MyPass will retry.") });
    expect(toast?.text).not.toContain("disk full");
    expect(toast?.detail).toBe("disk full: /Users/x/vault.kdbx");
    // A repeated failure does not stack toasts; recovery removes it.
    await fake.emit("saveStateChanged", { state: "failed", message: "disk full again" });
    await fake.emit("saveStateChanged", { state: "idle", message: null });
    await vi.waitFor(() => expect(toasts.items).toHaveLength(0));
    expect(session.status.save.state).toBe("idle");
  });
});
