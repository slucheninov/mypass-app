import { screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { calls } from "../../test/mainApp";
import { renderSettingsApp } from "../../test/settingsApp";

const open = (opts: Parameters<typeof renderSettingsApp>[0] = {}) => renderSettingsApp(opts, { pane: "Security" });

describe("Security", () => {
  it("idle_timeout_select_has_eight_options_default_thirty_minutes", async () => {
    const user = userEvent.setup();
    const { fake } = await open();
    const select = screen.getByRole("combobox", { name: "Lock after inactivity" }) as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => [o.value, o.textContent])).toEqual([
      ["immediately", "Immediately"],
      ["oneMinute", "1 minute"],
      ["fiveMinutes", "5 minutes"],
      ["fifteenMinutes", "15 minutes"],
      ["thirtyMinutes", "30 minutes"],
      ["oneHour", "1 hour"],
      ["fourHours", "4 hours"],
      ["never", "Never"],
    ]);
    expect(select.value).toBe("thirtyMinutes");
    await user.selectOptions(select, "fourHours");
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([{ autoLockIdleTimeout: "fourHours" }]);
    await waitFor(() => expect(select.value).toBe("fourHours"));
  });

  it("a rejected idle timeout puts the select back and shows the error", async () => {
    const user = userEvent.setup();
    const { fake } = await open();
    fake.state.settingsFailure = "internal";
    const select = screen.getByRole("combobox", { name: "Lock after inactivity" }) as HTMLSelectElement;
    await user.selectOptions(select, "never");
    await screen.findByRole("alert");
    // The select is rebuilt from the stored value.
    expect((screen.getByRole("combobox", { name: "Lock after inactivity" }) as HTMLSelectElement).value).toBe("thirtyMinutes");
  });

  it("sleep_and_screen_lock_toggles_wired", async () => {
    const user = userEvent.setup();
    const { fake, container } = await open();
    const sleep = screen.getByRole("switch", { name: "Lock when the computer sleeps" });
    const screenLock = screen.getByRole("switch", { name: "Lock when the screen locks" });
    expect([sleep, screenLock].map((s) => s.getAttribute("aria-checked"))).toEqual(["true", "true"]);
    await user.click(sleep);
    await user.click(screenLock);
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([
      { lockOnDeviceSleep: false },
      { lockOnScreenLock: false },
    ]);
    await waitFor(() => expect(sleep.getAttribute("aria-checked")).toBe("false"));
    expect(container.textContent).not.toMatch(/Mac/);
  });

  it("device_unlock_section_absent_when_unavailable", async () => {
    await open({ deviceUnlockAvailable: false });
    expect(screen.queryByTestId("device-unlock-slot")).toBeNull();
  });

  it("device_unlock_slot_mounted_when_available", async () => {
    await open({ deviceUnlockAvailable: true });
    const slot = await screen.findByTestId("device-unlock-slot");
    expect(slot.children.length).toBe(0);
    expect(slot.textContent).toBe("");
    expect(slot.querySelectorAll("button, input, select, a, [tabindex]").length).toBe(0);
  });
});
