import { api } from "./api";
import type { EntryTypeDto } from "./bindings";
import { lockGuard } from "./lock-guard";
import { navigation } from "./stores/navigation.svelte";
import { toasts } from "./stores/toasts.svelte";

/** The group a new item or group lands in: the selected group, else the backend's default. */
export function currentGroup(): string | null {
  return navigation.scope.kind === "group" ? navigation.scope.groupId : null;
}

/**
 * The toolbar "+" menu's New <type> action, shared with the extension's request (`NewEntryEvent`):
 * a fresh model of `entryType` opens in the editor. Behind an editor with unsaved changes it waits
 * for the user's answer to "Discard changes?" (`requestOpenEditor`), so no edit is dropped silently.
 */
export async function startNewEntry(entryType: EntryTypeDto): Promise<void> {
  const unchanged = lockGuard();
  await toasts.guard(async () => {
    const model = await api.entryNewModel(entryType, currentGroup());
    // A lock that landed meanwhile must not leave an editor behind for the next unlock.
    if (unchanged()) navigation.requestOpenEditor(model);
  });
}
