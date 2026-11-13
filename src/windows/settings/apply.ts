import { errorDetail, errorMessage } from "../../lib/api";
import type { SettingsPatch_Deserialize } from "../../lib/bindings";
import { settings } from "../../lib/stores/settings.svelte";

/** Sends one patch through the settings store; resolves to the localised error text, or `null` on success. */
export async function applyPatch(patch: SettingsPatch_Deserialize): Promise<string | null> {
  try {
    await settings.update(patch);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/** The system's own words for an `io` / `internal` failure (used verbatim by the login-item message). */
export function systemText(e: unknown): string {
  return errorDetail(e) ?? errorMessage(e);
}
