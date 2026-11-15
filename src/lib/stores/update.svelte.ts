import { api } from "../api";
import { events, type UpdateStatus } from "../bindings";
import { toasts } from "./toasts.svelte";

class UpdateStore {
  status = $state<UpdateStatus>({ status: { state: "idle" }, banner: null });
  #sequence = 0;

  async check() {
    if (this.status.status.state === "checking") return;
    const sequence = ++this.#sequence;
    this.status = { ...this.status, status: { state: "checking" } };
    try {
      const status = await api.updateCheckNow();
      if (sequence === this.#sequence) this.status = status;
    } catch (e) {
      if (sequence === this.#sequence) {
        this.status = { ...this.status, status: { state: "failed", kind: "unexpected" } };
        toasts.reportError(e);
      }
    }
  }

  async dismiss() {
    const version = this.status.banner?.version;
    await api.updateDismiss();
    // A newer event may already have advertised a different release.
    if (this.status.banner?.version === version) {
      this.#sequence++;
      this.status = { ...this.status, banner: null };
    }
  }

  async open() {
    try { await api.updateOpenRelease(); }
    catch (e) { toasts.reportError(e); }
  }

  start() {
    let active = true;
    let off: (() => void) | undefined;
    void events.updateChanged.listen(e => {
      if (active) { this.#sequence++; this.status = e.payload; }
    }).then(async unlisten => {
      if (!active) return unlisten();
      off = unlisten;
      const sequence = this.#sequence;
      const status = await api.updateStatus();
      if (active && sequence === this.#sequence) this.status = status;
    }).catch(e => { if (active) toasts.reportError(e); });
    return () => {
      active = false;
      this.#sequence++;
      off?.();
      this.status = { status: { state: "idle" }, banner: null };
    };
  }
}
export const update = new UpdateStore();
