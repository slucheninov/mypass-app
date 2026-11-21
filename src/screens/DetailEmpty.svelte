<script lang="ts">
  // The detail pane while nothing is selected: a calm, centered picture of the current scope (the
  // vault, a category or a group) with its item count, and the file size for the whole vault.
  // Purely decorative art; the text says it all.
  import { categoryIcon, categoryLabel } from "../lib/categories";
  import { findGroup } from "../lib/group-tree";
  import { formatBytes } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { entries } from "../lib/stores/entries.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { session } from "../lib/stores/session.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import type { IconName } from "../lib/ui/icons";
  import { connectedInfo, fileNameOf } from "./vault-status";

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

  // The vault file's size on disk (the unlock screen's reading), shown with the whole vault only.
  const size = $derived(view.kind === "vault" ? (connectedInfo(session.status.fileStatus)?.size ?? null) : null);
</script>

<div class="empty" data-testid="detail-empty" data-kind={view.kind}>
  <div class="art" aria-hidden="true">
    {#if view.kind === "vault"}
      <!-- The app icon's key (scripts/make-app-icons.mjs, same key units), as an outline only. -->
      <svg class="key" width="82" height="150" viewBox="22 -2 56 102" fill="none" focusable="false">
        <path d="M56 49.24V54L63 60L57 65L63 70L57 75L63 80L56 86V89L48.5 96.5L41 89V48.25A24 24 0 1 1 56 49.24Z" />
        <circle cx="50" cy="19.5" r="6.5" />
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
  {#if view.count !== null || size !== null}
    <p class="count">
      {#if view.count !== null}<span data-part="count">{t("%lld items", view.count)}</span>{/if}
      {#if view.count !== null && size !== null}<span class="sep" aria-hidden="true">·</span>{/if}
      {#if size !== null}<span data-part="size">{formatBytes(size)}</span>{/if}
    </p>
  {/if}
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
    height: 150px;
    margin-bottom: var(--space-4);
    color: var(--text-2);
    opacity: 0.55;
  }
  .key {
    stroke: var(--text-2);
    stroke-width: 2;
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
  .sep {
    margin: 0 0.4em;
  }
</style>
