<script lang="ts">
  // Competing copies of the open vault that a save or sync kept next to it instead of
  // overwriting (`.displaced` files). Read-only: MyPass never deletes them and this pane has no
  // way to; it names them (file names only, no folder) and says where to look. A folder that could
  // not be read, or a status that could not be fetched, is reported as such, never as "no copies".
  // Shown only while there is something to say. The list is fetched on mount and again whenever the
  // vault or the sync status changes (a sync result can have produced a copy). Known limitation: a
  // pane that stays open while another app or device keeps a copy shows the list as of the last
  // refresh; it refreshes when the pane opens, the vault changes or a sync finishes, not by polling.
  import { api } from "../lib/api";
  import type { DisplacedCopies } from "../lib/bindings";
  import { formatBytes } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { sync } from "../lib/stores/sync.svelte";

  let found = $state<DisplacedCopies>({ count: 0, copies: [], unavailable: [] });
  let failed = $state(false);
  let sequence = 0;
  let loadedFor = "";

  async function load(forKey: string) {
    const mine = ++sequence;
    // A different vault (or lock state) must never show the previous vault's file names, not even
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

  const vaultKey = $derived(`${session.status.state}|${session.status.vaultPath ?? ""}`);
  const key = $derived(
    `${vaultKey}|${sync.status.state}|${sync.status.state === "idle" ? (sync.status.lastSync ?? "") : ""}`,
  );
  $effect(() => {
    void key;
    void load(vaultKey);
  });

  const place = (location: "primary" | "secondary") => (location === "primary" ? t("Vault folder") : t("Sync folder"));
</script>

{#if found.count > 0 || found.unavailable.length > 0 || failed}
  <section data-testid="displaced-copies">
    <h2>{t("Competing copies of your vault")}</h2>
    {#if failed}
      <p role="alert" data-testid="displaced-unavailable">{t("Competing copies could not be checked.")}</p>
    {/if}
    {#if found.count > 0}
      <p role="status">{t("Competing copies kept: %@", [String(found.count)])}</p>
      <p>{t("Another app or device saved the vault at the same moment as MyPass. MyPass kept those versions next to your vault and never deletes them automatically. You can review them and remove them yourself in the vault folder.")}</p>
      <ul>
        {#each found.copies as copy (`${copy.location}/${copy.name}`)}
          <li>
            <span class="where">{place(copy.location)}</span>
            <span class="name">{copy.name}</span>
            {#if copy.bytes !== null}<span class="size">{formatBytes(copy.bytes)}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if found.unavailable.length > 0}
      <p role="alert" data-testid="displaced-unavailable">{t("Competing copies could not be checked in: %@", [found.unavailable.map(place).join(", ")])}</p>
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
