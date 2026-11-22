import { api } from "../api";
import { events, type UpdateStatus } from "../bindings";
import { session } from "./session.svelte";
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

  /** `whileLocked: false` (the settings window, whose About pane shows it) reads and keeps the
   *  status only while the vault is unlocked; the banners of the other windows show it in any state. */
  start({ whileLocked = true } = {}) {
    let active = true;
    let listening = false;
    let off: (() => void) | undefined;
    const allowed = () => whileLocked || session.status.state === "unlocked";
    const load = async () => {
      if (!listening || !allowed()) return;
      const sequence = this.#sequence;
      const status = await api.updateStatus();
      if (active && sequence === this.#sequence && allowed()) this.status = status;
    };
    const offSession = whileLocked ? () => {} : session.onChange(() => {
      if (allowed()) void load().catch(e => { if (active) toasts.reportError(e); });
      else { this.#sequence++; this.status = { status: { state: "idle" }, banner: null }; }
    });
    void events.updateChanged.listen(e => {
      if (active && allowed()) { this.#sequence++; this.status = e.payload; }
    }).then(async unlisten => {
      if (!active) return unlisten();
      off = unlisten;
      listening = true;
      await load();
    }).catch(e => { if (active) toasts.reportError(e); });
    return () => {
      active = false;
      this.#sequence++;
      off?.();
      offSession();
      this.status = { status: { state: "idle" }, banner: null };
    };
  }
}
export const update = new UpdateStore();
