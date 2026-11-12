import { describe, expect, it } from "vitest";
import { createFakeBackend } from "../test/fakeBackend";
import { loadPlatform, platform } from "./platform.svelte";

describe("platform store", () => {
  it("platform_store_defaults_device_unlock_false_until_app_info_loaded", async () => {
    expect(platform.deviceUnlockAvailable).toBe(false);
    expect(platform.os).toBeNull();
    const fake = createFakeBackend({ os: "windows", deviceUnlockAvailable: true });
    fake.install();
    await loadPlatform();
    expect(platform.deviceUnlockAvailable).toBe(true);
    expect(platform.os).toBe("windows");
  });
});
