<script lang="ts">
  // The screen of a session with no unlocked vault: `noVault` offers create / open / find, `locked`
  // is the vault card with the master password field. The password is an uncontrolled input read
  // once on submit and emptied at once; it is never kept in state.
  import { tick } from "svelte";
  import { api, errorMessage } from "../lib/api";
  import type { RecentVaultDto } from "../lib/bindings";
  import { formatBytes, formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { openVaultAtPath, openVaultFromDialog } from "../lib/open-flow";
  import { platform } from "../lib/platform.svelte";
  import { session } from "../lib/stores/session.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import PasswordField from "../lib/ui/PasswordField.svelte";
  import CreateVault from "./CreateVault.svelte";
  import FindVaults from "./FindVaults.svelte";
  import KeyboardLayoutBadge from "./KeyboardLayoutBadge.svelte";
  import UnlockExtras from "./UnlockExtras.svelte";
  import { connectedInfo, fileNameOf, folderOf, statusKind, statusLabel } from "./vault-status";

  let field: PasswordField | undefined = $state();
  let hasPassword = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let showCreate = $state(false);
  let showFind = $state(false);
  let recents = $state<RecentVaultDto[]>([]);

  const status = $derived(session.status);
  const path = $derived(status.vaultPath ?? "");
  const kind = $derived(statusKind(status.fileStatus));
  const info = $derived(connectedInfo(status.fileStatus));
  // A file that is gone cannot be unlocked; an unreadable one is left to the backend to explain.
  const canUnlock = $derived(hasPassword && !busy && kind !== "missing");
  const others = $derived(recents.filter((r) => r.path !== status.vaultPath));

  let recentsRequest = 0;

  async function loadRecents(): Promise<void> {
    const mine = ++recentsRequest;
    let list: RecentVaultDto[] = [];
    try {
      list = await api.vaultRecent();
    } catch {
      // An unreadable list is shown as empty.
    }
    // An older answer never overwrites a newer one.
    if (mine === recentsRequest) recents = list;
  }

  $effect(() => {
    if (session.status.state !== "locked") return;
    void session.status.vaultPath;
    void loadRecents();
  });

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!canUnlock || !field) return;
    const password = field.take();
    if (password === "") return;
    busy = true;
    error = null;
    try {
      await session.unlock(password);
    } catch (e) {
      error = errorMessage(e);
      await tick();
      field?.focus();
    } finally {
      busy = false;
    }
  }

  async function forget(recent: RecentVaultDto): Promise<void> {
    try {
      await api.vaultForget(recent.path);
    } catch {
      // The list below is refreshed either way.
    }
    await loadRecents();
  }
</script>

<div class="screen" data-testid="unlock-screen">
  {#if status.state === "noVault"}
    <section class="hero">
      <Icon name="lock" size={40} />
      <h1>{t("MyPass")}</h1>
      <div class="actions">
        <Button variant="primary" onclick={() => (showCreate = true)}>{t("Create vault")}</Button>
        <Button onclick={() => void openVaultFromDialog()}>{t("Open existing vault")}</Button>
        <Button onclick={() => (showFind = true)}>{t("Find vault files")}</Button>
      </div>
    </section>
  {:else}
    <section class="card">
      <div class="vault">
        <Icon name="lock" size={28} />
        <div class="names">
          <h1>{fileNameOf(path)}</h1>
          <div class="folder">{folderOf(path)}</div>
        </div>
      </div>
      {#if kind}
        <div class="state" data-testid="vault-status" data-status={kind}>
          <span class="dot {kind}" aria-hidden="true"></span>
          <span>{statusLabel(kind)}</span>
        </div>
      {/if}
      {#if info}
        <div class="meta">
          {#if info.size !== null}<span>{formatBytes(info.size)}</span>{/if}
          {#if info.modified}<span>{t("Modified %@", [formatDate(info.modified)])}</span>{/if}
        </div>
      {/if}
      <form onsubmit={submit}>
        <PasswordField
          bind:this={field}
          label={t("Master password")}
          toggle
          focusOnMount
          oninput={(length) => {
            hasPassword = length > 0;
            error = null;
          }}
        />
        <KeyboardLayoutBadge />
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <Button type="submit" variant="primary" disabled={!canUnlock}>{t("Unlock")}</Button>
      </form>
      <UnlockExtras available={platform.deviceUnlockAvailable} />
    </section>
    {#if others.length > 0}
      <section class="recents">
        <h2>{t("Recent vaults")}</h2>
        <ul aria-label={t("Recent vaults")}>
          {#each others as recent (recent.path)}
            <li>
              {#if recent.exists}
                <button type="button" class="recent" onclick={() => void openVaultAtPath(recent.path)}>
                  <span class="name">{recent.fileName}</span>
                  <span class="folder">{folderOf(recent.path)}</span>
                </button>
              {:else}
                <span class="recent missing">
                  <span class="name">{recent.fileName}</span>
                  <span class="folder">{t("File not found")}</span>
                </span>
              {/if}
              <Button variant="plain" onclick={() => void forget(recent)}>{t("Remove from list")}</Button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
    <Button variant="plain" onclick={() => void openVaultFromDialog()}>{t("Open another vault…")}</Button>
  {/if}
</div>

{#if showCreate}<CreateVault onclose={() => (showCreate = false)} />{/if}
{#if showFind}<FindVaults onclose={() => (showFind = false)} />{/if}

<style>
  .screen {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 20px;
    min-height: 100vh;
    padding: 32px 16px;
    box-sizing: border-box;
  }
  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }
  .hero h1 {
    margin: 0;
    font-size: 22px;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 220px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: min(360px, 100%);
    padding: 20px;
    border: var(--sep);
    border-radius: 10px;
    background: var(--list-bg);
  }
  .vault {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .names {
    min-width: 0;
  }
  h1 {
    margin: 0;
    font-size: 16px;
    overflow-wrap: anywhere;
  }
  .folder {
    color: var(--text-2);
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dot.connected {
    background: #2e9d57;
  }
  .dot.missing {
    background: #d98a00;
  }
  .dot.unreadable {
    background: var(--danger);
  }
  .meta {
    display: flex;
    gap: 12px;
    color: var(--text-2);
    font-size: 12px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .recents {
    width: min(360px, 100%);
  }
  h2 {
    margin: 0 0 6px;
    font-size: 12px;
    color: var(--text-2);
    font-weight: 600;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .recent {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
  }
  button.recent:hover {
    background: var(--hover);
  }
  .recent.missing {
    opacity: 0.6;
  }
  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
</style>
