import { toasts } from "./toasts.svelte";
import { startActivityReporter } from "./activity.svelte";
import { entries } from "./entries.svelte";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { settings } from "./settings.svelte";

/**
 * Starts the stores of a window and the activity reporter; returns the function that stops them.
 * The lock hooks (navigation, entries) are registered before the first `vault_status` answer can
 * trigger them. Settings start first and independently, so theme and language are applied even if
 * the session cannot be read; a failing part is reported and the rest keeps running.
 * `entries: false` is for a window that shows no entries (settings): it keeps the lock hooks and the
 * activity reporter but never fetches the list or the sidebar.
 */
export async function startStores(opts: { entries?: boolean } = {}): Promise<() => void> {
  const withEntries = opts.entries ?? true;
  navigation.start();
  if (withEntries) await entries.start();
  const stopActivity = startActivityReporter();
  const results = await Promise.allSettled([settings.start(), session.start()]);
  for (const r of results) if (r.status === "rejected") toasts.reportError(r.reason);
  return () => {
    stopActivity();
    entries.stop();
    navigation.stop();
    settings.stop();
    session.stop();
  };
}
