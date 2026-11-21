<script lang="ts">
  // The tray panel window (340 x 520). It shares the stores and `api` with the main window.
  // Locked, it shows the main window's unlock card; unlocked, a search field over the five most
  // recent items (summaries only, no secret) or, while searching, the matching items, and a
  // compact detail. Under both sits a native-style menu (Open MyPass, Lock, Settings, Quit).
  // Showing and hiding the window (blur included) is the backend's; this component only reacts to
  // the session and to a hide.
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { events, type EntryQuery, type EntrySummary } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { platform } from "../lib/platform.svelte";
  import { onWindowHidden } from "../lib/secret.svelte";
  import { formatShortcut } from "../lib/shortcuts";
  import { session } from "../lib/stores/session.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import { quietly } from "../lib/unlisten";
  import UnlockCard from "../screens/UnlockCard.svelte";
  import TrayDetail from "./TrayDetail.svelte";
  import TrayEntryRow from "./TrayEntryRow.svelte";
  import TrayMenuRow from "./TrayMenuRow.svelte";
  import TrayUpdateRow from "./TrayUpdateRow.svelte";

  const RECENT_LIMIT = 5;
  /** A search lists at most this many matches; the main window has the full list. */
  const RESULT_LIMIT = 50;
  const SEARCH_DELAY_MS = 120;

  let recents = $state<EntrySummary[]>([]);
  let results = $state<EntrySummary[]>([]);
  let query = $state("");
  let selectedId = $state<string | null>(null);
  let card: UnlockCard | undefined = $state();
  let searchInput: HTMLInputElement | undefined = $state();
  let requests = 0;
  let searches = 0;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  const unlocked = $derived(session.status.state === "unlocked");
  const noVault = $derived(session.status.state === "noVault");
  // One key per unlocked stretch and vault: a save-state change replaces `status` but not this.
  const vaultKey = $derived(unlocked ? (session.status.vaultPath ?? "") : null);
  const searching = $derived(query.trim() !== "");
  const shown = $derived(searching ? results : recents);
  const lockShortcut = $derived(formatShortcut(settings.value?.shortcutLock ?? "", platform.os));
  const showShortcut = $derived(formatShortcut(settings.value?.shortcutShow ?? "", platform.os));

  async function loadRecents(): Promise<void> {
    const mine = ++requests;
    const unchanged = lockGuard();
    try {
      const list = await api.entryRecent(RECENT_LIMIT);
      if (mine === requests && unchanged()) recents = list;
    } catch (e) {
      if (mine === requests && unchanged()) toasts.reportError(e);
    }
  }

  function stopSearchTimer(): void {
    if (searchTimer !== null) clearTimeout(searchTimer);
    searchTimer = null;
  }

  /** Runs the search for the current text: title, user name, websites and the other visible
   *  fields, through the same backend query as the main window's search field. */
  async function runSearch(): Promise<void> {
    stopSearchTimer();
    const text = query.trim();
    const mine = ++searches;
    if (text === "" || !unlocked) {
      results = [];
      return;
    }
    const unchanged = lockGuard();
    const request: EntryQuery = {
      scope: { kind: "category", category: "all" },
      search: text,
      sort: "title",
      order: "ascending",
    };
    try {
      const list = await api.entryList(request);
      if (mine === searches && unchanged()) results = list.slice(0, RESULT_LIMIT);
    } catch (e) {
      if (mine === searches && unchanged()) toasts.reportError(e);
    }
  }

  function scheduleSearch(): void {
    stopSearchTimer();
    searchTimer = setTimeout(() => void runSearch(), SEARCH_DELAY_MS);
  }

  function resetSearch(): void {
    stopSearchTimer();
    searches++;
    query = "";
    results = [];
  }

  // Everything of one unlocked stretch is dropped on a lock; the next unlock starts at the list.
  $effect(() => {
    if (vaultKey !== null) {
      void loadRecents();
    } else {
      requests++;
      recents = [];
      selectedId = null;
      resetSearch();
    }
  });

  onMount(() => {
    // The panel is a long-lived hidden window: refresh when the vault changes and when it is shown.
    const clearOnHide = () => {
      selectedId = null;
      resetSearch();
      // A half-typed master password does not wait in a hidden window.
      card?.clear();
    };
    const offHidden = onWindowHidden(clearOnHide);
    // The backend hides the panel on blur, but whether the webview then reports `hidden` is not
    // guaranteed (WebView2), so a blur clears the secrets too; it costs nothing, the panel hides anyway.
    window.addEventListener("blur", clearOnHide);
    const refresh = () => {
      if (!unlocked || document.visibilityState === "hidden") return;
      void loadRecents();
      if (searching) void runSearch();
    };
    // A shown panel puts the keyboard where the user starts: the search field or the password.
    const onFocus = () => {
      refresh();
      if (unlocked && selectedId === null) searchInput?.focus();
      else if (!unlocked) card?.focus();
    };
    document.addEventListener("visibilitychange", refresh);
    window.addEventListener("focus", onFocus);
    let off: (() => void) | null = null;
    let gone = false;
    void events.vaultChanged
      .listen(() => refresh())
      .then((unlisten) => {
        if (gone) quietly(unlisten);
        else off = unlisten;
      })
      // Without the listener the lists refresh on focus and on unlock only.
      .catch(() => {});
    return () => {
      gone = true;
      stopSearchTimer();
      if (off) quietly(off);
      offHidden();
      window.removeEventListener("blur", clearOnHide);
      document.removeEventListener("visibilitychange", refresh);
      window.removeEventListener("focus", onFocus);
    };
  });

  function open(id: string) {
    selectedId = id;
    void toasts.guard(() => api.entryTouch(id));
  }

  function onSearchKey(event: KeyboardEvent): void {
    if (event.key === "Escape" && query !== "") {
      event.preventDefault();
      resetSearch();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const first = shown[0];
      if (first) open(first.id);
    }
  }

  const showMain = () => void toasts.guard(() => api.appShowMain());
  const lock = () => void toasts.guard(() => session.lock());
  const openSettings = () => void toasts.guard(() => api.appOpenSettings());
  const quit = () => void toasts.guard(() => api.appQuit());
  const newEntry = () => void toasts.guard(() => api.appNewEntry());
