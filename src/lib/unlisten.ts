/**
 * Calls an event unlistener and swallows what it throws or rejects with. A window that is closing
 * (or a test tearing its IPC down first) can no longer answer the plugin call behind it, and a
 * listener that is gone anyway must not surface as an unhandled rejection.
 */
export function quietly(off: () => unknown): void {
  try {
    void Promise.resolve(off()).catch(() => {});
  } catch {
    // Already gone.
  }
}
