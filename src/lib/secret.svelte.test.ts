import { emit } from "@tauri-apps/api/event";
import { mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createFakeBackend } from "../test/fakeBackend";
import { createRevealed, createRevealGroup, onWindowHidden } from "./secret.svelte";
import { session } from "./stores/session.svelte";

afterEach(() => session.stop());

describe("createRevealed", () => {
  it("reveal_stores_the_loaded_value_and_hide_clears_it", async () => {
    const r = createRevealed();
    expect(r.value).toBeNull();
    await r.reveal(async () => "s3cret");
    expect(r.value).toBe("s3cret");
    r.hide();
    expect(r.value).toBeNull();
  });

  it("createRevealed_clears_when_lock_epoch_changes", async () => {
    const r = createRevealed();
    await r.reveal(async () => "s3cret");
    session.lockEpoch += 1;
    expect(r.value).toBeNull();
  });

  it("createRevealed_clears_on_a_real_lock_transition", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.install();
    await session.start();
    const r = createRevealed();
    await r.reveal(async () => "s3cret");
    await session.lock();
    expect(r.value).toBeNull();
  });

  it("createRevealed_clears_on_hide_and_ignores_late_response_after_hide", async () => {
    const r = createRevealed();
    let finish: ((v: string) => void) | undefined;
    const pending = r.reveal(() => new Promise<string>((resolve) => (finish = resolve)));
    await vi.waitFor(() => expect(finish).toBeDefined());
    r.hide();
    finish?.("late");
    await pending;
    expect(r.value).toBeNull();
  });

  it("a_reveal_that_resolves_after_a_lock_is_discarded", async () => {
    const r = createRevealed();
    let finish: ((v: string) => void) | undefined;
    const pending = r.reveal(() => new Promise<string>((resolve) => (finish = resolve)));
    await vi.waitFor(() => expect(finish).toBeDefined());
    session.lockEpoch += 1;
    finish?.("late");
    await pending;
    expect(r.value).toBeNull();
  });

  it("a_failing_load_leaves_nothing_revealed_and_rejects", async () => {
    const r = createRevealed();
    await expect(r.reveal(async () => Promise.reject(new Error("x")))).rejects.toThrow("x");
    expect(r.value).toBeNull();
  });
});

function hideDocument(hidden: boolean) {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (hidden ? "hidden" : "visible") });
  document.dispatchEvent(new Event("visibilitychange"));
}

afterEach(() => hideDocument(false));

describe("createRevealGroup", () => {
  it("revealing_one_member_hides_the_others", async () => {
    const group = createRevealGroup();
    const a = group.create();
    const b = group.create();
    await a.reveal(async () => "alpha");
    expect(a.value).toBe("alpha");
    await b.reveal(async () => "beta");
    expect(a.value).toBeNull();
    expect(b.value).toBe("beta");
    group.destroy();
  });

  it("a_slow_reveal_loses_to_a_later_reveal_of_another_member", async () => {
    const group = createRevealGroup();
    const a = group.create();
    const b = group.create();
    let finish: ((v: string) => void) | undefined;
    const pending = a.reveal(() => new Promise<string>((resolve) => (finish = resolve)));
    await vi.waitFor(() => expect(finish).toBeDefined());
    await b.reveal(async () => "beta");
    finish?.("alpha");
    await pending;
    expect(a.value).toBeNull();
    expect(b.value).toBe("beta");
    group.destroy();
  });

  it("hide_all_and_destroy_leave_no_value", async () => {
    const group = createRevealGroup();
    const a = group.create();
    await a.reveal(async () => "alpha");
    group.hideAll();
    expect(a.value).toBeNull();
    await a.reveal(async () => "alpha");
    group.destroy();
    expect(a.value).toBeNull();
  });

  it("a_hidden_document_hides_every_member", async () => {
    const group = createRevealGroup();
    const a = group.create();
    await a.reveal(async () => "alpha");
    hideDocument(true);
    expect(a.value).toBeNull();
    group.destroy();
  });
});

describe("a minimised window", () => {
  it("a_minimised_window_leaves_no_member_value", async () => {
    mockWindows("main");
    let minimized = false;
    createFakeBackend({ plugins: { "plugin:window|is_minimized": () => minimized } }).install();
    const group = createRevealGroup();
    const a = group.create();
    const b = group.create();
    await a.reveal(async () => "alpha");
    await new Promise((r) => setTimeout(r, 10));
    await emit("tauri://resize", { width: 800, height: 600 });
    await new Promise((r) => setTimeout(r, 10));
    expect(a.value).toBe("alpha");
    minimized = true;
    await emit("tauri://resize", { width: 800, height: 600 });
    await vi.waitFor(() => expect(a.value).toBeNull());
    expect(b.value).toBeNull();
    group.destroy();
  });

  it("a_burst_of_resizes_asks_the_window_once_at_a_time", async () => {
    mockWindows("main");
    let inFlight = 0;
    let most = 0;
    let queries = 0;
    createFakeBackend({
      plugins: {
        "plugin:window|is_minimized": async () => {
          queries++;
          most = Math.max(most, ++inFlight);
          await new Promise((r) => setTimeout(r, 20));
          inFlight--;
          return false;
        },
      },
    }).install();
    const off = onWindowHidden(() => undefined);
    await new Promise((r) => setTimeout(r, 10));
    for (let i = 0; i < 10; i++) await emit("tauri://resize", { width: 800 + i, height: 600 });
    await new Promise((r) => setTimeout(r, 60));
    expect(most).toBe(1);
    expect(queries).toBeLessThan(10);
    off();
  });
});

describe("onWindowHidden", () => {
  it("fires_when_the_document_becomes_hidden_and_not_when_it_becomes_visible", () => {
    const cb = vi.fn();
    const off = onWindowHidden(cb);
    hideDocument(false);
    expect(cb).not.toHaveBeenCalled();
    hideDocument(true);
    expect(cb).toHaveBeenCalledTimes(1);
    off();
    hideDocument(true);
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("fires_when_the_window_reports_minimised_after_a_resize", async () => {
    mockWindows("main");
    let minimized = false;
    createFakeBackend({ plugins: { "plugin:window|is_minimized": () => minimized } }).install();
    const cb = vi.fn();
    const off = onWindowHidden(cb);
    await vi.waitFor(() => expect(cb).not.toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 10));
    await emit("tauri://resize", { width: 800, height: 600 });
    await new Promise((r) => setTimeout(r, 10));
    expect(cb).not.toHaveBeenCalled();
    minimized = true;
    await emit("tauri://resize", { width: 800, height: 600 });
    await vi.waitFor(() => expect(cb).toHaveBeenCalledTimes(1));
    off();
    await emit("tauri://resize", { width: 800, height: 600 });
    await new Promise((r) => setTimeout(r, 10));
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("works_without_a_tauri_window", () => {
    const cb = vi.fn();
    expect(() => onWindowHidden(cb)()).not.toThrow();
  });
});
