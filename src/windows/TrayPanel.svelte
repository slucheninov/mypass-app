<script lang="ts">
  // The tray panel window (320 x 440). It shares the stores and `api` with the main window but
  // loads no entries list: locked it is a master password field, unlocked it lists the five most
  // recent items (titles only, no secret) and opens a compact detail. Showing and hiding the window
  // (blur included) is the backend's; this component only reacts to the session and to a hide.
  import { onMount } from "svelte";
  import { api, errorMessage } from "../lib/api";
  import { events, type EntrySummary } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { quietly } from "../lib/unlisten";
  import { onWindowHidden } from "../lib/secret.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import PasswordField from "../lib/ui/PasswordField.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import TrayDetail from "./TrayDetail.svelte";
  import TrayUpdateRow from "./TrayUpdateRow.svelte";

  const RECENT_LIMIT = 5;

  let recents = $state<EntrySummary[]>([]);
  let selectedId = $state<string | null>(null);
  let field: PasswordField | undefined = $state();
  let hasPassword = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let requests = 0;

  const unlocked = $derived(session.status.state === "unlocked");
  const noVault = $derived(session.status.state === "noVault");
  // One key per unlocked stretch and vault: a save-state change replaces `status` but not this.
  const vaultKey = $derived(unlocked ? (session.status.vaultPath ?? "") : null);

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

  // Everything of one unlocked stretch is dropped on a lock; the next unlock starts at the list.
  $effect(() => {
    if (vaultKey !== null) {
      void loadRecents();
    } else {
      requests++;
      recents = [];
      selectedId = null;
    }
  });

  onMount(() => {
    // The panel is a long-lived hidden window: refresh when the vault changes and when it is shown.
    const clearOnHide = () => {
      selectedId = null;
      // A half-typed master password does not wait in a hidden window.
      field?.take();
      hasPassword = false;
      error = null;
    };
    const offHidden = onWindowHidden(clearOnHide);
    // The backend hides the panel on blur, but whether the webview then reports `hidden` is not
    // guaranteed (WebView2), so a blur clears the secrets too; it costs nothing, the panel hides anyway.
    window.addEventListener("blur", clearOnHide);
    const refresh = () => {
      if (unlocked && document.visibilityState !== "hidden") void loadRecents();
    };
    document.addEventListener("visibilitychange", refresh);
    window.addEventListener("focus", refresh);
    let off: (() => void) | null = null;
    let gone = false;
    void events.vaultChanged
      .listen(() => refresh())
      .then((unlisten) => {
        if (gone) quietly(unlisten);
        else off = unlisten;
      })
      // Without the listener the recents refresh on focus and on unlock only.
      .catch(() => {});
    return () => {
      gone = true;
      if (off) quietly(off);
      offHidden();
      window.removeEventListener("blur", clearOnHide);
      document.removeEventListener("visibilitychange", refresh);
      window.removeEventListener("focus", refresh);
    };
  });

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!field || !hasPassword || busy) return;
    const password = field.take();
    if (password === "") return;
    busy = true;
    error = null;
    try {
      await session.unlock(password);
    } catch (e) {
      error = errorMessage(e);
      field?.focus();
    } finally {
      busy = false;
    }
  }

  function open(id: string) {
    selectedId = id;
    void toasts.guard(() => api.entryTouch(id));
  }
</script>

<main class="panel" data-testid="tray-panel">
  <div class="content">
    {#if noVault}
      <p class="empty">{t("No vault is open")}</p>
    {:else if !unlocked}
      <form class="locked" onsubmit={submit}>
        <PasswordField bind:this={field} label={t("Master password")} focusOnMount oninput={(n) => (hasPassword = n > 0)} />
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <Button type="submit" variant="primary" disabled={!hasPassword || busy}>{t("Unlock")}</Button>
      </form>
    {:else if selectedId !== null}
      {#key selectedId}
        <TrayDetail id={selectedId} onback={() => (selectedId = null)} />
      {/key}
    {:else}
      <h2 class="heading">{t("Recent items")}</h2>
      {#if recents.length === 0}
        <p class="empty">{t("Nothing here yet")}</p>
      {:else}
        <ul class="recents" aria-label={t("Recent items")}>
          {#each recents as item (item.id)}
            <li><button type="button" class="recent" onclick={() => open(item.id)}>{item.title}</button></li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
  <TrayUpdateRow />
  <footer>
    <Button onclick={() => void toasts.guard(() => api.appShowMain())}>{t("Open MyPass")}</Button>
    {#if unlocked}<Button onclick={() => void toasts.guard(() => session.lock())}>{t("Lock")}</Button>{/if}
    <Button onclick={() => void toasts.guard(() => api.appOpenSettings())}>{t("Settings")}</Button>
    <Button variant="plain" onclick={() => void toasts.guard(() => api.appQuit())}>{t("Quit MyPass")}</Button>
  </footer>
</main>
<ToastHost />

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100vh;
    padding: 12px;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .locked {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .heading {
    margin: 0 0 6px;
    font-size: 12px;
    color: var(--text-2);
    font-weight: 600;
  }
  .recents {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .recent {
    display: block;
    width: 100%;
    padding: 7px 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: inherit;
    text-align: left;
    font: inherit;
    cursor: default;
  }
  .recent:hover {
    background: var(--list-bg);
  }
  .empty {
    color: var(--text-2);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  footer {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-top: 8px;
    border-top: var(--sep);
  }
</style>
