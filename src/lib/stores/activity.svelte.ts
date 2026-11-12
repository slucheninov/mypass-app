import { api } from "../api";
import { session } from "./session.svelte";

/** At most one report per this many milliseconds. */
export const ACTIVITY_THROTTLE_MS = 1000;

const EVENTS = ["pointerdown", "keydown", "wheel", "focus"] as const;

/**
 * Tells the backend the user is active (it resets the idle timer). Silent while the session is not
 * unlocked; failures are ignored (the idle timer is a convenience, never worth a message).
 * Returns the cleanup that removes the listeners.
 */
export function startActivityReporter(win: Window = window): () => void {
  let lastSentAt: number | null = null;
  const onActivity = () => {
    if (session.status.state !== "unlocked") return;
    const now = Date.now();
    if (lastSentAt !== null && now - lastSentAt < ACTIVITY_THROTTLE_MS) return;
    lastSentAt = now;
    api.appRecordActivity().catch(() => {});
  };
  // Capture, because `focus` does not bubble and other handlers may stop propagation.
  for (const name of EVENTS) win.addEventListener(name, onActivity, { capture: true, passive: true });
  return () => {
    for (const name of EVENTS) win.removeEventListener(name, onActivity, { capture: true });
  };
}
