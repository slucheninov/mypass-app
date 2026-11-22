import { api } from "../api";
import { events, type SecondaryStatus, type StorageStatus } from "../bindings";
import { session } from "./session.svelte";

const initial: StorageStatus = { location: "local", fallbackActive: false, cacheAhead: false, downloading: false };
class SyncStore {
  storage = $state<StorageStatus>({ ...initial });
  status = $state<SecondaryStatus>({ state: "off" });
  /** Takes a status a pane action answered, unless the vault locked since `epoch` (the action's
   *  start): an answer from before a lock never shows in the next session. */
  adopt(status: SecondaryStatus, epoch: number): boolean {
    if (session.status.state !== "unlocked" || session.lockEpoch !== epoch) return false;
    this.#statusSequence++;
    this.status = status;
    return true;
  }
  #statusSequence = 0;
  /**
   * The secondary-folder status is read only while the vault is unlocked (the backend serves it
   * only then) and is forgotten on lock. The storage status feeds the main window's banner in any
   * state; `whileLocked: false` (the settings window) reads and keeps it only while unlocked too.
   */
  start({ whileLocked = true } = {}) {
    let active = true;
    let storageSequence = 0;
    const offs: (() => void)[] = [];
    const unlocked = () => session.status.state === "unlocked";
    const storageAllowed = () => whileLocked || unlocked();
    const refreshStatus = async () => {
      const sequence = ++this.#statusSequence;
      const epoch = session.lockEpoch;
      if (!unlocked()) {
        this.status = { state: "off" };
        return;
      }
      const value = await api.syncStatus();
      if (active && sequence === this.#statusSequence) this.adopt(value, epoch);
    };
    const refresh = async () => {
      const storageBefore = ++storageSequence;
      if (!storageAllowed()) this.storage = { ...initial };
      await Promise.all([
        storageAllowed()
          ? api.vaultStorageStatus().then(value => { if (active && storageBefore === storageSequence && storageAllowed()) this.storage = value; })
          : null,
        refreshStatus(),
      ]);
    };
    offs.push(session.onChange(() => void refresh().catch(() => {})));
    void Promise.all([
      events.storageStatusChanged.listen(e => { storageSequence++; if (active && storageAllowed()) this.storage = e.payload; }),
      // Payload-free: the status is re-read through the gated `sync_status`, only while unlocked.
      events.syncStatusChanged.listen(() => { if (active && unlocked()) void refreshStatus().catch(() => {}); }),
    ]).then(list => {
      if (!active) { list.forEach(off => off()); return; }
      offs.push(...list);
      void refresh().catch(() => {});
    }).catch(() => {});
    return () => { active = false; offs.forEach(off => off()); this.storage = { ...initial }; this.status = { state: "off" }; };
  }
}
export const sync = new SyncStore();
