<script lang="ts">
  // Competing copies of the open vault that a save or sync kept next to it instead of
  // overwriting (`.displaced` files). Nothing deletes them but the button here, behind a
  // confirmation; the list is display-only (the backend finds the files again by itself, the webview
  // never names one). Shown only while there is at least one.
  import { api, errorMessage } from "../lib/api";
  import type { DisplacedCopies } from "../lib/bindings";
  import { formatBytes } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { sync } from "../lib/stores/sync.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";

  let found = $state<DisplacedCopies>({ count: 0, copies: [] });
  let confirming = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let sequence = 0;

  async function load() {
    const mine = ++sequence;
    try {
      const next = await api.syncDisplacedStatus();
      if (mine === sequence) found = next;
    } catch {
      // The list is a convenience: a failed read leaves the last known one.
    }
  }

  // The copies belong to the open vault and its sync folder; a sync result can have produced one.
  const key = $derived(
    `${session.status.state}|${session.status.vaultPath ?? ""}|${sync.status.state}|${
      sync.status.state === "idle" ? (sync.status.lastSync ?? "") : ""
    }`,
  );
  $effect(() => {
    void key;
    void load();
  });

  async function deleteAll() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      found = await api.syncDisplacedDeleteAll();
      confirming = false;
    } catch (e) {
      error = errorMessage(e);
      confirming = false;
      await load();
    } finally {
      busy = false;
    }
  }

  const place = (location: "primary" | "secondary") => (location === "primary" ? t("Vault folder") : t("Sync folder"));
</script>

{#if found.count > 0}
  <section data-testid="displaced-copies">
    <h2>{t("Competing copies of your vault")}</h2>
    <p role="status">{t("Competing copies kept: %@", [String(found.count)])}</p>
    <p>{t("Another app or device saved the vault at the same moment as MyPass. MyPass kept those versions next to your vault and never deletes them by itself. Check them, then delete them when you no longer need them.")}</p>
    <ul>
      {#each found.copies as copy (copy.path)}
        <li>
          <span class="where">{place(copy.location)}</span>
          <span class="path">{copy.path}</span>
          {#if copy.bytes !== null}<span class="size">{formatBytes(copy.bytes)}</span>{/if}
        </li>
      {/each}
    </ul>
    <Button variant="danger" disabled={busy} onclick={() => (confirming = true)}>{t("Delete competing copies…")}</Button>
    {#if error}<p role="alert">{error}</p>{/if}
  </section>
{/if}

<Modal open={confirming} title={t("Delete competing copies?")} onclose={() => (confirming = false)}>
  <p>{t("These files are deleted permanently and MyPass cannot restore them. Your vault itself is not touched.")}</p>
  {#snippet footer()}
    <Button onclick={() => (confirming = false)}>{t("Cancel")}</Button>
    <Button variant="danger" disabled={busy} onclick={() => void deleteAll()}>{t("Delete")}</Button>
  {/snippet}
</Modal>

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
  .path {
    flex: 1 1 16em;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .size {
    color: var(--text-2);
  }
</style>
