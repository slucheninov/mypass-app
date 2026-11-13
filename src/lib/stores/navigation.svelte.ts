import type { EntryEditModel_Serialize, Scope } from "../bindings";
import { api } from "../api";
import { session } from "./session.svelte";

export type Presentation = "browse" | "edit" | "create";

const ALL: Scope = { kind: "category", category: "all" };

class NavigationStore {
  scope = $state<Scope>({ ...ALL });
  selectedId = $state<string | null>(null);
  presentation = $state<Presentation>("browse");
  /** The store-held secret: the editor's model carries the entry's password. Nulled on close,
   *  save and lock. */
  editModel = $state<EntryEditModel_Serialize | null>(null);
  /** A model waiting to replace a dirty editor; the editor asks "Discard changes?" before it opens.
   *  It can carry a password too (a full edit model routed through `requestOpenEditor`), so it is the
   *  second store-held secret: it is nulled by `openEditor`, `closeEditor` (hence `reset` on lock)
   *  and the editor's Keep editing, and `no_secret_in_any_store_state` pins that. */
  pendingModel = $state<EntryEditModel_Serialize | null>(null);
  /** True while a user action (toolbar Lock, Open Existing) waits for the editor's answer to
   *  "Discard changes?" (see `confirmLeave`). */
  leavePending = $state(false);
  /** Registered by the mounted `EntryEditor`: whether it holds unsaved changes. */
  editorDirty: (() => boolean) | null = null;

  #leave: ((leave: boolean) => void) | null = null;
  #reportedDirty = false;
  #scopeListeners = new Set<(scope: Scope) => void>();
  #offLock: (() => void) | null = null;

  /** Forgets the selection, the editor and the scope when the vault locks. Call once per window. */
  start(): void {
    this.#offLock ??= session.onLocked(() => this.reset());
  }

  stop(): void {
    this.#offLock?.();
    this.#offLock = null;
    this.#scopeListeners.clear();
    this.reset();
  }

  /** Notifies `cb` after every `setScope`. Returns the unsubscriber. */
  onScopeChange(cb: (scope: Scope) => void): () => void {
    this.#scopeListeners.add(cb);
    return () => this.#scopeListeners.delete(cb);
  }

  /** Changes the scope. An open editor is left alone (a draft must survive navigation); browsing
   *  drops the selection, which belongs to the previous list. */
  setScope(scope: Scope): void {
    this.scope = scope;
    if (this.presentation === "browse") this.selectedId = null;
    for (const cb of [...this.#scopeListeners]) cb(scope);
  }

  select(id: string | null): void {
    this.selectedId = id;
  }

  openEditor(model: EntryEditModel_Serialize): void {
    this.pendingModel = null;
    this.editModel = model;
    this.presentation = model.id ? "edit" : "create";
    // An editor that replaced another one (an open-entry request after Discard) leaves the list
    // pointing at the entry it now edits.
    if (model.id) this.selectedId = model.id;
  }

  /**
   * Opens `model` in the editor. When an editor is already open with unsaved changes the model
   * waits in `pendingModel` and the editor asks first; otherwise it opens at once. Every caller
   * that can replace an open editor (Toolbar "New ...", the detail Edit button) uses this.
   */
  requestOpenEditor(model: EntryEditModel_Serialize): void {
    if (this.presentation !== "browse" && this.editModel && this.editorDirty?.()) {
      this.pendingModel = model;
      return;
    }
    this.pendingModel = null;
    this.openEditor(model);
  }

  /**
   * Asks whether the user may leave the editor behind: resolves `true` at once when no editor holds
   * unsaved changes, otherwise the editor shows its "Discard changes?" dialog and the promise
   * resolves with the answer (`true` after Discard, which also closes the editor). For user-initiated
   * actions only (toolbar Lock, Open Existing): idle, sleep, screen-lock, hotkey and tray locks never
   * ask, security beats unsaved edits. A lock that lands while the dialog is up resolves `false`.
   */
  confirmLeave(): Promise<boolean> {
    if (this.presentation === "browse" || !this.editModel || !this.editorDirty?.()) return Promise.resolve(true);
    this.#settleLeave(false);
    this.leavePending = true;
    return new Promise((resolve) => {
      this.#leave = resolve;
    });
  }

  /** The editor's answer to a pending `confirmLeave`. */
  answerLeave(leave: boolean): void {
    const settle = this.#leave;
    this.#leave = null;
    this.leavePending = false;
    if (leave && settle) this.closeEditor();
    settle?.(leave);
  }

  #settleLeave(leave: boolean): void {
    const settle = this.#leave;
    this.#leave = null;
    this.leavePending = false;
    settle?.(leave);
  }

  /**
   * Tells the backend whether the editor holds unsaved changes, so the quit flow can ask before it
   * discards them. The one place that calls `appSetEditorDirty`: only the main window mounts an
   * editor, and the call goes out on a change only. Advisory: a failed call is ignored.
   */
  reportEditorDirty(dirty: boolean): void {
    if (dirty === this.#reportedDirty) return;
    this.#reportedDirty = dirty;
    void api.appSetEditorDirty(dirty).catch(() => {});
  }

  closeEditor(): void {
    this.reportEditorDirty(false);
    this.#settleLeave(false);
    this.pendingModel = null;
    this.editModel = null;
    this.presentation = "browse";
  }

  /** Back to the start state (All items, nothing selected, no editor). Listeners stay. */
  reset(): void {
    this.scope = { ...ALL };
    this.selectedId = null;
    this.closeEditor();
  }
}

export const navigation = new NavigationStore();
