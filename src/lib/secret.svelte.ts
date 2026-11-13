import { getCurrentWindow } from "@tauri-apps/api/window";
import { session } from "./stores/session.svelte";

/**
 * A secret value that exists in the webview only after an explicit `reveal` and only until the
 * first of: `hide()`, a lock (`session.lockEpoch` changes), or a newer `reveal`. A load that
 * resolves after any of those is discarded. The owning component must call `hide()` when it is
 * destroyed (an `$effect` cleanup), or the value and the lock subscription outlive it until a lock.
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

export type Revealed = ReturnType<typeof createRevealed>;

/**
 * Calls `cb` when the window stops being visible: the document becomes hidden, or the Tauri
 * window reports itself minimised after a resize (WKWebView does not always fire
 * `visibilitychange` for a minimise). Returns the unsubscriber. Outside Tauri only the document
 * event is available.
 */
export function onWindowHidden(cb: () => void): () => void {
  let active = true;
  let unlisten: (() => void) | null = null;
  // A live resize fires dozens of events a second; one `isMinimized` query at a time is enough.
  let querying = false;
  const onVisibility = () => {
    if (document.visibilityState === "hidden") cb();
  };
  document.addEventListener("visibilitychange", onVisibility);
  try {
    const win = getCurrentWindow();
    win
      .onResized(async () => {
        if (querying) return;
        querying = true;
        try {
          if (active && (await win.isMinimized())) cb();
        } catch {
          // A refused or failed query: the document event still covers the common case.
        } finally {
          querying = false;
        }
      })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch(() => undefined);
  } catch {
    // No Tauri window here (tests, a plain browser).
  }
  return () => {
    active = false;
    document.removeEventListener("visibilitychange", onVisibility);
    unlisten?.();
    unlisten = null;
  };
}

/**
 * Revealed values that exclude each other: revealing one member hides every other, so only one
 * secret of a group is ever visible (notes and fields of one entry share a group). The window
 * hiding or minimising hides all of them. `destroy()` (call it when the owner unmounts) hides
 * everything and drops the window subscription.
 */
export function createRevealGroup(): {
  create(): Revealed;
  hideAll(): void;
  destroy(): void;
} {
  const members = new Set<Revealed>();
  let offWindow: (() => void) | null = null;

  function hideAll() {
    for (const member of members) member.hide();
  }

  return {
    create() {
      offWindow ??= onWindowHidden(hideAll);
      const inner = createRevealed();
      const member: Revealed = {
        get value() {
          return inner.value;
        },
        async reveal(load) {
          for (const other of members) if (other !== member) other.hide();
          await inner.reveal(load);
        },
        hide: inner.hide,
      };
      members.add(member);
      return member;
    },
    hideAll,
    destroy() {
      hideAll();
      members.clear();
      offWindow?.();
      offWindow = null;
    },
  };
}

export type RevealGroup = ReturnType<typeof createRevealGroup>;
