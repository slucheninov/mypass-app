import type { SessionState, SessionStatus_Serialize } from "../bindings";
import { api } from "../api";
import { events } from "../bindings";
import { t } from "../i18n/index.svelte";
import { toasts } from "./toasts.svelte";

const INITIAL: SessionStatus_Serialize = {
  state: "noVault",
  vaultPath: null,
  vaultName: null,
  lockReason: null,
  fileStatus: { kind: "noVault" },
  save: { state: "idle", message: null },
};

type Listener = (next: SessionState, prev: SessionState) => void;

/** The session as the backend reports it. Everything that must forget a vault on lock (entries,
 *  navigation, revealed values) hangs off `onChange` / `lockEpoch`; this store imports none of them. */
class SessionStore {
  status = $state<SessionStatus_Serialize>({ ...INITIAL });
  /** False until the first `vault_status` answer (or event) arrived. */
  loaded = $state(false);
  /** Incremented once whenever an unlocked session stops being unlocked. */
  lockEpoch = $state(0);

  #listeners = new Set<Listener>();
  #unlisten: (() => void)[] = [];
  /** Bumped by every event, so a slower `vault_status` answer never overwrites a newer event. */
  #seq = 0;
  #subscribed: Promise<void> | null = null;
  #saveToast: number | null = null;

  /** Calls `cb` whenever the session state changes (including the first load). Returns the unsubscriber. */
  onChange(cb: Listener): () => void {
    this.#listeners.add(cb);
    return () => this.#listeners.delete(cb);
  }

  /** Calls `cb` whenever an unlocked session stops being unlocked (lock, vault closed). */
  onLocked(cb: () => void): () => void {
    return this.onChange((next, prev) => {
      if (prev === "unlocked" && next !== "unlocked") cb();
    });
  }

  /** Registers the event listeners, then loads the status. Call once per window. */
  async start(): Promise<void> {
    // The promise is kept before the first await, so two concurrent calls subscribe once.
    this.#subscribed ??= this.#subscribe();
    await this.#subscribed;
    await this.init();
  }

  async #subscribe(): Promise<void> {
    const offs = await Promise.all([
      events.sessionChanged.listen((e) => {
        this.#seq++;
        this.#apply(e.payload);
      }),
      events.saveStateChanged.listen((e) => {
        this.#seq++;
        this.#applySave(e.payload);
      }),
    ]);
    this.#unlisten.push(...offs);
  }

  async init(): Promise<void> {
    const seq = this.#seq;
    const status = await api.vaultStatus();
    if (seq === this.#seq) this.#apply(status);
    else this.loaded = true;
  }

  async unlock(password: string): Promise<void> {
    const seq = ++this.#seq;
    const status = await api.vaultUnlock(password);
    // An event that arrived meanwhile is newer than this answer; it already applied the state.
    if (seq === this.#seq || status.state === this.status.state) this.#apply(status);
  }

  async lock(): Promise<void> {
    await api.vaultLock();
    // The event normally arrives first; asking again makes the result independent of its timing.
    await this.init();
  }

  /** Removes the listeners and returns to the initial state (tests and window teardown). */
  stop(): void {
    this.#subscribed = null;
    for (const off of this.#unlisten.splice(0)) off();
    this.#listeners.clear();
    if (this.#saveToast !== null) toasts.dismiss(this.#saveToast);
    this.#saveToast = null;
    this.status = { ...INITIAL };
    this.loaded = false;
    this.lockEpoch = 0;
    this.#seq = 0;
  }

  #apply(next: SessionStatus_Serialize): void {
    const prev = this.status.state;
    this.status = next;
    this.loaded = true;
    this.#syncSaveToast(next.save);
    if (prev === next.state) return;
    // Bump first, so a listener that reads the epoch already sees the lock.
    if (prev === "unlocked") this.lockEpoch++;
    for (const cb of [...this.#listeners]) cb(next.state, prev);
  }

  #applySave(save: SessionStatus_Serialize["save"]): void {
    this.status = { ...this.status, save };
    this.#syncSaveToast(save);
  }

  /** One sticky toast while saving keeps failing; the backend text goes into the details only. */
  #syncSaveToast(save: SessionStatus_Serialize["save"]): void {
    if (save.state === "failed") {
      if (this.#saveToast === null) {
        this.#saveToast = toasts.push("error", t("Saving failed. MyPass will retry."), {
          sticky: true,
          ...(save.message ? { detail: save.message } : {}),
        });
      }
    } else if (this.#saveToast !== null) {
      toasts.dismiss(this.#saveToast);
      this.#saveToast = null;
    }
  }
}

export const session = new SessionStore();
