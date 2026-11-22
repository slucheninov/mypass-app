import { api } from "./api";
import { platform } from "./platform.svelte";

/** Loads the home directory for vault path labels in the main window and tray panel. */
export async function loadHomeDirectory(): Promise<void> {
  try {
    platform.homeDir = await api.appHomeDir();
  } catch {
    // Path labels can render without home directory shortening.
  }
}
