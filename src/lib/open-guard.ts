// The open guard (D6/D7): before a vault path is used for anything (password prompt, create, open)
// the backend is asked whether opening it is risky, and each warning is put to the user.
//
// The dialog itself is drawn by `OpenGuardHost.svelte` (mounted once in the main window), which
// registers itself here as the presenter, so this module stays plain TypeScript and a flow can
// `await confirmOpen(path)` from anywhere.
import { api } from "./api";
import type { OpenWarning } from "./bindings";

/** Shows one warning and resolves `true` for Continue, `false` for Cancel or Escape. */
export type WarningPresenter = (warning: OpenWarning) => Promise<boolean>;

let presenter: WarningPresenter | null = null;

/** Called by `OpenGuardHost` on mount (and with `null` on destroy). */
export function setWarningPresenter(next: WarningPresenter | null): void {
  presenter = next;
}

/**
 * Resolves `true` when the flow may go on. A failing `vault_open_warnings` call counts as "no
 * warnings" (the guard must never block opening); a warning that cannot be shown (no host mounted)
 * counts as declined, so the guard never lets a risky open through unseen.
 */
export async function confirmOpen(path: string): Promise<boolean> {
  let warnings: OpenWarning[];
  try {
    warnings = await api.vaultOpenWarnings(path);
  } catch {
    return true;
  }
  for (const warning of warnings) {
    if (!presenter) return false;
    if (!(await presenter(warning))) return false;
  }
  return true;
}
