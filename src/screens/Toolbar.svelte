<script lang="ts">
  import { api } from "../lib/api";
  import type { EntryTypeDto } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { openVaultFromDialog } from "../lib/open-flow";
  import { lockGuard } from "../lib/lock-guard";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import IconButton from "../lib/ui/IconButton.svelte";
  import Menu from "../lib/ui/Menu.svelte";
  import type { MenuItem } from "../lib/ui/menu";
  import SaveIndicator from "./SaveIndicator.svelte";

  let {
    onnewgroup,
    onimportcsv,
  }: {
    /** Opens the group editor for a new group under `parentId` (`null` is the top level). */
    onnewgroup: (parentId: string | null) => void;
    /** The CSV import flow (wired by the import task); the menu item is always listed. */
    onimportcsv?: () => void;
  } = $props();

  let search: HTMLInputElement | undefined = $state();

  const newItems = $derived<MenuItem[]>([
    { id: "login", label: t("New Login") },
    { id: "address", label: t("New Address") },
    { id: "card", label: t("New Card") },
    { id: "notes", label: t("New Note") },
    { id: "files", label: t("New Files item") },
    { id: "sep", label: "", separator: true },
    { id: "group", label: t("New Group") },
  ]);
  const importItems = $derived<MenuItem[]>([
    { id: "csv", label: t("Import CSV…") },
    { id: "kdbx", label: t("Open Existing .kdbx…") },
  ]);

  /** The group a new item or group lands in: the selected group, else the backend's default. */
  function currentGroup(): string | null {
    return navigation.scope.kind === "group" ? navigation.scope.groupId : null;
  }

  async function createItem(id: string) {
    if (id === "group") {
      onnewgroup(currentGroup());
      return;
    }
    const unchanged = lockGuard();
    await toasts.guard(async () => {
      const model = await api.entryNewModel(id as EntryTypeDto, currentGroup());
      // A lock that landed meanwhile must not leave an editor behind for the next unlock.
      if (unchanged()) navigation.openEditor(model);
    });
  }

  function importItem(id: string) {
    if (id === "csv") onimportcsv?.();
    else void openVaultFromDialog();
  }

  async function lock() {
    await toasts.guard(() => session.lock());
  }

  async function openSettings() {
    await toasts.guard(() => api.appOpenSettings());
  }

  function onkeydown(event: KeyboardEvent) {
    if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "f") {
      event.preventDefault();
      search?.focus();
      search?.select();
    }
  }
</script>

<svelte:window {onkeydown} />

<!-- The drag region is on the toolbar on every OS: the macOS overlay title bar has no drag area of its own. -->
<div class="toolbar" data-testid="toolbar" data-tauri-drag-region>
  <Menu items={newItems} onselect={(id) => void createItem(id)} label={t("New Entry")}>
    {#snippet trigger()}<Icon name="plus" />{/snippet}
  </Menu>
  <div class="search">
    <Icon name="search" size={14} />
    <input type="search" aria-label={t("Search")} placeholder={t("Search")} bind:this={search} value={entries.searchText} oninput={(e) => entries.setSearch(e.currentTarget.value)} />
  </div>
  <span class="spacer" data-tauri-drag-region></span>
  <SaveIndicator />
  <Menu items={importItems} onselect={importItem} label={t("Import")}>
    {#snippet trigger()}<Icon name="import" />{/snippet}
  </Menu>
  <IconButton icon="lock" label={t("Lock")} onclick={() => void lock()} />
  <IconButton icon="gear" label={t("Settings")} onclick={() => void openSettings()} />
</div>

<style>
  .toolbar {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    min-width: 0;
    height: 100%;
    padding: 0 12px;
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 240px;
    min-width: 120px;
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text-2);
  }
  .search input {
    flex: 1;
    min-width: 0;
    padding: 4px 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
  }
  .search:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
