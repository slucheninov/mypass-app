// The notes editor's own undo history. Programmatic toolbar edits (Bold, Link, ...) replace the
// textarea value, which breaks the browser's native history, so the editor keeps this stack and
// handles Mod+Z itself. It holds note text: the editor clears it on close, save, discard and lock,
// and it lives in the component, never in a store.

export interface Selection {
  start: number;
  end: number;
}

export interface UndoSnapshot {
  text: string;
  selection: Selection;
}

export interface UndoStack {
  /**
   * Records the editor state. Typing (the default) is coalesced with the previous typing when it
   * came within `coalesceMs`; `{ step: true }` (toolbar edits, restores, the base state) is always
   * its own step. A push with the text of the current state only refreshes that state's selection.
   */
  push(text: string, selection: Selection, opts?: { step?: boolean }): void;
  undo(): UndoSnapshot | null;
  redo(): UndoSnapshot | null;
  canUndo(): boolean;
  canRedo(): boolean;
  clear(): void;
}

export interface UndoOptions {
  /** Snapshots kept, the current state included. */
  max?: number;
  coalesceMs?: number;
  now?: () => number;
}

export function createUndoStack(opts: UndoOptions = {}): UndoStack {
  const max = opts.max ?? 200;
  const coalesceMs = opts.coalesceMs ?? 500;
  const now = opts.now ?? (() => Date.now());
  let states: UndoSnapshot[] = [];
  let index = -1;
  let lastPushAt = 0;
  let lastWasTyping = false;

  const copy = (s: UndoSnapshot): UndoSnapshot => ({ text: s.text, selection: { ...s.selection } });
  const at = (i: number): UndoSnapshot | undefined => states[i];

  return {
    push(text, selection, o = {}) {
      const step = o.step ?? false;
      const current = at(index);
      if (current && current.text === text) {
        current.selection = { ...selection };
        if (step) lastWasTyping = false;
        return;
      }
      states = states.slice(0, index + 1);
      const time = now();
      const coalesce = !step && lastWasTyping && index > 0 && time - lastPushAt <= coalesceMs;
      const snapshot = { text, selection: { ...selection } };
      if (coalesce) {
        states[index] = snapshot;
      } else {
        states.push(snapshot);
        index = states.length - 1;
        if (states.length > max) {
          states.shift();
          index--;
        }
      }
      lastPushAt = time;
      lastWasTyping = !step && states.length > 1;
    },
    undo() {
      lastWasTyping = false;
      const previous = at(index - 1);
      if (!previous) return null;
      index--;
      return copy(previous);
    },
    redo() {
      lastWasTyping = false;
      const next = at(index + 1);
      if (!next) return null;
      index++;
      return copy(next);
    },
    canUndo: () => index > 0,
    canRedo: () => index >= 0 && index < states.length - 1,
    clear() {
      states = [];
      index = -1;
      lastPushAt = 0;
      lastWasTyping = false;
    },
  };
}
