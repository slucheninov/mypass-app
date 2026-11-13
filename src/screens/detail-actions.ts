import { api } from "../lib/api";
import type { FieldRef } from "../lib/bindings";
import { t } from "../lib/i18n/index.svelte";
import { settings } from "../lib/stores/settings.svelte";
import { toasts } from "../lib/stores/toasts.svelte";

/**
 * Puts one field on the clipboard through the backend (the value never reaches the webview) and
 * says when the clipboard clears. Throws what the command throws; callers wrap it in `toasts.guard`.
 */
export async function copyField(entryId: string, field: FieldRef): Promise<void> {
  await api.entryCopy(entryId, field);
  const seconds = settings.value?.clipboardClearSeconds;
  toasts.push("success", seconds ? t("Copied. The clipboard clears in %lld s.", seconds) : t("Copied."));
}
