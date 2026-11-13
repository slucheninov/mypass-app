import { afterEach, describe, expect, it, vi } from "vitest";
import { createFakeBackend } from "../test/fakeBackend";
import { createRevealed } from "./secret.svelte";
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
