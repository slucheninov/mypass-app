// Local UI preferences (pane widths and the like). The only module allowed to touch web storage
// (pinned by the `no_secret_sinks` guard); callers store layout, never vault data. Every access is
// wrapped: storage can be missing, blocked or full (private windows, previews), and the UI must
// render the same without it.

/** The stored string for `key`, or `null` when absent or storage is unavailable. */
export function readPref(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** Stores `value` under `key`; a failure is ignored (the preference is a convenience). */
export function writePref(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Ignored on purpose.
  }
}
