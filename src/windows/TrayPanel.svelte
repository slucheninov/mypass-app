<script lang="ts">
  // The tray panel shows the current vault's recents or the unlock card.
  import { onMount } from "svelte";
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
  import IconButton from "../lib/ui/IconButton.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import { quietly } from "../lib/unlisten";
  import UnlockCard from "../screens/UnlockCard.svelte";
  import TrayDetail from "./TrayDetail.svelte";
  import TrayEntryRow from "./TrayEntryRow.svelte";
  import TrayMenuRow from "./TrayMenuRow.svelte";
  import TrayUpdateRow from "./TrayUpdateRow.svelte";

  const RECENT_LIMIT = 5;

  let recents = $state<EntrySummary[]>([]);
  let selectedId = $state<string | null>(null);
  let card: UnlockCard | undefined = $state();
  let requests = 0;

  const unlocked = $derived(session.status.state === "unlocked");
  const noVault = $derived(session.status.state === "noVault");
  // One key per unlocked stretch and vault: a save-state change replaces `status` but not this.
  const vaultKey = $derived(unlocked ? (session.status.vaultPath ?? "") : null);
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

  // Locking or switching vaults discards any outstanding recents.
  $effect(() => {
    const key = vaultKey;
    requests++;
    recents = [];
    selectedId = null;
    if (key !== null) void loadRecents();
  });

  onMount(() => {
    // The panel is a long-lived hidden window: refresh when the vault changes and when it is shown.
    const clearOnHide = () => {
      selectedId = null;
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
    };
    // A shown locked panel focuses the password.
    const onFocus = () => {
      refresh();
      if (!unlocked) card?.focus();
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
    <div class="content">
      {#if recents.length === 0}
        <p class="empty">{t("Nothing here yet")}</p>
      {:else}
        <ul class="entries" aria-label={t("Recent items")}>
          {#each recents as item (item.id)}
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
