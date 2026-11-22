<script lang="ts">
  // The welcome screen offers recent vaults and create / open / find. The locked screen shows
  // the vault card with the master password field. The password is an uncontrolled input read
  // once on submit and emptied at once; it is never kept in state.
  import { onMount } from "svelte";
  import { events } from "../lib/bindings";
  import { device } from "../lib/stores/device.svelte";
  import { api } from "../lib/api";
  import type { RecentVaultDto } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { openVaultAtPath, openVaultFromDialog } from "../lib/open-flow";
  import { session } from "../lib/stores/session.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import CreateVault from "./CreateVault.svelte";
  import FindVaults from "./FindVaults.svelte";
  import UnlockCard from "./UnlockCard.svelte";
  import VaultLocation from "./VaultLocation.svelte";

  onMount(() => {
    let active = true;
    let off: (() => void) | undefined;
    const consume = async () => {
      if (await api.deviceUnlockConsumeExtensionRequest()) {
        await device.refresh();
        if (active) await device.unlock(true);
      }
    };
    void events.unlockRequestedEvent.listen(() => void consume().catch(() => {})).then(unlisten => {
      if (!active) return unlisten();
      off = unlisten;
      void consume().catch(() => {});
    }).catch(() => {});
    return () => { active = false; off?.(); };
  });

  let showCreate = $state(false);
  let showFind = $state(false);
  let recents = $state<RecentVaultDto[]>([]);

  const status = $derived(session.status);
  const path = $derived(status.vaultPath ?? "");
  const featured = $derived(status.state === "noVault" ? recents.find((r) => r.exists) : undefined);
  const others = $derived(recents.filter((r) => r.path !== (featured?.path ?? status.vaultPath)));

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

  // Reload when the screen changes, including a switch between locked vaults.
  const recentsScope = $derived(
    status.state === "locked" ? `locked:${path}` : status.state === "noVault" ? "noVault" : null,
  );
  $effect(() => {
    if (recentsScope === null) return;
    void loadRecents();
  });

  async function forget(recent: RecentVaultDto): Promise<void> {
    try {
      await api.vaultForget(recent.path);
    } catch {
      // The list below is refreshed either way.
    }
    await loadRecents();
  }
</script>

{#snippet recentList()}
  {#if others.length > 0}
    <section class="recents">
      <h2>{t("Recent vaults")}</h2>
      <ul aria-label={t("Recent vaults")}>
        {#each others as recent (recent.path)}
          <li>
            {#if recent.exists}
              <button type="button" class="recent" onclick={() => void openVaultAtPath(recent.path)}>
                <span class="name">{recent.fileName}</span>
                <span class="folder"><VaultLocation path={recent.path} /></span>
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
{/snippet}

<div class="screen" data-testid="unlock-screen">
  {#if status.state === "noVault"}
    <section class="hero">
      <Icon name="lock" size={40} />
      <h1>{t("MyPass")}</h1>
      {#if featured}
        {@const recent = featured}
        <button type="button" class="featured" onclick={() => void openVaultAtPath(recent.path)}>
          <span class="file-icon"><Icon name="note" size={22} /></span>
          <span class="names">
            <span class="name">{recent.fileName}</span>
            <span class="folder"><VaultLocation path={recent.path} /></span>
          </span>
          <span class="open">{t("Open")}</span>
        </button>
      {/if}
      {@render recentList()}
      <div class="actions">
        <Button variant={recents.length > 0 ? "secondary" : "primary"} onclick={() => (showCreate = true)}>{t("Create vault")}</Button>
        <Button onclick={() => void openVaultFromDialog()}>{t("Open existing vault")}</Button>
        <Button onclick={() => (showFind = true)}>{t("Find vault files")}</Button>
      </div>
    </section>
  {:else}
    <UnlockCard />
    {@render recentList()}
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
    gap: 16px;
    width: min(420px, 100%);
  }
  .hero h1 {
    margin: 0;
    font-size: 22px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-4);
  }
  .featured {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 12px;
    border: 1px solid var(--control-border);
    border-radius: 10px;
    background: var(--control-bg);
    text-align: left;
    cursor: pointer;
  }
  .featured:hover {
    background: var(--hover);
  }
  .file-icon {
    display: grid;
    place-items: center;
    flex: none;
    width: 40px;
    height: 40px;
    border-radius: var(--radius);
    background: var(--hover);
    color: var(--accent);
  }
  .featured .names {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .open {
    padding: var(--space-2) 12px;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }
  .recents {
    width: 100%;
    max-width: 360px;
  }
  .hero .recents {
    max-width: none;
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
    color: var(--text-2);
  }
  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .names {
    min-width: 0;
  }
  .folder {
    color: var(--text-2);
    font-size: 12px;
    overflow-wrap: anywhere;
  }
</style>
