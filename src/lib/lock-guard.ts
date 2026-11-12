import { session } from "./stores/session.svelte";

/**
 * Captures the current lock epoch; the returned function says whether the session is still the
 * same unlocked stretch. Call it after an `await` before writing to a store, so an answer that
 * arrives after a lock never leaves state behind for the next unlock.
 */
export function lockGuard(): () => boolean {
  const epoch = session.lockEpoch;
  return () => session.lockEpoch === epoch;
}
