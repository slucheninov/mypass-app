<script lang="ts">
  // The list names retained files and identifies which folder to check. It refreshes on mount,
  // a vault change, a sync status change and a settled save. A copy the backend marked deletable
  // (it carries an id, only while the vault is unlocked) can be deleted one at a time, after a
  // confirmation that names the file; the backend re-verifies that exact copy before deleting.
  import { untrack } from "svelte";
  import { api, ApiFailure } from "../lib/api";
  import type { DisplacedCopies, DisplacedCopyInfo } from "../lib/bindings";
  import { formatBytes } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { sync } from "../lib/stores/sync.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";

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

  // The copy whose deletion is being confirmed; the confirmation is bound to its id.
  let deleting = $state<DisplacedCopyInfo | null>(null);
  let busy = $state(false);

  // A vault switch, a lock or a refresh that no longer offers the copy closes the confirmation.
  $effect(() => {
    const pending = deleting;
    if (pending && !found.copies.some((copy) => copy.id && copy.id === pending.id)) {
      untrack(() => (deleting = null));
    }
  });

  async function remove() {
    const target = deleting;
    if (!target?.id || busy) return;
    busy = true;
    try {
      await api.syncDisplacedDelete(target.id);
    } catch (e) {
      toasts.push(
        "error",
        e instanceof ApiFailure && e.code === "retainedCopyRefused"
          ? t("The copy changed or could not be verified, so it was not deleted. The list was refreshed.")
          : t("Could not delete this copy. Try again."),
      );
    } finally {
      busy = false;
      deleting = null;
    }
    // A listing started after the delete settled: a refresh already in flight can never put
    // the deleted copy back on screen.
    await load(vaultKey);
  }
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
            {#if copy.id}
              <Button variant="danger" aria-label={`${t("Delete")}: ${copy.name}`} disabled={busy} onclick={() => (deleting = copy)}>
                {t("Delete")}
              </Button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if found.unavailable.length > 0}
      <p role="alert" data-testid="displaced-unavailable">{t("Retained copies could not be checked in: %@", [found.unavailable.map(place).join(", ")])}</p>
    {/if}
  </section>
{/if}

{#if deleting}
  <Modal open={true} title={t("Delete retained copy?")} onclose={() => (deleting = null)}>
    <p data-testid="displaced-delete-confirm">{t("Delete %@? MyPass checks that the file is unchanged before deleting it. This cannot be undone.", [deleting.name])}</p>
    {#snippet footer()}
      <Button onclick={() => (deleting = null)}>{t("Cancel")}</Button>
      <Button variant="danger" disabled={busy} onclick={() => void remove()}>{t("Delete")}</Button>
    {/snippet}
  </Modal>
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
