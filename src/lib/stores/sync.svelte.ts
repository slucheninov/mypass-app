import { api } from "../api";
import { events, type SecondaryStatus, type StorageStatus } from "../bindings";
import { session } from "./session.svelte";

const initial: StorageStatus = { location: "local", fallbackActive: false, cacheAhead: false, downloading: false };
class SyncStore {
  storage = $state<StorageStatus>({ ...initial });
  status = $state<SecondaryStatus>({ state: "off" });
  start() {
    let active = true;
    let storageSequence = 0;
    let statusSequence = 0;
    const offs: (() => void)[] = [];
    const refresh = async () => {
      const storageBefore = ++storageSequence;
      const statusBefore = ++statusSequence;
      await Promise.all([
        api.vaultStorageStatus().then(value => { if (active && storageBefore === storageSequence) this.storage = value; }),
        api.syncStatus().then(value => { if (active && statusBefore === statusSequence) this.status = value; }),
      ]);
    };
    offs.push(session.onChange(() => void refresh().catch(() => {})));
    void Promise.all([
      events.storageStatusChanged.listen(e => { storageSequence++; if (active) this.storage = e.payload; }),
      events.syncStatusChanged.listen(e => { statusSequence++; if (active) this.status = e.payload; }),
    ]).then(list => {
      if (!active) { list.forEach(off => off()); return; }
      offs.push(...list);
      void refresh().catch(() => {});
    }).catch(() => {});
    return () => { active = false; offs.forEach(off => off()); this.storage = { ...initial }; this.status = { state: "off" }; };
  }
}
export const sync = new SyncStore();
