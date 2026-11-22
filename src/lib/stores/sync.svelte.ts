import { api } from "../api";
import { events, type SecondaryStatus, type StorageStatus } from "../bindings";
import { session } from "./session.svelte";

const initial: StorageStatus = { location: "local", fallbackActive: false, cacheAhead: false, downloading: false };
class SyncStore {
  storage = $state<StorageStatus>({ ...initial });
  status = $state<SecondaryStatus>({ state: "off" });
  /**
   * The secondary-folder status is read only while the vault is unlocked (the backend serves it
   * only then) and is forgotten on lock. The storage status feeds the main window's banner in any
   * state; `whileLocked: false` (the settings window) reads and keeps it only while unlocked too.
   */
  start({ whileLocked = true } = {}) {
    let active = true;
    let storageSequence = 0;
    let statusSequence = 0;
    const offs: (() => void)[] = [];
    const unlocked = () => session.status.state === "unlocked";
    const storageAllowed = () => whileLocked || unlocked();
    const refresh = async () => {
      const storageBefore = ++storageSequence;
      const statusBefore = ++statusSequence;
      if (!unlocked()) this.status = { state: "off" };
      if (!storageAllowed()) this.storage = { ...initial };
      await Promise.all([
        storageAllowed()
          ? api.vaultStorageStatus().then(value => { if (active && storageBefore === storageSequence && storageAllowed()) this.storage = value; })
          : null,
        unlocked()
          ? api.syncStatus().then(value => { if (active && statusBefore === statusSequence && unlocked()) this.status = value; })
          : null,
      ]);
    };
    offs.push(session.onChange(() => void refresh().catch(() => {})));
    void Promise.all([
      events.storageStatusChanged.listen(e => { storageSequence++; if (active && storageAllowed()) this.storage = e.payload; }),
      events.syncStatusChanged.listen(e => { statusSequence++; if (active && unlocked()) this.status = e.payload; }),
    ]).then(list => {
      if (!active) { list.forEach(off => off()); return; }
      offs.push(...list);
      void refresh().catch(() => {});
    }).catch(() => {});
    return () => { active = false; offs.forEach(off => off()); this.storage = { ...initial }; this.status = { state: "off" }; };
  }
}
export const sync = new SyncStore();
