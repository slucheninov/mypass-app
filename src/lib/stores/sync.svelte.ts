import { api } from "../api";
import { events, type SecondaryStatus, type StorageStatus } from "../bindings";
import { session } from "./session.svelte";

const initial: StorageStatus = { location: "local", fallbackActive: false, cacheAhead: false, downloading: false };
class SyncStore {
  storage = $state<StorageStatus>({ ...initial });
  status = $state<SecondaryStatus>({ state: "off" });
  start() {
    let active = true;
    let sequence = 0;
    const offs: (() => void)[] = [];
    const refresh = async () => {
      const before = ++sequence;
      const [storage, status] = await Promise.all([api.vaultStorageStatus(), api.syncStatus()]);
      if (active && before === sequence) { this.storage = storage; this.status = status; }
    };
    offs.push(session.onChange(() => void refresh().catch(() => {})));
    void Promise.all([
      events.storageStatusChanged.listen(e => { sequence++; if (active) this.storage = e.payload; }),
      events.syncStatusChanged.listen(e => { sequence++; if (active) this.status = e.payload; }),
    ]).then(list => {
      if (!active) { list.forEach(off => off()); return; }
      offs.push(...list);
      void refresh().catch(() => {});
    }).catch(() => {});
    return () => { active = false; offs.forEach(off => off()); this.storage = { ...initial }; this.status = { state: "off" }; };
  }
}
export const sync = new SyncStore();
