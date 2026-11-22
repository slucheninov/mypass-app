import { api, ApiFailure, errorMessage } from "../api";
import { events, type DeviceUnlockStatus_Serialize, type SessionState } from "../bindings";
import { t } from "../i18n/index.svelte";
import { onWindowHidden } from "../secret.svelte";
import { session } from "./session.svelte";

const DEVICE_FAILURE_LIMIT = 3;

/** The unlock screen can prompt now, on its next showing, or after hiding and showing again. */
type Arm = "now" | "onShow" | "off";

function shownToUser(): boolean {
  return document.visibilityState === "visible" && document.hasFocus();
}

class DeviceStore {
  status = $state<DeviceUnlockStatus_Serialize | null>(null);
  pending = $state(false);
  #error = $state<string | null>(null);
  #method = $state<"device" | "password" | null>(null);
  #sequence = 0;
  #arm: Arm = "now";
  #attached = 0;

  get touchFirst(): boolean {
    const status = this.status;
    return session.status.state === "locked" && !!status?.canOfferNow && status.biometrics && status.touchIdEnabled;
  }

  get blocked(): boolean {
    return this.status?.deviceBlocked ?? false;
  }

  get showTouchId(): boolean {
    return this.touchFirst && !this.blocked && (this.#method ?? (
      (this.status?.deviceFailures ?? 0) >= DEVICE_FAILURE_LIMIT ? "password" : "device"
    )) === "device";
  }

  get error(): string | null {
    if (this.blocked) return this.#error ?? errorMessage(new ApiFailure("deviceUnlockUnavailable"));
    if (this.touchFirst && !this.showTouchId && (this.status?.deviceFailures ?? 0) >= DEVICE_FAILURE_LIMIT) {
      return t("Touch ID didn’t recognize your fingerprint 3 times. Enter your master password.");
    }
    return this.#error;
  }

  /** The live backend read used by every desktop unlock card. */
  #readDeviceStatus() {
    return api.deviceUnlockStatus();
  }

  async refresh(): Promise<boolean> {
    const sequence = ++this.#sequence;
    const status = await this.#readDeviceStatus();
    if (sequence !== this.#sequence) return false;
    if (status.deviceFailures >= DEVICE_FAILURE_LIMIT && status.deviceFailures > (this.status?.deviceFailures ?? 0)) {
      this.#method = "password";
    }
    this.status = status;
    return true;
  }

  async configure(password: string | null, touchId: boolean, macPassword: boolean) {
    const sequence = ++this.#sequence;
    const status = await api.deviceUnlockConfigure(password, touchId, macPassword);
    if (sequence === this.#sequence) this.status = status;
  }

  /** The backend serializes prompts across both desktop windows and the browser. */
  async unlock(automatic = false) {
    if (this.pending || this.blocked || !this.status?.canOfferNow || session.status.state !== "locked") return;
    if (automatic && !this.showTouchId) return;
    this.pending = true;
    this.#error = null;
    this.#arm = "off";
    try {
      await api.vaultUnlockDevice("Unlock MyPass", automatic);
      await session.init();
    } catch (error) {
      if (!(error instanceof ApiFailure && error.code === "cancelled")) {
        this.#error = errorMessage(error);
      }
    } finally {
      // The sheet may briefly hide the tray panel. A prompt that finishes after the window
      // was shown again uses up that showing's automatic prompt.
      if (shownToUser()) this.#arm = "off";
      await this.refresh().catch(() => {});
      this.pending = false;
    }
  }

  usePassword(): void {
    this.#method = "password";
    this.#error = null;
    this.#arm = "off";
  }

  tryDeviceAgain(): void {
    if (this.blocked) return;
    this.#method = "device";
    this.#error = null;
    void this.unlock();
  }

  #sessionChanged(next: SessionState, prev: SessionState): void {
    this.#method = null;
    this.#error = null;
    this.status = null;
    this.#arm = next === "locked" && prev === "unlocked" ? "onShow" : "now";
  }

  async #refreshForUnlockCard(): Promise<void> {
    if (await this.refresh()) this.#maybeAuto();
  }

  #maybeAuto(): void {
    if (this.#attached === 0 || this.#arm !== "now" || !this.showTouchId || this.pending || !shownToUser()) return;
    void this.unlock(true);
  }

  /** Read the current locked-vault state before deciding whether to prompt on each showing. */
  attach(): () => void {
    this.#attached++;
    const refresh = () => void this.#refreshForUnlockCard().catch(() => {});
    const onShow = () => {
      if (!shownToUser()) return;
      if (this.#arm === "onShow") this.#arm = "now";
      refresh();
    };
    const offHidden = onWindowHidden(() => {
      this.#arm = "now";
      this.#method = null;
      this.#error = null;
    });
    window.addEventListener("focus", onShow);
    document.addEventListener("visibilitychange", onShow);
    refresh();
    return () => {
      this.#attached--;
      offHidden();
      window.removeEventListener("focus", onShow);
      document.removeEventListener("visibilitychange", onShow);
    };
  }

  start() {
    let stopped = false;
    let offAttempts: (() => void) | undefined;
    void events.deviceUnlockChanged.listen(() => {
      void this.refresh().catch(() => {});
    }).then((off) => {
      if (stopped) off();
      else offAttempts = off;
    }).catch(() => {});
    const off = session.onChange((next, prev) => {
      this.#sessionChanged(next, prev);
      void this.#refreshForUnlockCard().catch(() => {});
    });
    void this.#refreshForUnlockCard().catch(() => {});
    return () => {
      stopped = true;
      offAttempts?.();
      off();
      this.#sequence++;
      this.pending = false;
      this.#sessionChanged("noVault", "noVault");
    };
  }
}
export const device = new DeviceStore();
