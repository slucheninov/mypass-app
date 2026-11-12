import { afterEach, describe, expect, it, vi } from "vitest";
import type { EntrySummary, Sidebar_Serialize } from "../bindings";
import { createFakeBackend } from "../../test/fakeBackend";
import { fixtureSidebar, sessionStatus } from "../../test/fixtures";
import { entries } from "./entries.svelte";
import { startStores } from "./index";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { createRevealed } from "../secret.svelte";
import { settings } from "./settings.svelte";
import { toasts } from "./toasts.svelte";

let stop: (() => void) | undefined;
afterEach(() => {
  stop?.();
  stop = undefined;
  toasts.clear();
  vi.useRealTimers();
});

const count = (fake: { calls: { command: string }[] }, command: string) =>
  fake.calls.filter((c) => c.command === command).length;

async function bootUnlocked(opts: { entries?: boolean } = {}) {
  const fake = createFakeBackend({ session: "unlocked" });
  fake.install();
  stop = await startStores(opts);
  if (opts.entries !== false) await vi.waitFor(() => expect(entries.items.length).toBe(4));
  fake.calls.length = 0;
  return fake;
}

describe("sidebar staleness", () => {
  it("stale_sidebar_response_ignored", async () => {
    const fake = await bootUnlocked();
    const real = fixtureSidebar(fake.state.entries);
    const older: Sidebar_Serialize = { ...real, rootGroupId: "older" };
    const newer: Sidebar_Serialize = { ...real, rootGroupId: "newer" };
    const pending: ((s: Sidebar_Serialize) => void)[] = [];
    fake.handlers.sidebarGet = () => new Promise((resolve) => pending.push(resolve));
    const first = entries.refresh();
    const second = entries.refresh();
    await vi.waitFor(() => expect(pending).toHaveLength(2));
    pending[1]?.(newer);
    pending[0]?.(older);
    await Promise.all([first, second]);
    expect(entries.sidebar?.rootGroupId).toBe("newer");
  });

  it("sidebar_resolving_after_lock_and_unlock_is_discarded", async () => {
    const fake = await bootUnlocked();
    const pending: ((s: Sidebar_Serialize) => void)[] = [];
    fake.handlers.sidebarGet = () => new Promise((resolve) => pending.push(resolve));
    const refreshing = entries.refresh();
    await vi.waitFor(() => expect(pending).toHaveLength(1));
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await vi.waitFor(() => expect(session.status.state).toBe("locked"));
    pending[0]?.({ ...fixtureSidebar(fake.state.entries), rootGroupId: "previous-session" });
    await refreshing;
    expect(entries.sidebar).toBeNull();
  });

  it("list_resolving_after_lock_is_discarded", async () => {
    const fake = await bootUnlocked();
    let release: ((rows: EntrySummary[]) => void) | undefined;
    fake.handlers.entryList = () => new Promise((resolve) => (release = resolve));
    const refreshing = entries.refreshList();
    await vi.waitFor(() => expect(release).toBeDefined());
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await vi.waitFor(() => expect(session.status.state).toBe("locked"));
    release?.([fake.state.entries[0]!.summary]);
    await refreshing;
    expect(entries.items).toEqual([]);
    expect(entries.loading).toBe(false);
  });
});

describe("vault switch while unlocked", () => {
  const switchedTo = (path: string) => ({ ...sessionStatus("unlocked"), vaultPath: path, vaultName: "Other" });

  it("switching_vaults_forgets_navigation_edit_state_and_revealed_values", async () => {
    const fake = await bootUnlocked();
    navigation.setScope({ kind: "category", category: "favorites" });
    navigation.select("e-github");
    navigation.openEditor({ id: "e-github", title: "A" } as never);
    const revealed = createRevealed();
    await revealed.reveal(async () => "vault-a-secret");
    expect(revealed.value).toBe("vault-a-secret");
    const epoch = session.lockEpoch;
    fake.calls.length = 0;
    await fake.emit("sessionChanged", switchedTo("/vaults/b.kdbx"));
    await vi.waitFor(() => expect(session.status.vaultPath).toBe("/vaults/b.kdbx"));
    expect(session.status.state).toBe("unlocked");
    expect(navigation.editModel).toBeNull();
    expect(navigation.presentation).toBe("browse");
    expect(navigation.selectedId).toBeNull();
    expect(navigation.scope).toEqual({ kind: "category", category: "all" });
    expect(revealed.value).toBeNull();
    expect(session.lockEpoch).toBe(epoch + 1);
    await vi.waitFor(() => expect(count(fake, "entryList")).toBeGreaterThan(0));
    expect(count(fake, "sidebarGet")).toBeGreaterThan(0);
  });

  it("a_fetch_begun_for_the_old_vault_is_discarded_after_the_switch", async () => {
    const fake = await bootUnlocked();
    let release: ((rows: EntrySummary[]) => void) | undefined;
    fake.handlers.entryList = () => new Promise((resolve) => (release = resolve));
    const refreshing = entries.refreshList();
    await vi.waitFor(() => expect(release).toBeDefined());
    fake.handlers.entryList = () => Promise.resolve([]);
    await fake.emit("sessionChanged", switchedTo("/vaults/b.kdbx"));
    await vi.waitFor(() => expect(session.status.vaultPath).toBe("/vaults/b.kdbx"));
    release?.([fake.state.entries[0]!.summary]);
    await refreshing;
    await new Promise((r) => setTimeout(r, 20));
    expect(entries.items).toEqual([]);
  });

  it("the_same_path_again_is_not_a_switch", async () => {
    await bootUnlocked();
    const epoch = session.lockEpoch;
    navigation.select("e-github");
    await session.init();
    expect(session.lockEpoch).toBe(epoch);
    expect(navigation.selectedId).toBe("e-github");
  });
});

