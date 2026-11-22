import { api, ApiFailure, errorMessage } from "../api";
import { events, type DeviceUnlockOffer_Serialize, type DeviceUnlockStatus_Serialize, type SessionState } from "../bindings";
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
  // Unlock cards read the offer. The Security pane reads the gated status.
  offer = $state<DeviceUnlockOffer_Serialize | null>(null);
  status = $state<DeviceUnlockStatus_Serialize | null>(null);
  pending = $state(false);
  #error = $state<string | null>(null);
  #method = $state<"device" | "password" | null>(null);
  #sequence = 0;
  #offerSequence = 0;
  #arm: Arm = "now";
  #attached = 0;

  get touchFirst(): boolean {
    return session.status.state === "locked" && (this.offer?.touchIdAvailable ?? false);
  }

  get blocked(): boolean {
    return this.offer?.deviceBlocked ?? false;
  }

  get showTouchId(): boolean {
    return this.touchFirst && !this.blocked && (this.#method ?? (
      (this.offer?.deviceFailures ?? 0) >= DEVICE_FAILURE_LIMIT ? "password" : "device"
    )) === "device";
  }

  get error(): string | null {
    if (this.blocked) return this.#error ?? errorMessage(new ApiFailure("deviceUnlockUnavailable"));
    if (this.touchFirst && !this.showTouchId && (this.offer?.deviceFailures ?? 0) >= DEVICE_FAILURE_LIMIT) {
      return t("Touch ID didn’t recognize your fingerprint 3 times. Enter your master password.");
    }
    return this.#error;
  }

  /** The live, read-only offer used by the desktop unlock cards. */
  async refreshOffer(): Promise<boolean> {
    const sequence = ++this.#offerSequence;
    const path = session.status.vaultPath;
    if (session.status.state !== "locked") return false;
    const offer = await api.deviceUnlockOffer();
    if (sequence !== this.#offerSequence || session.status.state !== "locked" || session.status.vaultPath !== path) return false;
    if (offer.deviceFailures >= DEVICE_FAILURE_LIMIT && offer.deviceFailures > (this.offer?.deviceFailures ?? 0)) {
      this.#method = "password";
    }
    this.offer = offer;
    return true;
  }

  /** The Security pane drops full status on lock and discards stale replies. */
  async refresh() {
    const sequence = ++this.#sequence;
    const epoch = session.lockEpoch;
    if (session.status.state !== "unlocked") { this.status = null; return; }
    const status = await api.deviceUnlockStatus();
    if (sequence === this.#sequence && epoch === session.lockEpoch && session.status.state === "unlocked") this.status = status;
  }

  async configure(password: string | null, touchId: boolean, macPassword: boolean) {
    const sequence = ++this.#sequence;
    const epoch = session.lockEpoch;
    const status = await api.deviceUnlockConfigure(password, touchId, macPassword);
    if (sequence === this.#sequence && epoch === session.lockEpoch && session.status.state === "unlocked") this.status = status;
  }

  /** The backend serializes prompts across both desktop windows and the browser. */
  async unlock(automatic = false) {
    if (this.pending || this.blocked || !this.offer?.canOfferNow || session.status.state !== "locked") return;
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
      await this.refreshOffer().catch(() => {});
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
    this.offer = null;
    this.status = null;
    this.#offerSequence++;
    this.#sequence++;
    this.#arm = next === "locked" && prev === "unlocked" ? "onShow" : "now";
  }

  async #refreshForUnlockCard(): Promise<void> {
    if (await this.refreshOffer()) this.#maybeAuto();
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

  start({ whileLocked = true } = {}) {
    let stopped = false;
    let offAttempts: (() => void) | undefined;
    if (whileLocked) void events.deviceUnlockChanged.listen(() => {
      void this.refreshOffer().catch(() => {});
    }).then((off) => {
      if (stopped) off();
      else offAttempts = off;
    }).catch(() => {});
    const follow = () => {
      if (whileLocked) void this.#refreshForUnlockCard().catch(() => {});
      else void this.refresh().catch(() => {});
    };
    const off = session.onChange((next, prev) => {
      this.#sessionChanged(next, prev);
      follow();
    });
    follow();
    return () => {
      stopped = true;
      offAttempts?.();
      off();
      this.pending = false;
      this.#sessionChanged("noVault", "noVault");
    };
  }
}
export const device = new DeviceStore();
