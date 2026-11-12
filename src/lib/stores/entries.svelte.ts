import { ApiFailure, api } from "../api";
import { events, type EntryQuery, type EntrySummary, type SortKey, type Sidebar_Serialize } from "../bindings";
import { navigation } from "./navigation.svelte";
import { session } from "./session.svelte";
import { toasts } from "./toasts.svelte";

/** Quiet time after the last keystroke before the list is asked again. */
export const SEARCH_DEBOUNCE_MS = 150;

class EntriesStore {
  items = $state<EntrySummary[]>([]);
  sidebar = $state<Sidebar_Serialize | null>(null);
  loading = $state(false);
  /** What the search box shows right now; `query.search` is the debounced value in use. */
  searchText = $state("");
  #search = $state("");
  #sort = $state<SortKey>("title");

  #generation = 0;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #unlisten: (() => void)[] = [];
  #offHooks: (() => void)[] = [];

  /** The query in force: the navigation scope plus the committed search and sort. */
  get query(): EntryQuery {
    return { scope: $state.snapshot(navigation.scope), search: this.#search, sort: this.#sort };
  }

  /** Wires the store to the session and to `VaultChanged`. Call once per window, before `session.start`. */
  async start(): Promise<void> {
    if (this.#offHooks.length > 0) return;
    this.#offHooks.push(
      session.onChange((next, prev) => {
        if (next === "unlocked") void this.refresh();
        else if (prev === "unlocked") this.clear();
      }),
      navigation.onScopeChange(() => void this.refreshList()),
    );
    this.#unlisten.push(
      await events.vaultChanged.listen(() => {
        if (session.status.state === "unlocked") void this.refresh();
      }),
    );
  }

  stop(): void {
    for (const off of this.#unlisten.splice(0)) off();
    for (const off of this.#offHooks.splice(0)) off();
    this.clear();
    this.#sort = "title";
  }

  /** Debounced: the list is fetched once, 150 ms after the last call. */
  setSearch(text: string): void {
    this.searchText = text;
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#search = text;
      void this.refreshList();
    }, SEARCH_DEBOUNCE_MS);
  }

  setSort(sort: SortKey): Promise<void> {
    this.#sort = sort;
    return this.refreshList();
  }

  /** The list and the sidebar counts. */
  async refresh(): Promise<void> {
    await Promise.all([this.refreshList(), this.#refreshSidebar()]);
  }

  /** Only the list; a response that is no longer the newest request is dropped. */
  async refreshList(): Promise<void> {
    const generation = ++this.#generation;
    this.loading = true;
    try {
      const rows = await api.entryList(this.query);
      if (generation === this.#generation) this.items = rows;
    } catch (e) {
      this.#fail(e);
    } finally {
      if (generation === this.#generation) this.loading = false;
    }
  }

  async #refreshSidebar(): Promise<void> {
    try {
      const sidebar = await api.sidebarGet();
      if (session.status.state === "unlocked") this.sidebar = sidebar;
    } catch (e) {
      this.#fail(e);
    }
  }

  /** A `locked` answer is the lock racing the request: nothing to show. */
  #fail(e: unknown): void {
    if (e instanceof ApiFailure && (e.code === "locked" || e.code === "noVault")) return;
    toasts.reportError(e);
  }

  /** Forgets everything of the vault: list, sidebar, search, selection and any edit model. */
  clear(): void {
    this.#generation++;
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    this.items = [];
    this.sidebar = null;
    this.loading = false;
    this.searchText = "";
    this.#search = "";
    navigation.select(null);
    navigation.closeEditor();
  }
}

export const entries = new EntriesStore();
