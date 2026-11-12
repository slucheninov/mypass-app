import { session } from "./stores/session.svelte";

/**
 * A secret value that exists in the webview only after an explicit `reveal` and only until the
 * first of: `hide()`, a lock (`session.lockEpoch` changes), or a newer `reveal`. A load that
 * resolves after any of those is discarded.
 */
export function createRevealed(): {
  readonly value: string | null;
  reveal(load: () => Promise<string>): Promise<void>;
  hide(): void;
} {
  let raw = $state<string | null>(null);
  let shownAtEpoch = 0;
  let generation = 0;
  let offLock: (() => void) | null = null;

  function hide() {
    generation++;
    raw = null;
    offLock?.();
    offLock = null;
  }

  return {
    // Also checks the epoch, so a lock hides the value even before the active wipe ran.
    get value() {
      return raw !== null && shownAtEpoch === session.lockEpoch ? raw : null;
    },
    async reveal(load) {
      hide();
      const mine = generation;
      const epoch = session.lockEpoch;
      const loaded = await load();
      if (mine !== generation || epoch !== session.lockEpoch) return;
      raw = loaded;
      shownAtEpoch = epoch;
      offLock = session.onLocked(hide);
    },
    hide,
  };
}
