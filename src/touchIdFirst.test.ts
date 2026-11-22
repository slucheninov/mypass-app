// Touch ID first on the unlock screens (main window and tray panel): with Touch ID enabled and
// offered, the card shows the Touch ID view and prompts once by itself when the window is shown;
// the password form follows on "Use password", after three failed prompts or on a lockout.
import { screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { expectNoAxeViolations } from "./test/axe";
import { apiError, type FakeBackend } from "./test/fakeBackend";
import { calls, renderMainApp } from "./test/mainApp";
import { renderTrayPanel } from "./test/trayApp";

const NOT_RECOGNIZED = "Not recognized. Try again.";
const THREE_TIMES = "Touch ID didn’t recognize your fingerprint 3 times. Enter your master password.";

function touchId(f: FakeBackend, extra: Partial<FakeBackend["state"]["device"]> = {}) {
  Object.assign(f.state.device, { enrolled: true, touchIdEnabled: true, canOfferNow: true, reason: null, ...extra });
}

function failWith(f: FakeBackend, code: Parameters<typeof apiError>[0]) {
  f.handlers.vaultUnlockDevice = () => {
    if (code === "authenticationFailed") f.state.device.deviceFailures++;
    if (["lockout", "notEnrolledOnDevice", "deviceUnlockUnavailable"].includes(code)) {
      f.state.device.deviceBlocked = true;
    }
    throw apiError(code, "untrusted backend text");
  };
}

/** Makes the window look hidden (to the tray, minimized) or shown and focused. */
function setShown(shown: boolean) {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (shown ? "visible" : "hidden") });
  vi.spyOn(document, "hasFocus").mockReturnValue(shown);
}

function show() {
  setShown(true);
  document.dispatchEvent(new Event("visibilitychange"));
  window.dispatchEvent(new Event("focus"));
}

function hide() {
  setShown(false);
  document.dispatchEvent(new Event("visibilitychange"));
  window.dispatchEvent(new Event("blur"));
}

afterEach(() => {
  delete (document as { visibilityState?: unknown }).visibilityState;
  vi.restoreAllMocks();
});

const deviceCalls = (fake: FakeBackend) => calls(fake, "vaultUnlockDevice");

