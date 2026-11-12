<script lang="ts">
  import { api } from "../lib/api";
  import type { Category, GroupNode_Serialize, Scope } from "../lib/bindings";
  import { dragKind, readDrag, startDrag } from "../lib/dnd";
  import { isSelfOrDescendant, visibleGroups } from "../lib/group-tree";
  import { t } from "../lib/i18n/index.svelte";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import type { IconName } from "../lib/ui/icons";
  import Modal from "../lib/ui/Modal.svelte";
  import { treeNav } from "../lib/ui/tree-nav";

  let {
    onnewgroup,
    oneditgroup,
  }: {
    /** Opens the group editor for a new group under `parentId` (`null` is the top level). */
    onnewgroup: (parentId: string | null) => void;
    /** Opens the group editor for an existing group. */
    oneditgroup: (group: { id: string; name: string; emoji: string | null }) => void;
  } = $props();

  // The spec's order; the counts come from the backend by category.
  const CATEGORIES: { category: Category; icon: IconName }[] = [
    { category: "all", icon: "all" },
    { category: "favorites", icon: "star" },
    { category: "codes", icon: "shield-key" },
    { category: "passkeys", icon: "passkey" },
    { category: "addresses", icon: "address" },
    { category: "cards", icon: "card" },
    { category: "notes", icon: "note" },
    { category: "files", icon: "paperclip" },
    { category: "security", icon: "security" },
    { category: "deleted", icon: "trash" },
  ];

  function categoryLabel(category: Category): string {
    switch (category) {
      case "all":
        return t("All");
      case "favorites":
        return t("Favorites");
      case "codes":
        return t("Codes");
      case "passkeys":
        return t("Passkeys");
      case "addresses":
        return t("Addresses");
      case "cards":
        return t("Cards");
      case "notes":
        return t("Notes");
      case "files":
        return t("Files");
      case "security":
        return t("Security");
      case "deleted":
        return t("Deleted");
    }
  }

  const sidebar = $derived(entries.sidebar);
  let collapsed = $state<Record<string, boolean>>({});
  const rows = $derived(visibleGroups(sidebar?.groups ?? [], collapsed));
  const scope = $derived(navigation.scope);

  const countOf = (category: Category): number | null =>
    sidebar?.categories.find((c) => c.category === category)?.count ?? null;

  const isCategory = (s: Scope, category: Category) => s.kind === "category" && s.category === category;
  const keyOf = (s: Scope) => (s.kind === "category" ? `cat:${s.category}` : `grp:${s.groupId}`);

  // Roving tabindex: the focused row, else the selected one, else the first.
  let focusKey = $state<string | null>(null);
  // A key only counts while its row exists (a refresh or a delete can remove the focused row);
  // otherwise the selected row, else the first row, is the tab stop.
  const keys = $derived(new Set([...CATEGORIES.map((c) => `cat:${c.category}`), ...rows.map((r) => `grp:${r.node.id}`)]));
  const tabKey = $derived.by(() => {
    if (focusKey !== null && keys.has(focusKey)) return focusKey;
    const selected = keyOf(scope);
    return keys.has(selected) ? selected : "cat:all";
  });

  function choose(next: Scope) {
    navigation.setScope(next);
  }

  function toggle(id: string, open: boolean) {
    collapsed = { ...collapsed, [id]: !open };
  }

  // ---- context menu and delete ----------------------------------------------------------------

  let menu = $state<{ x: number; y: number; group: GroupNode_Serialize; opener: HTMLElement } | null>(null);
  // The menu closes before `onselect` runs, so the group it was opened for is kept apart from `menu`.
  let menuGroup: GroupNode_Serialize | null = null;
  let confirming = $state<GroupNode_Serialize | null>(null);

  function openMenu(event: MouseEvent, group: GroupNode_Serialize) {
    event.preventDefault();
    const row = event.currentTarget as HTMLElement;
    row.focus();
    menuGroup = group;
    menu = { x: event.clientX, y: event.clientY, group, opener: row };
  }

  function menuFromKeyboard(event: KeyboardEvent, group: GroupNode_Serialize) {
    if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
    event.preventDefault();
    const row = event.currentTarget as HTMLElement;
    const box = row.getBoundingClientRect();
    menuGroup = group;
    menu = { x: box.left + 24, y: box.bottom, group, opener: row };
  }

  function onmenu(id: string) {
    const group = menuGroup;
    if (!group) return;
    if (id === "subgroup") onnewgroup(group.id);
    else if (id === "edit") oneditgroup({ id: group.id, name: group.name, emoji: group.emoji });
    else confirming = group;
  }

  async function deleteGroup() {
    const group = confirming;
    confirming = null;
    if (!group) return;
    // Worked out before the call: once the vault changes the refreshed tree no longer has the group.
    const s = navigation.scope;
    const selectedInside = s.kind === "group" && sidebar !== null && isSelfOrDescendant(sidebar.groups, group.id, s.groupId);
    try {
      await api.groupDelete(group.id);
      // The group is gone: leave a scope that points into it.
      if (selectedInside) choose({ kind: "category", category: "all" });
    } catch (e) {
      toasts.reportError(e);
    }
  }

  // ---- drag and drop --------------------------------------------------------------------------

  let dropKey = $state<string | null>(null);
  // The group being dragged: `dragover` hides the data, so it is remembered from `dragstart` to
  // refuse (no highlight, no drop cursor) itself and its own descendants.
  let draggingGroup: string | null = null;
  const blocked = (targetId: string) =>
    draggingGroup !== null && sidebar !== null && isSelfOrDescendant(sidebar.groups, draggingGroup, targetId);

  async function run(action: () => Promise<unknown>) {
    try {
      await action();
    } catch (e) {
      toasts.reportError(e);
    }
  }

  function over(event: DragEvent, key: string, accepts: (kind: "entry" | "group") => boolean) {
    const kind = dragKind(event);
    if (!kind || !accepts(kind)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    dropKey = key;
  }

  /** Entries and groups can land on a group (or on the top level, `rootGroupId`). */
  async function dropOnGroup(event: DragEvent, targetId: string) {
    event.preventDefault();
    dropKey = null;
    draggingGroup = null;
    const dragged = readDrag(event);
    if (!dragged) return;
    if (dragged.kind === "entry") {
      await run(() => api.entryMove(dragged.id, targetId));
      return;
    }
    // A group cannot move into itself or into one of its own descendants.
    if (sidebar && isSelfOrDescendant(sidebar.groups, dragged.id, targetId)) return;
    await run(() => api.groupMove(dragged.id, targetId));
  }

  /** An entry dropped on Deleted goes to the recycle bin. */
  async function dropOnDeleted(event: DragEvent) {
    event.preventDefault();
    dropKey = null;
    const dragged = readDrag(event);
    if (dragged?.kind === "entry") await run(() => api.entryDelete(dragged.id));
  }

  const leave = (key: string) => {
    if (dropKey === key) dropKey = null;
  };
</script>

<div class="sidebar">
  <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
  <div
    class="tree"
    role="tree"
    aria-label={t("Sidebar")}
    tabindex="-1"
    use:treeNav={{ toggle }}
    onfocusin={(e) => (focusKey = (e.target as HTMLElement).closest<HTMLElement>("[data-key]")?.dataset.key ?? null)}
    onfocusout={(e) => {
      if (!(e.relatedTarget instanceof Node) || !e.currentTarget.contains(e.relatedTarget)) focusKey = null;
    }}
  >
    <div role="group" aria-label={t("Categories")}>
      {#each CATEGORIES as { category, icon } (category)}
        {@const count = countOf(category)}
        {@const key = `cat:${category}`}
        <!-- Keys are handled once for the whole tree (treeNav). -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="row"
          class:selected={isCategory(scope, category)}
          class:drop={dropKey === key}
          role="treeitem"
          aria-level="1"
          aria-selected={isCategory(scope, category)}
          data-key={key}
          tabindex={tabKey === key ? 0 : -1}
          onclick={() => choose({ kind: "category", category })}
          ondragover={category === "deleted" ? (e) => over(e, key, (k) => k === "entry") : undefined}
          ondragleave={() => leave(key)}
          ondrop={category === "deleted" ? dropOnDeleted : undefined}
        >
          <span class="icon"><Icon name={icon} /></span>
          <span class="name">{categoryLabel(category)}</span>
          <span class="count">{count === null ? "—" : count}</span>
        </div>
      {/each}
    </div>

    <div role="group" aria-label={t("Groups")}>
      {#if sidebar}
        <!-- The heading is also the top-level drop target: a group dropped here leaves its parent. -->
        <div
          class="heading"
          class:drop={dropKey === "root"}
          aria-hidden="true"
          ondragover={(e) => over(e, "root", () => true)}
          ondragleave={() => leave("root")}
          ondrop={(e) => void dropOnGroup(e, sidebar.rootGroupId)}
        >
          {t("Groups")}
        </div>
      {/if}
      {#each rows as { node, level, expandable, expanded } (node.id)}
        {@const key = `grp:${node.id}`}
        <div
          class="row"
          class:selected={scope.kind === "group" && scope.groupId === node.id}
          class:drop={dropKey === key}
          role="treeitem"
          aria-level={level}
          aria-selected={scope.kind === "group" && scope.groupId === node.id}
          aria-expanded={expandable ? expanded : undefined}
          data-key={key}
          data-id={node.id}
          style="padding-left: {8 + (level - 1) * 14}px"
          tabindex={tabKey === key ? 0 : -1}
          draggable="true"
          onclick={() => choose({ kind: "group", groupId: node.id })}
          oncontextmenu={(e) => openMenu(e, node)}
          onkeydown={(e) => menuFromKeyboard(e, node)}
          ondragstart={(e) => {
            draggingGroup = node.id;
            startDrag(e, "group", node.id);
          }}
          ondragend={() => (draggingGroup = null)}
          ondragover={(e) => over(e, key, (kind) => kind === "entry" || !blocked(node.id))}
          ondragleave={() => leave(key)}
          ondrop={(e) => void dropOnGroup(e, node.id)}
        >
          {#if expandable}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <span
              class="chevron"
              class:open={expanded}
              aria-hidden="true"
              onclick={(e) => {
                e.stopPropagation();
                toggle(node.id, !expanded);
              }}><Icon name="chevron" size={12} /></span
            >
          {:else}
            <span class="chevron"></span>
          {/if}
          <span class="icon">{#if node.emoji}<span aria-hidden="true">{node.emoji}</span>{:else}<Icon name="folder" />{/if}</span>
          <span class="name">{node.name}</span>
          <span class="count">{node.count}</span>
        </div>
      {/each}
    </div>
  </div>
</div>

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    opener={menu.opener}
    items={[
      { id: "subgroup", label: t("New subgroup") },
      { id: "edit", label: t("Edit group…") },
      { id: "delete", label: t("Delete group"), danger: true },
    ]}
    onselect={onmenu}
    onclose={() => (menu = null)}
  />
{/if}

{#if confirming}
  <Modal open={true} title={t("Delete group")} onclose={() => (confirming = null)}>
    <p class="confirm">{t("Delete this group and move its items to Deleted?")}</p>
    {#snippet footer()}
      <Button onclick={() => (confirming = null)}>{t("Cancel")}</Button>
      <Button variant="danger" onclick={() => void deleteGroup()}>{t("Delete")}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .sidebar {
    height: 100%;
    overflow-y: auto;
    padding: 8px 6px;
    background: var(--sidebar-bg);
  }
  .tree {
    display: flex;
    flex-direction: column;
    gap: 8px;
    outline: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: var(--radius);
    cursor: default;
    user-select: none;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.selected {
    background: var(--accent);
    color: var(--accent-text);
  }
  .row.drop {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .icon {
    display: inline-flex;
    flex: none;
    width: 18px;
    justify-content: center;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    color: var(--text-2);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .selected .count {
    color: inherit;
  }
  .heading {
    padding: 6px 8px 2px;
    color: var(--text-2);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
  }
  .chevron {
    display: inline-flex;
    flex: none;
    width: 12px;
    transition: transform 0.1s;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .confirm {
    margin: 0;
  }
</style>
