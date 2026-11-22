import { api, ApiFailure, errorMessage } from "../api";
import type { DeviceUnlockOffer_Serialize, DeviceUnlockStatus_Serialize } from "../bindings";
import { session } from "./session.svelte";

/**
 * Device unlock as the UI sees it. `offer` is what an unlock screen renders (the button and its
 * label), read in any session state without side effects (`device_unlock_offer`). `status` is the
 * full state for the Security pane (toggles, enrolment, block reason), served only while unlocked.
 * Every answer is tagged with the lock epoch it was asked in, so one from before a lock never shows.
 */
class DeviceStore {
  offer = $state<DeviceUnlockOffer_Serialize | null>(null);
  status = $state<DeviceUnlockStatus_Serialize | null>(null);
  pending = $state(false);
  error = $state<string | null>(null);
  #offerSequence = 0;
  #sequence = 0;
  async refreshOffer() {
    const sequence = ++this.#offerSequence;
    const offer = await api.deviceUnlockOffer();
    if (sequence === this.#offerSequence) this.offer = offer;
  }
  /** The full status (Security pane); nothing is read or kept unless the vault is unlocked. */
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
    if (sequence === this.#sequence && epoch === session.lockEpoch) this.status = status;
  }
  async unlock() {
    if (this.pending || !this.offer?.canOfferNow || session.status.state !== "locked") return;
    this.pending = true;
    this.error = null;
    try {
      await api.vaultUnlockDevice("Unlock MyPass");
      await session.init();
    } catch (e) {
      if (!(e instanceof ApiFailure && e.code === "cancelled")) this.error = errorMessage(e);
    } finally {
      this.pending = false;
      await this.refreshOffer().catch(() => {});
    }
  }
  /** The unlock screens (`whileLocked`, the default) follow the offer in every state; the
   *  settings window (`whileLocked: false`) reads the full status only while unlocked and forgets
   *  it on lock. */
  start({ whileLocked = true } = {}) {
    const follow = () => {
      this.error = null;
      if (whileLocked) void this.refreshOffer().catch(() => {});
      else if (session.status.state === "unlocked") void this.refresh().catch(() => {});
      else { this.#sequence++; this.status = null; }
    };
    const off = session.onChange(follow);
    follow();
    return () => {
      off();
      this.#sequence++;
      this.#offerSequence++;
      this.status = null;
      this.offer = null;
      this.error = null;
    };
  }
}
export const device = new DeviceStore();
