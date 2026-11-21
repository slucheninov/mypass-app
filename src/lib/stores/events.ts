import { api } from "../api";
import { events, type OpenEntryEvent, type ShortcutConflictEvent } from "../bindings";
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

/** The tray panel's "+" (a login) or the browser extension's "+" menu (an address or files item)
 *  asked for a new entry. The request itself waits in the backend (newest wins, 120 s, taken at
 *  most once); `NewEntryEvent` is only a payload-free nudge, which any webview could forge, so it
 *  never decides anything here: the main window takes the request with `app_take_new_entry` (a
 *  main-window-only command that answers `null` while locked and keeps the request) and runs the
 *  toolbar's own New action, so a dirty editor asks "Discard changes?" first. It takes:
 *  - once its listener is registered (a request made before the webview listened, at startup or
 *    after a reload, is not lost),
 *  - on every nudge,
 *  - after every unlock (a request made while locked). */
function newEntryBridge(): { take: () => Promise<void>; stop: () => void } {
  let active = true;
  const take = async () => {
    let entryType;
    try {
      entryType = await api.appTakeNewEntry();
    } catch {
      // Nothing to start; the user can still use the toolbar.
      return;
    }
    if (active && entryType !== null) await startNewEntry(entryType);
  };
  const off = session.onChange((next, prev) => {
    if (next === "unlocked" && prev !== "unlocked") void take();
  });
  return {
    take,
    stop: () => {
      active = false;
      off();
    },
  };
}

function toastConflict({ shortcut }: ShortcutConflictEvent): void {
  toasts.push("error", `${shortcut}: ${t("That shortcut is already in use")}`);
}

/**
 * The main window's backend events as UI: the clipboard clear, the tray panel's open-entry request,
 * the new-entry request of the tray panel and the browser extension (taken from the backend, see
 * `newEntryBridge`) and the hotkeys that could not be registered. The conflict listener is registered BEFORE the
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
  // Taken right after the listener is registered, so a request made before it is not lost and a
  // nudge from now on is heard.
  void events.newEntryEvent
    .listen(() => void newEntry.take())
    .then((off) => {
      if (!active) return quietly(off);
      offs.push(off);
      void newEntry.take();
    })
    .catch(() => {
      // Without the listener the request is still taken after the next unlock.
    });
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
