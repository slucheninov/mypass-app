<script lang="ts">
  // The middle pane: the entries of the current scope and search as one listbox. Selection lives
  // in the navigation store (`navigation.selectedId`); the detail pane (Task 8) reads it from
  // there. Every action here is a backend command; no secret value ever reaches this component.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { tick } from "svelte";
  import { api } from "../lib/api";
  import type { EntrySummary, FieldRef, SecurityFlag, SortKey } from "../lib/bindings";
  import { startDrag } from "../lib/dnd";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { webUrl } from "../lib/web-url";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Avatar from "../lib/ui/Avatar.svelte";
  import Button from "../lib/ui/Button.svelte";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import type { MenuItem } from "../lib/ui/menu";
  import Modal from "../lib/ui/Modal.svelte";
  import Select from "../lib/ui/Select.svelte";
  import GroupPicker from "./GroupPicker.svelte";

  const uid = $props.id();
  const countId = `entry-count-${uid}`;
  const rowId = (id: string) => `entry-row-${uid}-${id}`;

  const items = $derived(entries.items);
  const query = $derived(entries.query);
  const searching = $derived(query.search !== "");
  const inDeletedScope = $derived(query.scope.kind === "category" && query.scope.category === "deleted");
  const selectedId = $derived(navigation.selectedId);

  let listbox: HTMLElement | undefined = $state();
  let menu = $state<{ x: number; y: number; id: string } | null>(null);
  let confirming = $state<{ kind: "purge"; id: string } | { kind: "empty" } | null>(null);
  let moving = $state<string | null>(null);
  let restoring = $state<string | null>(null);
  // The menu closes before `onselect` runs, so the entry it was opened for is kept here too.
  let menuEntryId: string | null = null;

  const sortOptions: { value: SortKey; label: string }[] = $derived([
    { value: "title", label: t("Title") },
    { value: "modified", label: t("Modified") },
  ]);

  const FLAG_LABEL = (flag: SecurityFlag): string => {
    switch (flag) {
      case "reused":
        return t("Reused password");
      case "weak":
        return t("Weak password");
      case "emptyPassword":
        return t("No password");
      case "insecureUrl":
        return t("Insecure URL");
    }
  };

  // One empty-state text per category, one for a group and one for a search; a heading is a
  // catalog key, so it is translated and found by the key scanner.
  const empty = $derived.by((): { scope: string; heading: string; hint: string | null } => {
    if (searching) {
      return { scope: "search", heading: t("No results"), hint: t("No items match “{0}”.", [query.search]) };
    }
    const scope = query.scope;
    if (scope.kind === "group") return { scope: "group", heading: t("This group is empty"), hint: null };
    const heading = {
      all: t("Nothing here yet"),
      favorites: t("No favorites"),
      codes: t("No items with codes"),
      passkeys: t("No passkeys"),
      addresses: t("No addresses"),
      cards: t("No cards"),
      notes: t("No notes"),
      files: t("No files"),
      security: t("No security issues found"),
      deleted: t("Deleted is empty"),
    }[scope.category];
    return { scope: `cat:${scope.category}`, heading, hint: null };
  });

  const byId = (id: string): EntrySummary | undefined => items.find((e) => e.id === id);
  const activeRow = $derived(selectedId !== null && byId(selectedId) ? rowId(selectedId) : undefined);

  function select(id: string | null) {
    navigation.select(id);
  }

  // Keep the active row visible while the arrow keys move through a long list.
  $effect(() => {
    if (activeRow) document.getElementById(activeRow)?.scrollIntoView?.({ block: "nearest" });
  });

  function move(to: "next" | "previous" | "first" | "last") {
    if (items.length === 0) return;
    const at = items.findIndex((e) => e.id === selectedId);
    const index =
      to === "first"
        ? 0
        : to === "last"
          ? items.length - 1
          : to === "next"
            ? Math.min(at + 1, items.length - 1)
            : at < 0
              ? 0
              : Math.max(at - 1, 0);
    select(items[index]?.id ?? null);
  }

  function focusDetail() {
    document.querySelector<HTMLElement>('[data-pane="detail"]')?.focus();
  }

  // ---- actions ----

  /** Until the sidebar answers the bin is assumed to work (the common case). */
  const binEnabled = $derived(entries.sidebar?.recycleBinEnabled ?? true);
  const guard = (work: () => Promise<unknown>) => toasts.guard(work);

  const copy = (id: string, field: FieldRef) =>
    guard(async () => {
      await api.entryCopy(id, field);
      // The value stays in Rust; the toast only says that something was copied.
      toasts.push("success", t("Copied"));
    });

  const openWebsite = (id: string) =>
    guard(async () => {
      const raw = await api.entryReveal(id, { kind: "url" });
      // Only web pages are ever handed to the system; `javascript:`, `file:` and friends are not.
      const url = webUrl(raw);
      if (url === null) {
        toasts.push("error", t("Invalid input"));
        return;
      }
      await openUrl(url);
    });

  /**
   * The entry the editor has open. Deleting, moving or restoring it from the list would be undone
   * by the editor's Save (the backend moves an entry to the editor's group, which brings a
   * binned one back), so the list refuses those actions until the editor closes.
   */
  const isBeingEdited = (id: string) => navigation.editModel?.id === id;

  const toggleFavorite = (entry: EntrySummary) => guard(() => api.entrySetFavorite(entry.id, !entry.favorite));

  /** Where the selection goes when `id` leaves the list: the row below, else the row above. */
  function neighbourOf(id: string): string | null {
    const at = items.findIndex((e) => e.id === id);
    return items[at + 1]?.id ?? items[at - 1]?.id ?? null;
  }

  const remove = (entry: EntrySummary, permanent: boolean) =>
    guard(async () => {
      const next = neighbourOf(entry.id);
      const unchanged = lockGuard();
      if (permanent) await api.entryPurge(entry.id);
      else await api.entryDelete(entry.id);
      // After a lock the selection is already cleared; a stale one must not outlive it.
      if (unchanged() && navigation.selectedId === entry.id) select(next);
    });

  /**
   * Delete key and menu: an enabled bin takes a live entry at once; anything permanent (an entry
   * already in the bin, or a vault whose bin is disabled) asks first.
   */
  function requestDelete(entry: EntrySummary) {
    if (isBeingEdited(entry.id)) return;
    if (entry.inRecycleBin || !binEnabled) confirming = { kind: "purge", id: entry.id };
    else void remove(entry, false);
  }

  async function confirmed() {
    const what = confirming;
    confirming = null;
    if (!what) return;
    if (what.kind === "purge") {
      const entry = byId(what.id);
      if (entry) await remove(entry, true);
      return;
    }
    await guard(async () => {
      const count = await api.entryEmptyBin();
      toasts.push("success", t("%lld items deleted", count));
      select(null);
    });
  }

  // ---- keyboard ----

  function onkeydown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    const current = selectedId === null ? undefined : byId(selectedId);
    const mod = event.metaKey || event.ctrlKey;
    if (mod && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "c") {
      // A text selection elsewhere keeps its own copy.
      if (current?.entryType === "login" && !window.getSelection()?.toString()) {
        event.preventDefault();
        void copy(current.id, { kind: "password" });
      }
      return;
    }
    if (mod || event.altKey) return;
    switch (event.key) {
      case "ArrowDown":
        move("next");
        break;
      case "ArrowUp":
        move("previous");
        break;
      case "Home":
        move("first");
        break;
      case "End":
        move("last");
        break;
      case "Enter":
        if (!current) return;
        focusDetail();
        break;
      case "Delete":
      case "Backspace":
        if (!current) return;
        requestDelete(current);
        break;
      case "ContextMenu":
        if (current) openMenuForRow(current.id);
        break;
      case "F10":
        if (!event.shiftKey) return;
        if (current) openMenuForRow(current.id);
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  // ---- context menu ----

  function openMenuForRow(id: string) {
    const rect = document.getElementById(rowId(id))?.getBoundingClientRect();
    menuEntryId = id;
    menu = { id, x: rect ? rect.left + 24 : 0, y: rect ? rect.bottom : 0 };
  }

  function oncontextmenu(event: MouseEvent, entry: EntrySummary) {
    event.preventDefault();
    // The list owns focus, so a closed menu hands it back here; the row under the pointer is the selection.
    listbox?.focus();
    select(entry.id);
    menuEntryId = entry.id;
    menu = { id: entry.id, x: event.clientX, y: event.clientY };
  }

  const menuItems = $derived.by((): MenuItem[] => {
    const entry = menu ? byId(menu.id) : undefined;
    if (!entry) return [];
    const login = entry.entryType === "login";
    const edited = isBeingEdited(entry.id);
    return [
      { id: "username", label: t("Copy username"), icon: "copy", disabled: !login },
      { id: "password", label: t("Copy password"), icon: "copy", disabled: !login },
      { id: "website", label: t("Open website"), icon: "link", disabled: !login },
      { id: "sep-1", label: "", separator: true },
      { id: "favorite", label: entry.favorite ? t("Remove from Favorites") : t("Add to Favorites"), icon: "star" },
      // A bin entry is brought back with Restore, not moved.
      ...(entry.inRecycleBin
        ? [
            { id: "restore", label: t("Restore"), icon: "undo" as const, disabled: edited },
            { id: "restore-to", label: t("Restore to…"), icon: "folder" as const, disabled: edited },
          ]
        : []),
      { id: "move", label: t("Move to…"), icon: "folder", disabled: entry.inRecycleBin || edited },
      { id: "sep-2", label: "", separator: true },
      {
        id: "delete",
        label: entry.inRecycleBin || !binEnabled ? t("Delete permanently") : t("Delete"),
        icon: "trash",
        danger: true,
        disabled: edited,
      },
    ];
  });

  function onmenu(action: string) {
    const entry = menuEntryId === null ? undefined : byId(menuEntryId);
    if (!entry) return;
    switch (action) {
      case "username":
        void copy(entry.id, { kind: "username" });
        break;
      case "password":
        void copy(entry.id, { kind: "password" });
        break;
      case "website":
        void openWebsite(entry.id);
        break;
      case "favorite":
        void toggleFavorite(entry);
        break;
      case "move":
        moving = entry.id;
        break;
      case "restore":
        void restore(entry.id, null);
        break;
      case "restore-to":
        restoring = entry.id;
        break;
      case "delete":
        requestDelete(entry);
        break;
    }
  }

  async function restore(id: string, groupId: string | null) {
    await guard(() => api.entryRestore(id, groupId));
    await tick();
    listbox?.focus();
  }

  function restoreTo(groupId: string) {
    const id = restoring;
    restoring = null;
    if (id !== null) void restore(id, groupId);
  }

  async function moveTo(groupId: string) {
    const id = moving;
    moving = null;
    if (id === null) return;
    await guard(() => api.entryMove(id, groupId));
    await tick();
    listbox?.focus();
  }
</script>

<section class="entries" data-testid="entry-list">
  <header class="head">
    <span class="count" id={countId} data-testid="list-count"
      >{searching ? t("%lld matching items", items.length) : t("%lld items", items.length)}</span
    >
    {#if inDeletedScope && items.length > 0}
      <Button variant="plain" onclick={() => (confirming = { kind: "empty" })}>{t("Empty Deleted")}</Button>
    {/if}
    <Select value={query.sort} options={sortOptions} label={t("Sort by")} onchange={(sort) => void entries.setSort(sort)} />
  </header>

  {#if items.length === 0}
    {#if entries.loaded}
      <div class="empty" data-testid="empty-state" data-scope={empty.scope} role="status">
        <p class="heading" data-part="heading">{empty.heading}</p>
        {#if empty.hint}<p class="hint">{empty.hint}</p>{/if}
      </div>
    {/if}
  {:else}
    <div
      class="listbox"
      role="listbox"
      tabindex="0"
      aria-labelledby={countId}
      aria-activedescendant={activeRow}
      bind:this={listbox}
      {onkeydown}
    >
      {#each items as entry (entry.id)}
        <!-- Keyboard use is the listbox's (aria-activedescendant); the row only answers the pointer. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="row"
          class:selected={entry.id === selectedId}
          role="option"
          id={rowId(entry.id)}
          tabindex="-1"
          aria-selected={entry.id === selectedId}
          draggable={!entry.inRecycleBin && !isBeingEdited(entry.id)}
          onclick={() => {
            // The listbox keeps focus, so aria-activedescendant is announced for the clicked row.
            listbox?.focus();
            select(entry.id);
          }}
          oncontextmenu={(e) => oncontextmenu(e, entry)}
          ondragstart={(e) => startDrag(e, "entry", entry.id)}
        >
          <Avatar text={entry.title} size={32} />
          <span class="text">
            <span class="title" data-part="title">{entry.title}</span>
            {#if entry.subtitle}<span class="subtitle">{entry.subtitle}</span>{/if}
            {#if entry.flags.length > 0}
              <span class="flags">
                {#each entry.flags as flag (flag)}<span class="chip">{FLAG_LABEL(flag)}</span>{/each}
              </span>
            {/if}
          </span>
          <span class="badges">
            {#if entry.favorite}<span role="img" aria-label={t("Favorites")}><Icon name="star" size={13} /></span>{/if}
            {#if entry.hasTotp}<span role="img" aria-label={t("Verification Code")}><Icon name="shield-key" size={13} /></span>{/if}
            {#if entry.hasPasskey}<span role="img" aria-label={t("Passkey")}><Icon name="passkey" size={13} /></span>{/if}
            {#if entry.attachmentCount > 0}
              <span role="img" aria-label={t("%lld files", entry.attachmentCount)}
                ><Icon name="paperclip" size={13} />{entry.attachmentCount}</span
              >
            {/if}
          </span>
        </div>
      {/each}
    </div>
  {/if}
</section>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} opener={listbox ?? null} items={menuItems} onselect={onmenu} onclose={() => (menu = null)} />
{/if}

{#if confirming}
  <Modal
    open={true}
    title={confirming.kind === "empty" ? t("Empty Deleted") : t("Delete permanently")}
    onclose={() => (confirming = null)}
  >
    <p class="confirm">
      {confirming.kind === "empty"
        ? t("Empty Deleted? This cannot be undone.")
        : t("This entry will be permanently deleted. This cannot be undone.")}
    </p>
    {#snippet footer()}
      <Button onclick={() => (confirming = null)}>{t("Cancel")}</Button>
      <Button variant="danger" onclick={() => void confirmed()}
        >{confirming?.kind === "empty" ? t("Empty Deleted") : t("Delete permanently")}</Button
      >
    {/snippet}
  </Modal>
{/if}

<GroupPicker
  open={restoring !== null}
  title={t("Choose a group")}
  onpick={restoreTo}
  onclose={() => (restoring = null)}
/>

<GroupPicker
  open={moving !== null}
  onpick={(groupId) => void moveTo(groupId)}
  onclose={() => (moving = null)}
  current={moving === null ? null : (byId(moving)?.groupId ?? null)}
/>

<style>
  .entries {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .head {
    display: flex;
    flex: none;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: var(--sep);
  }
  .count {
    flex: 1;
    color: var(--text-2);
    font-size: 12px;
  }
  .listbox {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--separator);
    cursor: default;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.selected {
    background: var(--selected);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    gap: 1px;
  }
  .title {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .subtitle {
    overflow: hidden;
    color: var(--text-2);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .flags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 2px;
  }
  .chip {
    padding: 0 6px;
    border: 1px solid var(--danger);
    border-radius: 999px;
    color: var(--danger);
    font-size: 11px;
  }
  .badges {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: 11px;
  }
  .badges > span {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .empty {
    display: flex;
    flex: 1;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px;
    color: var(--text-2);
    text-align: center;
  }
  .heading {
    margin: 0;
    font-weight: 600;
  }
  .hint {
    margin: 4px 0 0;
    overflow-wrap: anywhere;
  }
  .confirm {
    margin: 0;
  }
</style>
