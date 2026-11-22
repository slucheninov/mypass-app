import { device } from "./device.svelte";
import { update } from "./update.svelte";
import { sync } from "./sync.svelte";
import { toasts } from "./toasts.svelte";
import { startActivityReporter } from "./activity.svelte";
import { entries } from "./entries.svelte";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { settings } from "./settings.svelte";

/**
 * Starts the stores of a window and the activity reporter; returns the function that stops them.
 * The lock hooks (navigation, entries) are registered before the first `vault_status` answer can
 * trigger them. Entries, settings and session start side by side and independently (the entries
 * hooks are registered synchronously, only the `VaultChanged` subscription is awaited), so theme and
 * language are applied even if one part cannot start; a failing part is reported, the rest keeps running.
 * `entries: false` is for a window that shows no entries (settings): it keeps the lock hooks and the
 * activity reporter but never fetches the list or the sidebar. `saveToast: false` (tray panel, settings)
 * leaves the sticky "Saving failed" toast to the main window. `sync: false` keeps the tray
 * panel from querying sync/storage state that it does not render or have permission to read.
 * `whileLocked: false` (settings) reads no device, storage or update state unless the vault is
 * unlocked, and the settings store never reads more than the public settings then: the settings
 * window shows only its locked placeholder.
 */
export async function startStores(
  opts: { entries?: boolean; saveToast?: boolean; sync?: boolean; theme?: boolean; whileLocked?: boolean } = {},
): Promise<() => void> {
  const whileLocked = opts.whileLocked ?? true;
  const withEntries = opts.entries ?? true;
  session.saveToasts = opts.saveToast ?? true;
  navigation.start();
  const stopActivity = startActivityReporter();
  const stopDevice = device.start({ whileLocked });
  const stopUpdate = update.start({ whileLocked });
  const stopSync = opts.sync === false ? () => {} : sync.start({ whileLocked });
  const results = await Promise.allSettled([
    settings.start({ followTheme: opts.theme }),
    withEntries ? entries.start() : Promise.resolve(),
    session.start(),
  ]);
  for (const r of results) if (r.status === "rejected") toasts.reportError(r.reason);
  return () => {
    stopActivity();
    stopDevice();
    stopUpdate();
    stopSync();
    entries.stop();
    navigation.stop();
    settings.stop();
    session.stop();
  };
}