async function settle() {
  // Lets pending promises (a prompt's answer, the status refresh) run.
  for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("main window, Touch ID first", () => {
  it("touch_id_first_prompts_once_by_itself_when_the_window_is_shown", async () => {
    const { fake, container } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    expect(deviceCalls(fake)[0]?.args).toEqual({ reasonKey: "Unlock MyPass", automatic: true });
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
    expect(screen.getByText("Touch your Touch ID sensor")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Unlock with Touch ID" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Use password" })).toBeTruthy();
    expect(screen.queryByLabelText("Master password")).toBeNull();
    // A cancel shows nothing.
    expect(screen.queryByRole("alert")).toBeNull();
    await expectNoAxeViolations(container);
  });

  it("a_successful_automatic_prompt_unlocks", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => touchId(f));
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    await waitFor(() => expect(screen.queryByTestId("unlock-screen")).toBeNull());
  });

  it("a_cancel_is_not_followed_by_another_automatic_prompt_until_the_window_is_shown_anew", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    // The prompt's sheet took the focus and gave it back: no new prompt.
    window.dispatchEvent(new Event("blur"));
    window.dispatchEvent(new Event("focus"));
    await settle();
    expect(deviceCalls(fake)).toHaveLength(1);
    expect(screen.getByTestId("touch-id-view")).toBeTruthy();
    // Hidden to the tray and shown again: one new automatic prompt.
    hide();
    await settle();
    show();
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(2));
    expect(deviceCalls(fake)[1]?.args).toEqual({ reasonKey: "Unlock MyPass", automatic: true });
  });

  it("the_retry_button_prompts_again_explicitly", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    await settle();
    await userEvent.click(screen.getByRole("button", { name: "Unlock with Touch ID" }));
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(2));
    expect(deviceCalls(fake)[1]?.args).toEqual({ reasonKey: "Unlock MyPass", automatic: false });
  });

  it.each([
    ["disabled", (_f: FakeBackend): void => {}, true],
    ["unavailable", (_f: FakeBackend): void => {}, false],
    ["mac password only", (f: FakeBackend): void => touchId(f, { touchIdEnabled: false, macPasswordEnabled: true }), true],
    ["expired device window", (f: FakeBackend): void => touchId(f, { canOfferNow: false, reason: "deviceWindowExpired" }), true],
  ] as const)("no_automatic_prompt_and_todays_password_form_when_%s", async (_name, prepare, available) => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: available }, prepare);
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    await settle();
    expect(deviceCalls(fake)).toHaveLength(0);
    expect(screen.queryByTestId("touch-id-view")).toBeNull();
    expect(screen.queryByRole("button", { name: "Try Touch ID again" })).toBeNull();
  });

  it("keeps the manual device button when live biometrics are unavailable", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f, { biometrics: false, macPasswordEnabled: true });
      failWith(f, "cancelled");
    });
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect(deviceCalls(fake)).toHaveLength(0);
    await userEvent.click(await screen.findByRole("button", { name: "Unlock with Touch ID or Mac password" }));
    expect(deviceCalls(fake)[0]?.args).toEqual({ reasonKey: "Unlock MyPass", automatic: false });
  });

  it.each([
    ["main window", renderMainApp],
    ["tray panel", renderTrayPanel],
  ] as const)("%s switches to password when another surface reports the third failure", async (_surface, render) => {
    const { fake } = await render({ session: "locked", deviceUnlockAvailable: true }, (f) => {
      touchId(f, { deviceFailures: 2 });
      failWith(f, "cancelled");
    });
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
    fake.state.device.deviceFailures = 3;
    await fake.emit("deviceUnlockChanged");
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect((await screen.findByRole("alert")).textContent).toBe(THREE_TIMES);
    expect(deviceCalls(fake)).toHaveLength(1);
  });

  it("no_automatic_prompt_while_the_window_is_hidden_then_one_when_it_is_shown", async () => {
    setShown(false);
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
    await settle();
    expect(deviceCalls(fake)).toHaveLength(0);
    show();
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
  });

  it("no_automatic_prompt_while_the_window_is_visible_but_in_the_background", async () => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
    await settle();
    expect(deviceCalls(fake)).toHaveLength(0);
  });

  it("three_failed_prompts_switch_to_the_password_form", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "authenticationFailed");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    expect((await screen.findByRole("alert")).textContent).toBe(NOT_RECOGNIZED);
    await userEvent.click(screen.getByRole("button", { name: "Unlock with Touch ID" }));
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(2));
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
    await settle();
    await userEvent.click(screen.getByRole("button", { name: "Unlock with Touch ID" }));
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(3));
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect(screen.queryByTestId("touch-id-view")).toBeNull();
    expect((await screen.findByRole("alert")).textContent).toBe(THREE_TIMES);
    // Focus changes do not bring Touch ID back by themselves.
    window.dispatchEvent(new Event("focus"));
    await settle();
    expect(deviceCalls(fake)).toHaveLength(3);
    // "Try Touch ID again" returns to the Touch ID view with one explicit prompt.
    await userEvent.click(screen.getByRole("button", { name: "Try Touch ID again" }));
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(4));
    expect(deviceCalls(fake)[3]?.args).toEqual({ reasonKey: "Unlock MyPass", automatic: false });
  });

  it("the_failure_count_survives_hiding_the_window_within_one_lock_spell", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "authenticationFailed");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    for (const n of [2, 3]) {
      hide();
      await settle();
      show();
      await waitFor(() => expect(deviceCalls(fake)).toHaveLength(n));
      await settle();
    }
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    hide();
    await settle();
    show();
    await settle();
    expect(deviceCalls(fake)).toHaveLength(3);
    expect(screen.getByLabelText("Master password")).toBeTruthy();
  });

  it("use_password_switches_to_the_password_form_without_an_error", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    await userEvent.click(await screen.findByRole("button", { name: "Use password" }));
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("button", { name: "Try Touch ID again" })).toBeTruthy();
    window.dispatchEvent(new Event("focus"));
    await settle();
    expect(deviceCalls(fake)).toHaveLength(1);
  });

  it.each([
    ["lockout", "Too many attempts. Use your master password."],
    ["notEnrolledOnDevice", "No fingerprint or device password is set up on this computer"],
    ["deviceUnlockUnavailable", "Device unlock is not available on this computer"],
  ] as const)("%s_switches_straight_to_the_password_form", async (code, message) => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, code);
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect((await screen.findByRole("alert")).textContent).toBe(message);
    // Touch ID stays out of reach for this lock spell, also after the window is shown anew.
    expect(screen.queryByRole("button", { name: "Try Touch ID again" })).toBeNull();
    hide();
    await settle();
    show();
    await settle();
    expect(deviceCalls(fake)).toHaveLength(1);
  });

  it("refocusing_during_an_in_flight_prompt_never_starts_a_second_one", async () => {
    let release: (() => void) | undefined;
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      f.handlers.vaultUnlockDevice = async () => {
        await new Promise<void>((resolve) => (release = resolve));
        throw apiError("cancelled");
      };
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    hide();
    show();
    window.dispatchEvent(new Event("focus"));
    await settle();
    expect(deviceCalls(fake)).toHaveLength(1);
    expect((screen.getByRole("button", { name: "Unlock with Touch ID" }) as HTMLButtonElement).disabled).toBe(true);
    release?.();
    await waitFor(() => expect((screen.getByRole("button", { name: "Unlock with Touch ID" }) as HTMLButtonElement).disabled).toBe(false));
  });

  it("a_lock_while_the_window_is_in_front_waits_for_the_next_show", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true, session: "unlocked" }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    await settle();
    fake.state.session = { ...fake.state.session, state: "locked" };
    await fake.emit("sessionChanged", fake.state.session);
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
    await settle();
    expect(deviceCalls(fake)).toHaveLength(0);
    window.dispatchEvent(new Event("focus"));
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
  });

  it("a_new_lock_spell_resets_the_failure_count", async () => {
    const { fake } = await renderMainApp({ deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "lockout");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    fake.state.session = { ...fake.state.session, state: "unlocked" };
    await fake.emit("sessionChanged", fake.state.session);
    await waitFor(() => expect(screen.queryByTestId("unlock-screen")).toBeNull());
    failWith(fake, "cancelled");
    fake.state.device.deviceBlocked = false;
    fake.state.session = { ...fake.state.session, state: "locked" };
    await fake.emit("sessionChanged", fake.state.session);
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
  });
});

