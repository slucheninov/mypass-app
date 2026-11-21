<script lang="ts">
  // The detail pane while nothing is selected: a calm, centered picture of the current scope (the
  // vault, a category or a group) with its item count. Purely decorative art; the text says it all.
  import { categoryIcon, categoryLabel } from "../lib/categories";
  import { findGroup } from "../lib/group-tree";
  import { t } from "../lib/i18n/index.svelte";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { session } from "../lib/stores/session.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import type { IconName } from "../lib/ui/icons";
  import { fileNameOf } from "./vault-status";

  type View =
    | { kind: "vault"; title: string; count: number | null }
    | { kind: "category"; icon: IconName; title: string; count: number | null }
    | { kind: "group"; emoji: string | null; title: string; count: number | null };

  /** The vault's display name: its database name, else the file name without `.kdbx`, else the app name. */
  function vaultTitle(): string {
    const name = session.status.vaultName?.trim();
    if (name) return name;
    const path = session.status.vaultPath;
    if (path) return fileNameOf(path).replace(/\.kdbx$/i, "");
    return t("MyPass");
  }

  const view = $derived.by((): View => {
    const scope = navigation.scope;
    const sidebar = entries.sidebar;
    if (scope.kind === "group") {
      const node = sidebar ? findGroup(sidebar.groups, scope.groupId) : null;
      return { kind: "group", emoji: node?.emoji ?? null, title: node?.name ?? "", count: node?.count ?? null };
    }
    // Security has no count (the backend sends `null`), so none is shown.
    const count = sidebar?.categories.find((c) => c.category === scope.category)?.count ?? null;
    if (scope.category === "all") return { kind: "vault", title: vaultTitle(), count };
    return { kind: "category", icon: categoryIcon(scope.category), title: categoryLabel(scope.category), count };
  });
</script>

<div class="empty" data-testid="detail-empty" data-kind={view.kind}>
  <div class="art" aria-hidden="true">
    {#if view.kind === "vault"}
      <svg class="key" width="120" height="120" viewBox="0 0 120 120" fill="none" focusable="false">
        <circle cx="38" cy="60" r="22" />
        <circle cx="38" cy="60" r="7" />
        <path d="M60 60h46 M88 60v14 M99 60v10" />
      </svg>
    {:else if view.kind === "category"}
      <Icon name={view.icon} size={72} stroke={1} />
    {:else if view.emoji}
      <span class="emoji">{view.emoji}</span>
    {:else}
      <Icon name="folder" size={72} stroke={1} />
    {/if}
  </div>
  {#if view.title}<p class="title" data-part="title">{view.title}</p>{/if}
  {#if view.count !== null}<p class="count" data-part="count">{t("%lld items", view.count)}</p>{/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 100%;
    padding: var(--space-8);
    color: var(--text-2);
    text-align: center;
  }
  .art {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 120px;
    margin-bottom: var(--space-4);
    color: var(--text-2);
    opacity: 0.55;
  }
  .key {
    stroke: var(--text-2);
    stroke-width: 5;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.65;
  }
  .emoji {
    font-size: 64px;
    line-height: 1;
    opacity: 0.9;
  }
  .title {
    max-width: 100%;
    margin: 0;
    overflow: hidden;
    color: var(--text-2);
    font-size: 26px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    margin: 0;
    color: var(--text-2);
    font-size: var(--font-size-md);
  }
</style>
