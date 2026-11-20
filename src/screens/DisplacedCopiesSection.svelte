<script lang="ts">
  // The read-only list names retained files and identifies which folder to check. It
  // refreshes on mount, a vault change, a sync status change and a settled save.
  import { untrack } from "svelte";
  import { api } from "../lib/api";
  import type { DisplacedCopies } from "../lib/bindings";
  import { formatBytes } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { sync } from "../lib/stores/sync.svelte";
  import { settings } from "../lib/stores/settings.svelte";

  let found = $state<DisplacedCopies>({ count: 0, copies: [], unavailable: [] });
  let failed = $state(false);
  let sequence = 0;
  let loadedFor = "";

  async function load(forKey: string) {
    const mine = ++sequence;
    // A different vault, folder or lock state must never show the previous vault's file names, not even
    // while the refresh runs or after it fails: clear the list before fetching.
    if (forKey !== loadedFor) {
      found = { count: 0, copies: [], unavailable: [] };
      failed = false;
      loadedFor = forKey;
    }
    try {
      const next = await api.syncDisplacedStatus();
      if (mine !== sequence) return;
      found = next;
      failed = false;
    } catch {
      if (mine === sequence) {
        // A failed refresh never keeps stale names on screen.
        found = { count: 0, copies: [], unavailable: [] };
        failed = true;
      }
    }
  }

  // A save of the primary vault alone (no secondary folder, so no sync status change) can keep a
  // copy too: count the saves that settle (pending or saving, then idle or failed), the same
  // transition the secondary sync waits for, and refresh on each.
  let settledSaves = $state(0);
  let saveBusy = false;
  $effect(() => {
    const state = session.status.save.state;
    const busy = state === "pending" || state === "saving";
    if (saveBusy && !busy) untrack(() => (settledSaves += 1));
    saveBusy = busy;
  });

  const vaultKey = $derived(`${session.status.state}|${session.status.vaultPath ?? ""}|${settings.value?.secondaryFolder ?? ""}`);
  const key = $derived(
    `${vaultKey}|${sync.status.state}|${sync.status.state === "idle" ? (sync.status.lastSync ?? "") : ""}|${"recoveryFile" in sync.status ? (sync.status.recoveryFile ?? "") : ""}|${settledSaves}`,
  );
  $effect(() => {
    void key;
    void load(vaultKey);
  });

  const place = (location: "primary" | "secondary") => (location === "primary" ? t("Vault folder") : t("Sync folder"));
</script>

{#if found.count > 0 || found.unavailable.length > 0 || failed}
  <section data-testid="displaced-copies">
    <h2>{t("Retained vault copies")}</h2>
    {#if failed}
      <p role="alert" data-testid="displaced-unavailable">{t("Retained copies could not be checked.")}</p>
    {/if}
    {#if found.count > 0}
      <p role="status">{t("Retained copies: %@", [String(found.count)])}</p>
      <p>{t("Review these copies in the folder shown. MyPass never deletes competing copies automatically.")}</p>
      <ul>
        {#each found.copies as copy (`${copy.location}/${copy.name}`)}
          <li>
            <span class="where">{place(copy.location)}</span>
            <span>{copy.kind === "displaced" ? t("Competing copy") : t("Replacement working copy")}</span>
            <span class="name">{copy.name}</span>
            {#if copy.bytes !== null}<span class="size">{formatBytes(copy.bytes)}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if found.unavailable.length > 0}
      <p role="alert" data-testid="displaced-unavailable">{t("Retained copies could not be checked in: %@", [found.unavailable.map(place).join(", ")])}</p>
    {/if}
  </section>
{/if}

<style>
  h2 {
    margin: 18px 0 4px;
    font-size: 14px;
    font-weight: 600;
  }
  ul {
    margin: 8px 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    padding: 4px 0;
    border-bottom: var(--sep);
  }
  .name {
    flex: 1 1 16em;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .size {
    color: var(--text-2);
  }
</style>
