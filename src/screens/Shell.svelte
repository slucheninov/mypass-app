<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { platform } from "../lib/platform.svelte";
  import { readPref, writePref } from "../lib/prefs";
  import SplitPane from "../lib/ui/SplitPane.svelte";
  import WindowControls from "../lib/ui/WindowControls.svelte";
  import GroupEditor, { type GroupEditorTarget } from "./GroupEditor.svelte";
  import Sidebar from "./Sidebar.svelte";
  import Toolbar from "./Toolbar.svelte";

  let {
    list,
    detail,
    onimportcsv,
  }: {
    /** The entry list pane (320 px by default). */
    list?: Snippet;
    /** The detail or editor pane (the flexible one). */
    detail?: Snippet;
    /** The CSV import flow, reached from the toolbar's Import menu. */
    onimportcsv?: () => void;
  } = $props();

  const KEY = "layout.widths";
  const MIN: [number, number, number] = [180, 260, 360];
  const MAX = 640;
  const DEFAULT: [number, number] = [220, 320];

  const clampWidth = (value: unknown, min: number, fallback: number): number =>
    typeof value === "number" && Number.isFinite(value) ? Math.round(Math.min(MAX, Math.max(min, value))) : fallback;

  /** Widths from local storage, each clamped to its pane minimum; anything unreadable gives the defaults. */
  function loadWidths(): [number, number] {
    try {
      const parsed: unknown = JSON.parse(readPref(KEY) ?? "null");
      if (Array.isArray(parsed)) return [clampWidth(parsed[0], MIN[0], DEFAULT[0]), clampWidth(parsed[1], MIN[1], DEFAULT[1])];
    } catch {
      // A broken value is the same as none.
    }
    return DEFAULT;
  }

  let widths = $state<[number, number]>(loadWidths());
  let editor = $state<GroupEditorTarget | null>(null);

  function resize(next: [number, number]) {
    widths = next;
    writePref(KEY, JSON.stringify(next));
  }
</script>

<div class="shell" data-testid="vault-shell">
  <header class="titlebar" data-tauri-drag-region>
    <Toolbar onnewgroup={(parentId) => (editor = { mode: "create", parentId })} {onimportcsv} />
    <!-- Last in the DOM so the Windows buttons sit at the right edge; the macOS inset is ordered first. -->
    {#if platform.os}<WindowControls os={platform.os} />{/if}
  </header>
  <div class="body">
    <SplitPane {widths} min={MIN} onresize={resize}>
      {#snippet first()}
        <Sidebar
          onnewgroup={(parentId) => (editor = { mode: "create", parentId })}
          oneditgroup={(group) => (editor = { mode: "edit", ...group })}
        />
      {/snippet}
      {#snippet second()}
        <div class="list">{@render list?.()}</div>
      {/snippet}
      {#snippet third()}
        <!-- Programmatic focus target for the entry list's Enter key (`[data-pane="detail"]`). -->
        <div class="detail" role="region" aria-label={t("Details")} tabindex="-1" data-pane="detail">{@render detail?.()}</div>
      {/snippet}
    </SplitPane>
  </div>
</div>

{#if editor}
  {#key editor}
    <GroupEditor target={editor} onclose={() => (editor = null)} />
  {/key}
{/if}

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  .titlebar {
    display: flex;
    flex: none;
    align-items: stretch;
    height: 44px;
    border-bottom: var(--sep);
    background: var(--bg);
  }
  .body {
    flex: 1;
    min-height: 0;
  }
  .list,
  .detail {
    height: 100%;
    overflow: auto;
  }
  .list {
    background: var(--list-bg);
  }
  .detail {
    background: var(--bg);
  }
  .detail:focus {
    outline: none;
  }
</style>
