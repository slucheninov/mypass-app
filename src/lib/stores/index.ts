import { startActivityReporter } from "./activity.svelte";
import { entries } from "./entries.svelte";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { settings } from "./settings.svelte";

/**
 * Starts every store of a window and the activity reporter; returns the function that stops them.
 * Order matters: the hooks that clear on lock are registered before the first `vault_status`
 * answer can trigger them.
 */
export async function startStores(): Promise<() => void> {
  navigation.start();
  await entries.start();
  const stopActivity = startActivityReporter();
  await session.start();
  await settings.start();
  return () => {
    stopActivity();
    entries.stop();
    navigation.stop();
    settings.stop();
    session.stop();
  };
}
