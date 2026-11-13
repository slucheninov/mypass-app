// The ways a vault path reaches the app. Vault open and create are the only places the UI supplies
// a path to the backend (AGENTS.md "File dialogs"): the native dialogs below come from the dialog
// plugin, and every chosen path goes through the open guard before anything else happens.
import { open, save } from "@tauri-apps/plugin-dialog";
import { confirmOpen } from "./open-guard";
import { session } from "./stores/session.svelte";
import { toasts } from "./stores/toasts.svelte";

const FILTERS = [{ name: "KeePass", extensions: ["kdbx"] }];

/** The file dialog for an existing vault; `null` when cancelled. */
export async function pickVaultFile(): Promise<string | null> {
  const picked = await open({ multiple: false, directory: false, filters: FILTERS });
  return typeof picked === "string" ? picked : null;
}

/** The save dialog for a new vault (default file name `MyPass.kdbx`); `null` when cancelled. */
export async function pickSavePath(): Promise<string | null> {
  return await save({ defaultPath: "MyPass.kdbx", filters: FILTERS });
}

/** Guard, then the password step for `path` (locking an unlocked session first). `true` when started. */
export async function openVaultAtPath(path: string): Promise<boolean> {
  try {
    if (!(await confirmOpen(path))) return false;
    await session.openAnother(path);
    return true;
  } catch (e) {
    toasts.reportError(e);
    return false;
  }
}

/** "Open existing vault": file dialog, guard, password step. Used by the unlock screen and the toolbar. */
export async function openVaultFromDialog(): Promise<void> {
  try {
    const path = await pickVaultFile();
    if (path !== null) await openVaultAtPath(path);
  } catch (e) {
    toasts.reportError(e);
  }
}
