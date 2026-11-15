import { api, ApiFailure, errorMessage } from "../api";
import type { DeviceUnlockStatus_Serialize } from "../bindings";
import { session } from "./session.svelte";

class DeviceStore {
  status = $state<DeviceUnlockStatus_Serialize | null>(null);
  pending = $state(false);
  error = $state<string | null>(null);
  #sequence = 0;
  async refresh() {
    const sequence = ++this.#sequence;
    const status = await api.deviceUnlockStatus();
    if (sequence === this.#sequence) this.status = status;
  }
  async configure(password: string | null, touchId: boolean, macPassword: boolean) {
    const sequence = ++this.#sequence;
    const status = await api.deviceUnlockConfigure(password, touchId, macPassword);
    if (sequence === this.#sequence) this.status = status;
  }
  async unlock() {
    if (this.pending || !this.status?.canOfferNow || session.status.state !== "locked") return;
    this.pending = true;
    this.error = null;
    try {
      await api.vaultUnlockDevice("Unlock MyPass");
      await session.init();
    } catch (e) {
      if (!(e instanceof ApiFailure && e.code === "cancelled")) this.error = errorMessage(e);
    } finally {
      this.pending = false;
      await this.refresh().catch(() => {});
    }
  }
  start() {
    const off = session.onChange(() => { this.error = null; void this.refresh().catch(() => {}); });
    void this.refresh().catch(() => {});
    return () => { off(); this.#sequence++; this.status = null; this.error = null; };
  }
}
export const device = new DeviceStore();
