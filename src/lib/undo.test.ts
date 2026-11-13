import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createUndoStack } from "./undo";

const at = (n: number) => ({ start: n, end: n });

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("createUndoStack", () => {
  it("steps back and forward through pushed states", () => {
    const s = createUndoStack();
    s.push("", at(0), { step: true });
    s.push("a", at(1));
    vi.advanceTimersByTime(600);
    s.push("ab", at(2));
    expect(s.undo()).toEqual({ text: "a", selection: at(1) });
    expect(s.undo()).toEqual({ text: "", selection: at(0) });
    expect(s.undo()).toBeNull();
    expect(s.redo()).toEqual({ text: "a", selection: at(1) });
    expect(s.redo()).toEqual({ text: "ab", selection: at(2) });
    expect(s.redo()).toBeNull();
  });

  it("coalesces typing within 500 ms into one step", () => {
    const s = createUndoStack();
    s.push("", at(0), { step: true });
    s.push("a", at(1));
    vi.advanceTimersByTime(200);
    s.push("ab", at(2));
    vi.advanceTimersByTime(200);
    s.push("abc", at(3));
    expect(s.undo()).toEqual({ text: "", selection: at(0) });
    expect(s.undo()).toBeNull();
  });

  it("starts a new step after a 500 ms pause", () => {
    const s = createUndoStack();
    s.push("", at(0), { step: true });
    s.push("a", at(1));
    vi.advanceTimersByTime(501);
    s.push("ab", at(2));
    expect(s.undo()?.text).toBe("a");
  });

  it("never coalesces a toolbar step, before or after it", () => {
    const s = createUndoStack();
    s.push("", at(0), { step: true });
    s.push("a", at(1));
    s.push("**a**", { start: 2, end: 3 }, { step: true });
    s.push("**a**b", at(6));
    expect(s.undo()?.text).toBe("**a**");
    expect(s.undo()?.text).toBe("a");
    expect(s.undo()?.text).toBe("");
  });

  it("updates the selection of the current state when the text is unchanged", () => {
    const s = createUndoStack();
    s.push("abc", at(0), { step: true });
    s.push("abc", { start: 0, end: 3 }, { step: true });
    s.push("**abc**", { start: 2, end: 5 }, { step: true });
    expect(s.undo()).toEqual({ text: "abc", selection: { start: 0, end: 3 } });
  });

  it("drops the redo tail when a new state is pushed after an undo", () => {
    const s = createUndoStack();
    s.push("", at(0), { step: true });
    s.push("a", at(1), { step: true });
    s.undo();
    s.push("b", at(1), { step: true });
    expect(s.redo()).toBeNull();
    expect(s.canRedo()).toBe(false);
  });

  it("keeps at most 200 snapshots", () => {
    const s = createUndoStack();
    for (let i = 0; i < 300; i++) s.push(`t${i}`, at(0), { step: true });
    let undone = 0;
    while (s.undo()) undone++;
    expect(undone).toBe(199);
  });

  it("clear forgets every snapshot", () => {
    const s = createUndoStack();
    s.push("", at(0), { step: true });
    s.push("secret note", at(11), { step: true });
    s.clear();
    expect(s.canUndo()).toBe(false);
    expect(s.canRedo()).toBe(false);
    expect(s.undo()).toBeNull();
    expect(s.redo()).toBeNull();
  });
});
