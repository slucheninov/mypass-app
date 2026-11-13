import { afterEach, describe, expect, it, vi } from "vitest";
import { createFakeBackend } from "../../test/fakeBackend";
import { session } from "./session.svelte";
import { startActivityReporter } from "./activity.svelte";

let cleanup: (() => void) | undefined;
afterEach(() => {
  cleanup?.();
  cleanup = undefined;
  session.stop();
  vi.useRealTimers();
});

async function setup(state: "locked" | "unlocked") {
  const fake = createFakeBackend({ session: state });
  fake.install();
  await session.start();
  vi.useFakeTimers();
  cleanup = startActivityReporter();
  return fake;
}
const reports = (fake: { calls: { command: string }[] }) =>
  fake.calls.filter((c) => c.command === "appRecordActivity").length;

describe("activity reporter", () => {
  it("activity_reporter_throttles_to_one_call_per_second", async () => {
    const fake = await setup("unlocked");
    for (let i = 0; i < 10; i++) {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
      await vi.advanceTimersByTimeAsync(90);
    }
    expect(reports(fake)).toBe(1);
    await vi.advanceTimersByTimeAsync(1100);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "b" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(reports(fake)).toBe(2);
  });

  it("every_listed_input_kind_counts_as_activity", async () => {
    const fake = await setup("unlocked");
    for (const ev of [new Event("pointerdown"), new KeyboardEvent("keydown"), new Event("wheel"), new Event("focus")]) {
      window.dispatchEvent(ev);
      await vi.advanceTimersByTimeAsync(1000);
    }
    expect(reports(fake)).toBe(4);
  });

  it("activity_not_reported_when_locked", async () => {
    const fake = await setup("locked");
    window.dispatchEvent(new KeyboardEvent("keydown"));
    window.dispatchEvent(new Event("pointerdown"));
    await vi.advanceTimersByTimeAsync(2000);
    expect(reports(fake)).toBe(0);
  });

  it("a_failing_report_is_swallowed", async () => {
    const fake = await setup("unlocked");
    fake.handlers.appRecordActivity = () => {
      throw { code: "locked", message: "" };
    };
    window.dispatchEvent(new KeyboardEvent("keydown"));
    await vi.advanceTimersByTimeAsync(0);
    expect(reports(fake)).toBe(1);
  });

  it("activity_reporter_cleanup_removes_listeners", async () => {
    const fake = await setup("unlocked");
    cleanup?.();
    cleanup = undefined;
    window.dispatchEvent(new KeyboardEvent("keydown"));
    window.dispatchEvent(new Event("pointerdown"));
    await vi.advanceTimersByTimeAsync(2000);
    expect(reports(fake)).toBe(0);
  });
});
