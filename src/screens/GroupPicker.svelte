<script lang="ts">
  import { isSelfOrDescendant, visibleGroups } from "../lib/group-tree";
  import { t } from "../lib/i18n/index.svelte";
  import { entries } from "../lib/stores/entries.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import { treeNav } from "../lib/ui/tree-nav";

  /**
   * Chooses a group from the vault's group tree (the recycle bin is never offered). Used by "Move
   * to Group" and "Restore to...". Activating a row calls `onpick(groupId)`; the caller closes it.
   */
  let {
    open,
    title,
    onpick,
    onclose,
    current = null,
    exclude = null,
    includeRoot = true,
  }: {
    open: boolean;
    /** Dialog title, already translated; defaults to "Move to Group". */
    title?: string;
    onpick: (groupId: string) => void;
    onclose: () => void;
    /** The group the item is in now: shown, but not selectable. */
    current?: string | null;
    /** A group whose whole subtree is not selectable (the group being moved). */
    exclude?: string | null;
    /** Offer the top level (the vault's root group) as the first row. */
    includeRoot?: boolean;
  } = $props();

  let collapsed = $state<Record<string, boolean>>({});
  const sidebar = $derived(entries.sidebar);
  const rows = $derived(visibleGroups(sidebar?.groups ?? [], collapsed));

  function disabled(id: string): boolean {
    if (id === current) return true;
    return exclude !== null && sidebar !== null && isSelfOrDescendant(sidebar.groups, exclude, id);
  }

  function toggle(id: string, openIt: boolean) {
    collapsed = { ...collapsed, [id]: !openIt };
  }
</script>

<Modal {open} title={title ?? t("Move to Group")} {onclose}>
  <div class="tree" role="tree" aria-label={t("Groups")} use:treeNav={{ toggle }}>
    {#if includeRoot && sidebar}
      <button
        type="button"
        class="row"
        role="treeitem"
        aria-level="1"
        aria-selected="false"
        aria-disabled={current === sidebar.rootGroupId ? "true" : undefined}
        data-id={sidebar.rootGroupId}
        onclick={() => current !== sidebar.rootGroupId && onpick(sidebar.rootGroupId)}
      >
        <span class="icon"><Icon name="folder" /></span>
        <span class="name">{t("Vault")}</span>
      </button>
    {/if}
    {#each rows as { node, level, expandable, expanded } (node.id)}
      <button
        type="button"
        class="row"
        role="treeitem"
        aria-level={includeRoot ? level + 1 : level}
        aria-selected="false"
        aria-disabled={disabled(node.id) ? "true" : undefined}
        aria-expanded={expandable ? expanded : undefined}
        data-id={node.id}
        style="padding-left: {8 + (level - (includeRoot ? 0 : 1)) * 14}px"
        onclick={() => !disabled(node.id) && onpick(node.id)}
      >
        <span class="icon">{#if node.emoji}<span aria-hidden="true">{node.emoji}</span>{:else}<Icon name="folder" />{/if}</span>
        <span class="name">{node.name}</span>
      </button>
    {/each}
  </div>
  {#snippet footer()}
    <Button onclick={onclose}>{t("Cancel")}</Button>
  {/snippet}
</Modal>

<style>
  .tree {
    display: flex;
    flex-direction: column;
    min-width: 280px;
    max-height: 320px;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .row:hover:not([aria-disabled="true"]),
  .row:focus-visible {
    background: var(--accent);
    color: var(--accent-text);
  }
  .row[aria-disabled="true"] {
    opacity: 0.4;
  }
  .icon {
    display: inline-flex;
    width: 18px;
    justify-content: center;
  }
  .name {
    flex: 1;
  }
</style>