describe("tray panel, Touch ID first", () => {
  it("opening_the_locked_panel_prompts_once_by_itself", async () => {
    setShown(false);
    const { fake } = await renderTrayPanel({ session: "locked", deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    expect(await screen.findByTestId("touch-id-view")).toBeTruthy();
    await settle();
    // The panel window exists hidden from the start: no prompt before it opens.
    expect(deviceCalls(fake)).toHaveLength(0);
    show();
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    expect(deviceCalls(fake)[0]?.args).toEqual({ reasonKey: "Unlock MyPass", automatic: true });
    expect(screen.queryByLabelText("Master password")).toBeNull();
    // The prompt's sheet hides the panel; reopening it prompts again (the backend debounces).
    hide();
    await settle();
    show();
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(2));
    expect(screen.getByRole("menuitem", { name: /Settings/ })).toBeTruthy();
  });

  it("the_panel_keeps_todays_password_form_when_touch_id_is_off", async () => {
    const { fake } = await renderTrayPanel({ session: "locked", deviceUnlockAvailable: true });
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    await settle();
    expect(deviceCalls(fake)).toHaveLength(0);
    expect(screen.queryByTestId("touch-id-view")).toBeNull();
  });

  it("three_failed_prompts_in_the_panel_switch_to_the_password_form", async () => {
    const { fake } = await renderTrayPanel({ session: "locked", deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "authenticationFailed");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    for (const n of [2, 3]) {
      await settle();
      await userEvent.click(await screen.findByRole("button", { name: "Unlock with Touch ID" }));
      await waitFor(() => expect(deviceCalls(fake)).toHaveLength(n));
    }
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect((await screen.findByRole("alert")).textContent).toBe(THREE_TIMES);
  });

  it("use_password_in_the_panel_shows_the_password_form", async () => {
    const { fake } = await renderTrayPanel({ session: "locked", deviceUnlockAvailable: true }, (f) => {
      touchId(f);
      failWith(f, "cancelled");
    });
    await waitFor(() => expect(deviceCalls(fake)).toHaveLength(1));
    await userEvent.click(await screen.findByRole("button", { name: "Use password" }));
    expect(await screen.findByLabelText("Master password")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
