import { api, ApiFailure, errorMessage } from "../api";
import type { DeviceUnlockStatus_Serialize, ErrorCode, SessionState } from "../bindings";
import { t } from "../i18n/index.svelte";
import { onWindowHidden } from "../secret.svelte";
import { session } from "./session.svelte";

/** Failed Touch ID prompts in one lock spell before the unlock screen switches to the password. */
export const DEVICE_FAILURE_LIMIT = 3;

/** Outcomes after which Touch ID cannot be used for the rest of the lock spell. */
const BLOCKING: readonly ErrorCode[] = ["lockout", "notEnrolledOnDevice", "deviceUnlockUnavailable"];

/**
 * When the unlock screen may start a prompt by itself:
 * - `now`: as soon as it is shown and the status allows it (app start, another vault, the window
 *   was hidden or minimized and shows again);
 * - `onShow`: only when the window is shown or focused next (the vault just locked while the user
 *   looked at it: a manual or idle lock never prompts at once);
 * - `off`: not before the screen is shown anew (a prompt already ran, or the user chose the password).
 */
type Arm = "now" | "onShow" | "off";

/** The window is on screen and has the keyboard focus (not hidden to the tray, minimized or in the background). */
function shownToUser(): boolean {
  return document.visibilityState === "visible" && document.hasFocus();
}

/**
 * Device unlock for one window. The unlock screens (main window, tray panel) put Touch ID first:
 * when Touch ID is enabled and offered they show the Touch ID view and start one prompt by
 * themselves when shown (`attach`). The backend keeps one prompt in flight app-wide and refuses an
 * automatic prompt within two seconds of the last one ending; the counters here are per window.
 */
class DeviceStore {
  status = $state<DeviceUnlockStatus_Serialize | null>(null);
  pending = $state(false);
  error = $state<string | null>(null);
  /** The method the unlock screen shows while Touch ID first applies. */
  method = $state<"device" | "password">("device");
  /** Failed Touch ID prompts in this lock spell. */
  failures = $state(0);
  /** Touch ID is locked out or unavailable for the rest of this lock spell. */
  blocked = $state(false);
  #sequence = 0;
  #arm: Arm = "now";
  #attached = 0;
  /** Installed only by `attach` (an unlock card): the settings window never reaches a prompt. */
  #onStatus: (() => void) | null = null;

  /** Touch ID first applies: the vault is locked and Touch ID is enabled and offered now. */
  get touchFirst(): boolean {
    const s = this.status;
    return session.status.state === "locked" && !!s?.available && s.canOfferNow && s.touchIdEnabled;
  }

  /** The unlock screen shows the Touch ID view instead of the password form. */
  get showTouchId(): boolean {
    return this.touchFirst && this.method === "device" && !this.blocked;
  }

  async refresh() {
    const sequence = ++this.#sequence;
    const status = await api.deviceUnlockStatus();
    if (sequence === this.#sequence) {
      this.status = status;
      this.#onStatus?.();
    }
  }
  async configure(password: string | null, touchId: boolean, macPassword: boolean) {
    const sequence = ++this.#sequence;
    const status = await api.deviceUnlockConfigure(password, touchId, macPassword);
    if (sequence === this.#sequence) this.status = status;
  }

  /** One device prompt; `automatic` when the unlock screen starts it by itself. */
  async unlock(automatic = false) {
    if (this.pending || !this.status?.canOfferNow || session.status.state !== "locked") return;
    this.pending = true;
    this.error = null;
    // Any prompt, pressed or automatic, uses up the automatic one of this showing.
    this.#arm = "off";
    try {
      await api.vaultUnlockDevice("Unlock MyPass", automatic);
      await session.init();
    } catch (e) {
      this.#failed(e);
    } finally {
      this.pending = false;
      // A prompt that ended in front of the user used up this showing, even if the window was
      // hidden and shown meanwhile. One that ended while the window was hidden (the tray panel
      // closes when the system sheet takes the focus) leaves the next opening its own prompt.
      if (shownToUser()) this.#arm = "off";
      await this.refresh().catch(() => {});
    }
  }

  #failed(e: unknown): void {
    const code = e instanceof ApiFailure ? e.code : null;
    // A cancel (or an automatic prompt the backend did not start) shows nothing and waits for a press.
    if (code === "cancelled") return;
    if (code === "authenticationFailed") {
      this.failures++;
      if (this.touchFirst && this.failures >= DEVICE_FAILURE_LIMIT) {
        this.method = "password";
        this.error = t("Touch ID didn’t recognize your fingerprint 3 times. Enter your master password.");
        return;
      }
    } else if (code !== null && BLOCKING.includes(code)) {
      this.blocked = true;
      this.method = "password";
    }
    this.error = errorMessage(e);
  }

  /** The "Use password" link: the password form, and no automatic prompt until the screen shows anew. */
  usePassword(): void {
    this.method = "password";
    this.error = null;
    this.#arm = "off";
  }

  /** The "Try Touch ID again" link: back to the Touch ID view, with one prompt. */
  tryDeviceAgain(): void {
    if (this.blocked) return;
    this.method = "device";
    this.error = null;
    void this.unlock();
  }

  /** A new lock spell (or an unlock): Touch ID first again, with no failures counted. */
  #resetSpell(arm: Arm): void {
    this.method = "device";
    this.failures = 0;
    this.blocked = false;
    this.#arm = arm;
  }

  #sessionChanged(next: SessionState, prev: SessionState): void {
    if (next === "locked") this.#resetSpell(prev === "unlocked" ? "onShow" : "now");
    else this.#resetSpell("now");
  }

  /** The window was hidden or minimized: the screen will be shown anew. */
  #hidden(): void {
    this.#arm = "now";
    if (!this.blocked && this.failures < DEVICE_FAILURE_LIMIT && this.method === "password") {
      this.method = "device";
      this.error = null;
    }
  }

  #maybeAuto(trigger: "ready" | "show"): void {
    if (this.#attached === 0 || this.#arm === "off") return;
    if (trigger === "ready" && this.#arm !== "now") return;
    if (!this.showTouchId || this.pending || !shownToUser()) return;
    void this.unlock(true);
  }

  /**
   * Called by a mounted unlock card: starts the automatic Touch ID prompt when the window is shown
   * (now, on focus, or when it becomes visible again). Returns the detach function.
   */
  attach(): () => void {
    this.#attached++;
    this.#onStatus = () => this.#maybeAuto("ready");
    const onShow = () => this.#maybeAuto("show");
    const onVisibility = () => {
      if (document.visibilityState === "visible") onShow();
    };
    // Hidden (to the tray, a closed panel) or minimized.
    const offHidden = onWindowHidden(() => this.#hidden());
    window.addEventListener("focus", onShow);
    document.addEventListener("visibilitychange", onVisibility);
    this.#maybeAuto("ready");
    return () => {
      if (--this.#attached === 0) this.#onStatus = null;
      offHidden();
      window.removeEventListener("focus", onShow);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }

  start() {
    const off = session.onChange((next, prev) => {
      this.error = null;
      this.#sessionChanged(next, prev);
      void this.refresh().catch(() => {});
    });
    void this.refresh().catch(() => {});
    return () => {
      off();
      this.#sequence++;
      this.status = null;
      this.error = null;
      this.pending = false;
      this.#resetSpell("now");
    };
  }
}
export const device = new DeviceStore();