describe("listener lifecycle", () => {
  it("stop_unsubscribes_every_listener", async () => {
    const fake = await bootUnlocked();
    stop?.();
    stop = undefined;
    fake.calls.length = 0;
    await fake.emit("sessionChanged", sessionStatus("noVault"));
    await fake.emit("vaultChanged", null);
    await fake.emit("settingsChanged", { ...fake.state.settings, theme: "dark" });
    await fake.emit("saveStateChanged", { state: "failed", message: "x" });
    await new Promise((r) => setTimeout(r, 20));
    expect(session.status.state).toBe("noVault");
    expect(session.lockEpoch).toBe(0);
    expect(settings.value).toBeNull();
    expect(toasts.items).toHaveLength(0);
    expect(fake.calls).toEqual([]);
  });

  it("start_twice_registers_one_listener_set", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.install();
    const off = await startStores();
    stop = off;
    await Promise.all([session.start(), session.start(), settings.start(), settings.start(), entries.start()]);
    await vi.waitFor(() => expect(entries.items.length).toBe(4));
    fake.calls.length = 0;
    await fake.emit("vaultChanged", null);
    await vi.waitFor(() => expect(count(fake, "entryList")).toBeGreaterThan(0));
    await new Promise((r) => setTimeout(r, 20));
    expect(count(fake, "entryList")).toBe(1);
    expect(count(fake, "sidebarGet")).toBe(1);
    await fake.emit("saveStateChanged", { state: "failed", message: "x" });
    await vi.waitFor(() => expect(toasts.items).toHaveLength(1));
  });

  it("concurrent_first_start_subscribes_once", async () => {
    const fake = createFakeBackend({ session: "locked" });
    fake.install();
    await Promise.all([session.start(), session.start()]);
    await fake.emit("saveStateChanged", { state: "failed", message: "x" });
    await vi.waitFor(() => expect(toasts.items).toHaveLength(1));
    session.stop();
    expect(toasts.items).toHaveLength(0);
    await fake.emit("saveStateChanged", { state: "failed", message: "again" });
    await new Promise((r) => setTimeout(r, 20));
    // Both subscription handles were released: nothing reacts after stop.
    expect(toasts.items).toHaveLength(0);
  });

  it("a_late_unlock_answer_does_not_overwrite_a_newer_lock_event", async () => {
    const fake = createFakeBackend({ session: "locked" });
    fake.install();
    stop = await startStores();
    let release: (() => void) | undefined;
    fake.handlers.vaultUnlock = () =>
      new Promise((resolve) => (release = () => resolve(sessionStatus("unlocked"))));
    const unlocking = session.unlock("x");
    await vi.waitFor(() => expect(release).toBeDefined());
    await fake.emit("sessionChanged", sessionStatus("noVault"));
    await vi.waitFor(() => expect(session.status.state).toBe("noVault"));
    release?.();
    await unlocking;
    expect(session.status.state).toBe("noVault");
  });
});

describe("startStores", () => {
  it("settings_window_skips_the_entries_fetch_but_keeps_lock_hooks_and_activity", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.install();
    stop = await startStores({ entries: false });
    await vi.waitFor(() => expect(session.status.state).toBe("unlocked"));
    await fake.emit("vaultChanged", null);
    await new Promise((r) => setTimeout(r, 20));
    expect(count(fake, "entryList")).toBe(0);
    expect(count(fake, "sidebarGet")).toBe(0);
    expect(entries.items).toEqual([]);
    // Lock hooks still run.
    navigation.select("e-github");
    await fake.emit("sessionChanged", sessionStatus("locked"));
    await vi.waitFor(() => expect(navigation.selectedId).toBeNull());
    expect(session.lockEpoch).toBe(1);
  });

  it("a_failing_session_does_not_stop_settings_and_is_reported", async () => {
    const fake = createFakeBackend({ session: "locked" });
    fake.state.settings.theme = "dark";
    fake.handlers.vaultStatus = () => {
      throw { code: "io", message: "unreadable" };
    };
    fake.install();
    stop = await startStores();
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(toasts.items).toHaveLength(1);
    delete document.documentElement.dataset.theme;
  });

  it("a_failing_entries_listener_does_not_stop_settings_and_session", async () => {
    const fake = createFakeBackend({ session: "locked" });
    fake.state.settings.theme = "dark";
    fake.install();
    const { events } = await import("../bindings");
    const spy = vi.spyOn(events.vaultChanged, "listen").mockRejectedValue(new Error("listen failed"));
    stop = await startStores();
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(session.loaded).toBe(true);
    expect(toasts.items).toHaveLength(1);
    spy.mockRestore();
    delete document.documentElement.dataset.theme;
  });

  it("main_ts_skips_entries_only_for_the_settings_window", async () => {
    const { readFileSync } = await import("node:fs");
    const text = readFileSync("src/main.ts", "utf8");
    expect(text).toContain('startStores({ entries: getCurrentWindow().label !== "settings" })');
  });
});