</script>

<main class="panel" data-testid="tray-panel">
  {#if noVault}
    <div class="content">
      <p class="empty">{t("No vault is open")}</p>
    </div>
  {:else if !unlocked}
    <div class="content">
      <UnlockCard bind:this={card} compact />
    </div>
  {:else if selectedId !== null}
    {#key selectedId}
      <TrayDetail id={selectedId} onback={() => (selectedId = null)} />
    {/key}
  {:else}
    <header class="head">
      <h1>{t("Passwords")}</h1>
      <div class="tools">
        <IconButton icon="plus" label={t("New Login")} onclick={newEntry} />
        <IconButton icon="window" label={t("Open main window")} onclick={showMain} />
      </div>
    </header>
    <label class="search">
      <span class="search-icon"><Icon name="search" size={14} /></span>
      <input
        bind:this={searchInput}
        bind:value={query}
        type="search"
        placeholder={t("Search")}
        aria-label={t("Search")}
        autocomplete="off"
        spellcheck="false"
        oninput={scheduleSearch}
        onkeydown={onSearchKey}
      />
    </label>
    <div class="content">
      {#if shown.length === 0}
        <p class="empty">{searching ? t("No results") : t("Nothing here yet")}</p>
      {:else}
        <ul class="entries" aria-label={searching ? t("Search results") : t("Recent items")}>
          {#each shown as item (item.id)}
            <li><TrayEntryRow entry={item} onopen={open} /></li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
  {#if selectedId === null}
    <TrayUpdateRow />
    <nav class="menu" aria-label={t("MyPass")}>
      {#if unlocked}
        <TrayMenuRow label={t("Open MyPass")} shortcut={showShortcut} onclick={showMain} />
        <hr />
        <TrayMenuRow label={t("Lock")} shortcut={lockShortcut} onclick={lock} />
      {:else if noVault}
        <TrayMenuRow label={t("Open MyPass")} shortcut={showShortcut} onclick={showMain} />
        <hr />
      {/if}
      <TrayMenuRow label={t("Settings")} onclick={openSettings} />
      <TrayMenuRow label={t("Quit")} onclick={quit} />
    </nav>
  {/if}
</main>
<ToastHost />

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100vh;
    padding: 10px;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 2px 0 6px;
  }
  h1 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: 700;
  }
  .tools {
    display: flex;
    gap: 2px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: 8px;
    background: var(--control-bg);
    color: var(--text-2);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .search-icon {
    display: inline-flex;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-md);
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .entries {
    list-style: none;
    margin: 0;
    padding: 4px;
    border: var(--sep);
    border-radius: 10px;
    background: var(--list-bg);
  }
  .entries li + li {
    border-top: var(--sep);
  }
  .empty {
    margin: 12px 6px;
    color: var(--text-2);
  }
  .menu {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding-top: 6px;
    border-top: var(--sep);
  }
  hr {
    width: calc(100% - 20px);
    margin: 4px auto;
    border: 0;
    border-top: var(--sep);
  }
</style>
