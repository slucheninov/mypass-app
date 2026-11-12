import { afterEach, describe, expect, it } from "vitest";
import { lockGuard } from "./lock-guard";
import { session } from "./stores/session.svelte";

afterEach(() => {
  session.lockEpoch = 0;
});

describe("lockGuard", () => {
  it("holds_until_a_lock_bumps_the_epoch", () => {
    const unchanged = lockGuard();
    expect(unchanged()).toBe(true);
    session.lockEpoch++;
    expect(unchanged()).toBe(false);
    expect(lockGuard()()).toBe(true);
  });
});
