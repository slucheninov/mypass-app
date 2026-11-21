<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { platform } from "../lib/platform.svelte";
  import { readPref, writePref } from "../lib/prefs";
  import IconButton from "../lib/ui/IconButton.svelte";
  import SplitPane from "../lib/ui/SplitPane.svelte";
  import WindowControls from "../lib/ui/WindowControls.svelte";
  import GroupEditor, { type GroupEditorTarget } from "./GroupEditor.svelte";
  import PairingCodeSheet from "./PairingCodeSheet.svelte";
  import ProposalHandler from "./ProposalHandler.svelte";
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

  // ---- hiding the sidebar ---------------------------------------------------------------------

  const COLLAPSED_KEY = "layout.sidebarCollapsed";
  const uid = $props.id();
  const sidebarId = `sidebar-${uid}`;
  let sidebarCollapsed = $state(readPref(COLLAPSED_KEY) === "true");
  let toggleBox: HTMLElement | undefined = $state();

  // One shortcut per system, both free in the app and among the defaults of the global hotkeys:
  // Control-Command-S is the macOS convention (Mail, Notes), Ctrl+Shift+S elsewhere.
  const mac = $derived(platform.os === "macos");
  const shortcutLabel = $derived(mac ? "\u2303\u2318S" : "Ctrl+Shift+S");
  const shortcutAria = $derived(mac ? "Control+Meta+S" : "Control+Shift+S");

  function isToggleShortcut(e: KeyboardEvent): boolean {
    // `code` is the physical key, so a Cyrillic layout works the same.
    if (e.code !== "KeyS" || e.altKey) return false;
    return mac ? e.metaKey && e.ctrlKey && !e.shiftKey : e.ctrlKey && e.shiftKey && !e.metaKey;
  }

  function setSidebarCollapsed(next: boolean) {
    if (next === sidebarCollapsed) return;
    // Focus inside the sidebar would vanish with it: it moves to the list, else to the toggle.
    const active = document.activeElement;
    const focusInside = next && active instanceof Node && document.getElementById(sidebarId)?.contains(active);
    sidebarCollapsed = next;
    writePref(COLLAPSED_KEY, String(next));
    if (focusInside) {
      const target =
        document.querySelector<HTMLElement>('[data-pane="list"] [role="listbox"]') ?? toggleBox?.querySelector("button");
      target?.focus();
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || e.repeat || !isToggleShortcut(e)) return;
    e.preventDefault();
    setSidebarCollapsed(!sidebarCollapsed);
  }
</script>

<svelte:window {onkeydown} />

<div class="shell" data-testid="vault-shell">
  <header class="titlebar" class:windows={platform.os === "windows"} data-tauri-drag-region>
    <!-- Right after the macOS traffic lights (the inset is ordered first), at the left on Windows. -->
    <div class="sidebar-toggle" bind:this={toggleBox} data-tauri-drag-region>
      <IconButton
        icon="sidebar"
        label={sidebarCollapsed ? t("Show sidebar") : t("Hide sidebar")}
        tooltip={`${sidebarCollapsed ? t("Show sidebar") : t("Hide sidebar")} (${shortcutLabel})`}
        expanded={!sidebarCollapsed}
        controls={sidebarId}
        keyshortcuts={shortcutAria}
        onclick={() => setSidebarCollapsed(!sidebarCollapsed)}
      />
    </div>
    <Toolbar onnewgroup={(parentId) => (editor = { mode: "create", parentId })} {onimportcsv} />
    <!-- Last in the DOM so the Windows buttons sit at the right edge; the macOS inset is ordered first. -->
    {#if platform.os}<WindowControls os={platform.os} />{/if}
  </header>
  <div class="body">
    <SplitPane
      {widths}
      min={MIN}
      onresize={resize}
      collapsed={sidebarCollapsed}
      oncollapse={setSidebarCollapsed}
      firstId={sidebarId}
    >
      {#snippet first()}
        <Sidebar
          onnewgroup={(parentId) => (editor = { mode: "create", parentId })}
          oneditgroup={(group) => (editor = { mode: "edit", ...group })}
        />
      {/snippet}
      {#snippet second()}
        <div class="list" data-pane="list">{@render list?.()}</div>
      {/snippet}
      {#snippet third()}
        <!-- Programmatic focus target for the entry list's Enter key (`[data-pane="detail"]`). -->
        <div class="detail" role="region" aria-label={t("Details")} tabindex="-1" data-pane="detail">{@render detail?.()}</div>
      {/snippet}
    </SplitPane>
  </div>
</div>

<!-- Browser bridge, main window only: both die with the unlocked stretch, so a lock closes them. -->
<PairingCodeSheet />
<ProposalHandler />

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
  .titlebar.windows {
    height: 32px;
  }
  .titlebar {
    display: flex;
    flex: none;
    align-items: stretch;
    height: 44px;
    border-bottom: var(--sep);
    background: var(--bg);
  }
  .sidebar-toggle {
    display: flex;
    flex: none;
    align-items: center;
    padding-left: var(--space-4);
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
