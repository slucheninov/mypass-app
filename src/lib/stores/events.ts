import { api } from "../api";
import { events, type EntryTypeDto, type NewEntryEvent, type OpenEntryEvent, type ShortcutConflictEvent } from "../bindings";
import { t } from "../i18n/index.svelte";
import { lockGuard } from "../lock-guard";
import { startNewEntry } from "../new-entry";
import { quietly } from "../unlisten";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { toasts } from "./toasts.svelte";

/** Opens the entry the tray panel asked for. A locked session ignores it (the main window shows the
 *  unlock screen); an editor with unsaved changes asks "Discard changes?" through `requestOpenEditor`
 *  before it is replaced, so an event never drops edits silently. */
async function openEntry({ id, edit }: OpenEntryEvent): Promise<void> {
  if (session.status.state !== "unlocked") return;
  if (!edit) {
    // Selecting another entry over an open editor (maybe dirty) would land the editor's close or
    // save on that entry; the Task 9 ruling is to ask first, and nothing here is worth a prompt.
    if (navigation.presentation !== "browse") return;
    navigation.select(id);
    return;
  }
  const unchanged = lockGuard();
  let model;
  try {
    model = await api.entryEditModel(id);
  } catch (e) {
    toasts.reportError(e);
    return;
  }
  if (!unchanged()) return;
  navigation.requestOpenEditor(model);
  // Behind a dirty editor the model waits for the user's answer, and the selection stays where it is.
  if (navigation.editModel?.id === id) navigation.select(id);
}

/** How long a new-entry request that arrived while locked waits for the unlock. Past it the user has
 *  moved on, and an editor appearing out of nowhere after a later unlock would only surprise. */
export const PENDING_NEW_ENTRY_MS = 120_000;

/** The tray panel's "+" (a login) or the browser extension's "+" menu (an address or files item)
 *  asked for a new entry (`NewEntryEvent`); the backend already brought the window forward. Unlocked, it runs the toolbar's own New action now
 *  (so a dirty editor asks "Discard changes?" first). Locked or without a vault, the request waits
 *  for the next unlock, within `PENDING_NEW_ENTRY_MS`; a newer request replaces it. The pending
 *  value is only an entry type, no secret, and lives in this listener's closure. */
function newEntryBridge(): { request: (e: NewEntryEvent) => void; stop: () => void } {
  let pending: { type: EntryTypeDto; at: number } | null = null;
  const off = session.onChange((next, prev) => {
    if (next !== "unlocked" || prev === "unlocked" || !pending) return;
    const { type, at } = pending;
    pending = null;
    if (Date.now() - at <= PENDING_NEW_ENTRY_MS) void startNewEntry(type);
  });
  return {
    request: ({ entryType }) => {
      if (session.status.state === "unlocked") {
        pending = null;
        void startNewEntry(entryType);
      } else {
        pending = { type: entryType, at: Date.now() };
      }
    },
    stop: () => {
      pending = null;
      off();
    },
  };
}

function toastConflict({ shortcut }: ShortcutConflictEvent): void {
  toasts.push("error", `${shortcut}: ${t("That shortcut is already in use")}`);
}

/**
 * The main window's backend events as UI: the clipboard clear, the tray panel's open-entry request,
 * the new-entry request of the tray panel and the browser extension and the hotkeys that could not be registered. The conflict listener is registered BEFORE the
 * backlog is taken (`shortcut_conflicts_take` empties it), so none is lost or shown twice (R12).
 * The failed-save toast lives in the session store. Returns the function that stops the listeners.
 */
export function startEventBridge(): () => void {
  let active = true;
  const offs: (() => void)[] = [];
  const keep = (pending: Promise<() => void>) => {
    // A listener that cannot be registered only loses its toast; the stores report their own.
    void pending
      .then((off) => {
        if (active) offs.push(off);
        else quietly(off);
      })
      .catch(() => {});
  };
  keep(events.clipboardClearedEvent.listen(() => toasts.push("info", t("Clipboard cleared"))));
  keep(events.openEntryEvent.listen((e) => void openEntry(e.payload)));
  const newEntry = newEntryBridge();
  offs.push(newEntry.stop);
  keep(events.newEntryEvent.listen((e) => newEntry.request(e.payload)));
  void events.shortcutConflictEvent
    .listen((e) => toastConflict(e.payload))
    .then(async (off) => {
      if (!active) return quietly(off);
      offs.push(off);
      try {
        for (const conflict of await api.shortcutConflictsTake()) toastConflict(conflict);
      } catch {
        // Nothing to show: the settings pane still reports a conflict of a later change.
      }
    })
    .catch(() => {});
  return () => {
    active = false;
    for (const off of offs.splice(0)) quietly(off);
  };
}
