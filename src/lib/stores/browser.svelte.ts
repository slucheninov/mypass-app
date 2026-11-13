import { api } from "../api";
import { events, type BrowserStatus_Serialize, type PairingChallengeDto } from "../bindings";
import { quietly } from "../unlisten";
import { session } from "./session.svelte";
import { toasts } from "./toasts.svelte";

/** Registers a backend event listener and returns the function that stops it. The registration
 *  is asynchronous: a stop before it finished drops the listener as soon as it exists. */
export function listenTo<T>(
  event: { listen(cb: (e: { payload: T }) => void): Promise<() => void> },
  cb: (payload: T) => void,
): () => void {
  let active = true;
  let off: (() => void) | null = null;
  void event
    .listen((e) => {
      if (active) cb(e.payload);
    })
    .then((fn) => {
      if (active) off = fn;
      else quietly(fn);
    })
    .catch(() => {});
  return () => {
    active = false;
    if (off) quietly(off);
    off = null;
  };
}

/**
 * The browser bridge as the UI sees it. `status` feeds the Settings -> Browser pane (any window);
 * `pairing` is the live pairing code and exists in the main window only. The pairing code is the
 * one secret here: the stored status never keeps the copy `browser_status` carries, and `pairing`
 * is nulled when the challenge clears, on cancel, on expiry and on lock.
 */
class BrowserStore {
  status = $state<BrowserStatus_Serialize | null>(null);
  /** True when the last status query failed; the pane shows nothing stale as if it were current. */
  failed = $state(false);
  /** The code waiting for the browser (main window only). */
  pairing = $state<PairingChallengeDto | null>(null);

  #seq = 0;
  #pairingSeq = 0;

  /** Re-queries the status. The newest answer wins, and the pairing code is never kept from it. */
  async refresh(): Promise<void> {
    const seq = ++this.#seq;
    try {
      const next = await api.browserStatus();
      if (seq !== this.#seq) return;
      this.status = { ...next, pairing: null };
      this.failed = false;
    } catch (e) {
      if (seq !== this.#seq) return;
      this.failed = true;
      toasts.reportError(e);
    }
  }

  /** Loads the status now and again after every `BridgeChangedEvent`. Returns the stop function. */
  watchStatus(): () => void {
    const off = listenTo(events.bridgeChangedEvent, () => void this.refresh());
    void this.refresh();
    return () => {
      off();
      this.#seq++;
      this.status = null;
    };
  }

  /**
   * Follows the pairing challenge (main window): the event, the one already active at mount, and a
   * lock, which clears it. Returns the stop function, which clears it too.
   */
  watchPairing(): () => void {
    const off = listenTo(events.pairingChallengeEvent, (challenge) => {
      this.#pairingSeq++;
      this.pairing = challenge;
    });
    const offLock = session.onLocked(() => this.clearPairing());
    void this.requeryPairing();
    return () => {
      off();
      offLock();
      this.clearPairing();
    };
  }

  clearPairing(): void {
    this.#pairingSeq++;
    this.pairing = null;
  }

  /**
   * Asks the backend which challenge is active now: a countdown that reached zero calls this, so a
   * code that was refreshed shows again and one that is gone closes the sheet. An answer whose own
   * deadline has passed is treated as gone, which also ends a loop between an old record and a clock.
   */
  async requeryPairing(): Promise<void> {
    const seq = ++this.#pairingSeq;
    try {
      const next = (await api.browserStatus()).pairing;
      // An event or a clear that landed meanwhile is newer than this answer.
      if (seq !== this.#pairingSeq) return;
      this.pairing = next && (next.expiresAt === null || next.expiresAt > Date.now()) ? next : null;
    } catch {
      if (seq === this.#pairingSeq) this.pairing = null;
    }
  }

  async cancelPairing(): Promise<void> {
    try {
      await api.browserPairingCancel();
      this.clearPairing();
    } catch (e) {
      toasts.reportError(e);
    }
  }
}

export const browser = new BrowserStore();
