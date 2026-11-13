import type { EntryEditModel_Serialize, Scope } from "../bindings";
import { session } from "./session.svelte";

export type Presentation = "browse" | "edit" | "create";

const ALL: Scope = { kind: "category", category: "all" };

class NavigationStore {
  scope = $state<Scope>({ ...ALL });
  selectedId = $state<string | null>(null);
  presentation = $state<Presentation>("browse");
  /** The one store-held secret: the editor's model carries the entry's password. Nulled on close,
   *  save and lock. */
  editModel = $state<EntryEditModel_Serialize | null>(null);

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
    this.editModel = model;
    this.presentation = model.id ? "edit" : "create";
  }

  closeEditor(): void {
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
