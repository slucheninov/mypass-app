<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";
  import { api } from "../lib/api";
  import { events, type EntrySummary } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { platform } from "../lib/platform.svelte";
  import { onWindowHidden } from "../lib/secret.svelte";
  import { formatShortcut } from "../lib/shortcuts";
  import { session } from "../lib/stores/session.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import { quietly } from "../lib/unlisten";
  import UnlockCard from "../screens/UnlockCard.svelte";
  import TrayActions from "./TrayActions.svelte";
  import TrayEntryRow from "./TrayEntryRow.svelte";
  import TrayMenuRow from "./TrayMenuRow.svelte";
  import TrayUpdateRow from "./TrayUpdateRow.svelte";

  const RECENT_LIMIT = 5;
  const recentKey = (id: string) => `recent:${id}`;

  let recents = $state<EntrySummary[]>([]);
  type Selection = { entry: EntrySummary; epoch: number };
  let selected = $state.raw<Selection | null>(null);
  let highlight = $state<string | null>(null);
  let card: UnlockCard | undefined = $state();
  let panel: HTMLElement;
  let menu: HTMLElement | undefined = $state();
  let requests = 0;

  const unlocked = $derived(session.status.state === "unlocked");
  const actions = $derived(selected?.epoch === session.lockEpoch && unlocked ? selected : null);
  const noVault = $derived(session.status.state === "noVault");
  const vaultKey = $derived(unlocked ? (session.status.vaultPath ?? "") : null);
  const lockShortcut = $derived(formatShortcut(settings.value?.shortcutLock ?? "", platform.os));
  const showShortcut = $derived(formatShortcut(settings.value?.shortcutShow ?? "", platform.os));
  const primaryModifier = $derived(platform.os === "macos" ? "metaKey" : "ctrlKey");
  const settingsShortcut = $derived(primaryModifier === "metaKey" ? "⌘," : "Ctrl+,");
  const quitShortcut = $derived(primaryModifier === "metaKey" ? "⌘Q" : "Ctrl+Q");

  // Keep a selected recent among the recents if one disappears. A list refresh must not move
  // a keyboard selection onto a different kind of action, such as Quit.
  const activeKey = $derived.by(() => {
    if (highlight?.startsWith("recent:") && !recents.some((item) => recentKey(item.id) === highlight)) {
      const last = recents.at(-1);
      return last ? recentKey(last.id) : null;
    }
    return highlight;
  });

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

  $effect(() => {
    const key = vaultKey;
    requests++;
    recents = [];
    selected = null;
    highlight = null;
    if (key !== null) void loadRecents();
  });

  function clearOnHide(): void {
    selected = null;
    highlight = null;
    card?.clear();
  }

  function hide(): void {
    clearOnHide();
    void toasts.guard(() => api.trayPanelHide());
  }

  onMount(() => {
    const root = document.documentElement;
    root.dataset.material = "solid";
    // WebView2 shares its preferred colour scheme across the app's webviews.
    // The tray follows its own native window theme.
    root.dataset.theme = "light";
    let gone = false;
    let offTheme: (() => void) | undefined;
    void (async () => {
      const win = getCurrentWindow();
      let changed = false;
      const unlisten = await win.onThemeChanged(({ payload }) => {
        if (gone) return;
        changed = true;
        root.dataset.theme = payload;
      });
      if (gone) {
        quietly(unlisten);
        return;
      }
      offTheme = unlisten;
      const theme = await win.theme();
      if (!gone && !changed && theme) root.dataset.theme = theme;
    })().catch(() => {});
    let frame = 0;
    const fit = () => {
      if (frame !== 0) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const height = Math.max(1, Math.ceil(panel.scrollHeight));
        void api.trayPanelFit(height).then((material) => {
          if (!gone) root.dataset.material = material;
        }).catch(() => {});
      });
    };
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(fit);
    observer?.observe(panel);
    fit();

    const refresh = () => {
      if (unlocked && document.visibilityState !== "hidden") void loadRecents();
    };
    const onFocus = () => {
      refresh();
      fit();
      if (!unlocked) card?.focus();
    };
    const offHidden = onWindowHidden(clearOnHide);
    // WebView2 does not always report visibility changes when the native host hides the window.
    window.addEventListener("blur", clearOnHide);
    document.addEventListener("visibilitychange", refresh);
    window.addEventListener("focus", onFocus);
    let off: (() => void) | null = null;
    void events.vaultChanged.listen(refresh).then((unlisten) => {
      if (gone) quietly(unlisten);
      else off = unlisten;
    }).catch(() => {});
    return () => {
      gone = true;
      observer?.disconnect();
      cancelAnimationFrame(frame);
      delete root.dataset.material;
      delete root.dataset.theme;
      if (offTheme) quietly(offTheme);
      if (off) quietly(off);
      offHidden();
      window.removeEventListener("blur", clearOnHide);
      document.removeEventListener("visibilitychange", refresh);
      window.removeEventListener("focus", onFocus);
    };
  });

  async function focusHighlight(): Promise<void> {
    await tick();
    menuRows().find((row) => row.dataset.menuKey === activeKey)?.focus();
  }

  function menuRows(): HTMLButtonElement[] {
    return Array.from(menu?.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]') ?? []);
  }

  function open(id: string): void {
    const entry = recents.find((item) => item.id === id);
    if (!entry || !unlocked) return;
    const selection = { entry, epoch: session.lockEpoch };
    selected = selection;
    highlight = "back";
    void focusHighlight();
    void api.entryTouch(id).catch((e) => {
      if (actions === selection) toasts.reportError(e);
    });
  }

  function back(): void {
    const entry = actions?.entry;
    selected = null;
    highlight = entry ? recentKey(entry.id) : null;
    void focusHighlight();
  }

  const showMain = () => void toasts.guard(() => api.appShowMain());
  const lock = () => void toasts.guard(() => session.lock());
  const openSettings = () => void toasts.guard(() => api.appOpenSettings());
  const quit = () => void toasts.guard(() => api.appQuit());

  function onKey(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.isComposing) return;
    if (event.key === "Escape" || (event.key === "ArrowLeft" && actions)) {
      event.preventDefault();
      if (actions) back();
      else hide();
      return;
    }
    if (event[primaryModifier] && !event.altKey && !event.shiftKey) {
      if (event.key === "," || event.key.toLowerCase() === "q") {
        event.preventDefault();
        if (!event.repeat) (event.key === "," ? openSettings : quit)();
      }
      return;
    }
    if (event.target instanceof HTMLElement && event.target.closest("input, textarea, select, [contenteditable]")) return;
    if (!menu || event.metaKey || event.ctrlKey || event.altKey) return;

    const rows = menuRows();
    if (rows.length === 0) return;
    const index = rows.findIndex((row) => row.dataset.menuKey === activeKey);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const next = event.key === "ArrowDown"
        ? (index + 1) % rows.length
        : (index <= 0 ? rows.length - 1 : index - 1);
      const row = rows[next];
      highlight = row?.dataset.menuKey ?? null;
      row?.focus();
    } else if (event.key === "Enter" || (event.key === "ArrowRight" && rows[index]?.hasAttribute("aria-haspopup"))) {
      const row = rows[index];
      if (row) {
        event.preventDefault();
        if (!event.repeat) row.click();
      }
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<main class="panel" data-testid="tray-panel" bind:this={panel} onpointerleave={() => (highlight = null)}>
  {#if noVault}
    <p class="empty">{t("No vault is open")}</p>
  {:else if !unlocked}
    <UnlockCard bind:this={card} compact />
  {/if}
  <div role="menu" aria-label={actions?.entry.title ?? t("MyPass")} bind:this={menu}>
    {#if actions}
      {@const selection = actions}
      {#key selection}
        <TrayActions
          entry={selection.entry}
          highlight={activeKey}
          onback={back}
          ondone={hide}
          onhover={(key) => (highlight = key)}
          isCurrent={() => actions === selection}
        />
      {/key}
    {:else}
      {#if unlocked}
        <div class="heading" aria-hidden="true">{t("Recent")}</div>
        {#if recents.length === 0}
          <p class="empty">{t("Nothing here yet")}</p>
        {:else}
          <ul class="entries" role="group" aria-label={t("Recent")}>
            {#each recents as item (item.id)}
              {@const key = recentKey(item.id)}
              <li role="none">
                <TrayEntryRow entry={item} {key} active={activeKey === key} onopen={open} onhover={() => (highlight = key)} />
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
      <TrayUpdateRow active={activeKey === "update"} onhover={() => (highlight = "update")} />
      <hr />
      {#if unlocked || noVault}
        <TrayMenuRow key="open" label={t("Open MyPass")} icon="window" shortcut={showShortcut} active={activeKey === "open"} onhover={() => (highlight = "open")} onclick={showMain} />
      {/if}
      {#if unlocked}
        <TrayMenuRow key="lock" label={t("Lock")} icon="lock" shortcut={lockShortcut} active={activeKey === "lock"} onhover={() => (highlight = "lock")} onclick={lock} />
      {/if}
      <TrayMenuRow key="settings" label={t("Settings…")} icon="gear" shortcut={settingsShortcut} active={activeKey === "settings"} onhover={() => (highlight = "settings")} onclick={openSettings} />
      <hr />
      <TrayMenuRow key="quit" label={t("Quit")} icon="power" shortcut={quitShortcut} active={activeKey === "quit"} onhover={() => (highlight = "quit")} onclick={quit} />
    {/if}
  </div>
</main>
<ToastHost />

<style>
  :global(html[data-material]) {
    overflow-y: auto;
    scrollbar-width: thin;
  }
  :global(html[data-material="native"] body) {
    background: transparent;
  }
  :global(html[data-material="solid"] body) {
    background: Canvas;
  }
  .panel {
    --menu-accent: #0a64d6;
    padding: 5px;
    border-radius: 12px;
    background: transparent;
    color: CanvasText;
    font: 13px system-ui, sans-serif;
  }
  @supports (color: Highlight) {
    .panel { --menu-accent: Highlight; }
  }
  @supports (color: AccentColor) {
    .panel { --menu-accent: AccentColor; }
  }
  .heading {
    padding: 4px 9px 2px;
    color: GrayText;
    font-size: 11px;
    font-weight: 600;
  }
  .empty {
    margin: 0;
    padding: 5px 9px;
    color: GrayText;
  }
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  hr {
    height: 1px;
    margin: 4px 9px;
    border: 0;
    background: color-mix(in srgb, CanvasText 12%, transparent);
  }
</style>
