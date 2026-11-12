<script lang="ts">
  // Lists the `.kdbx` files the backend scanner finds; choosing one goes through the open guard and
  // on to the password step.
  import { api, errorMessage } from "../lib/api";
  import type { FoundVaultFileDto } from "../lib/bindings";
  import { formatBytes, formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { openVaultAtPath } from "../lib/open-flow";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import { fileNameOf, folderOf } from "./vault-status";

  let { onclose }: { onclose: () => void } = $props();

  /** `null` while searching. */
  let files = $state<FoundVaultFileDto[] | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const found = await api.vaultFindFiles();
        if (!cancelled) files = found;
      } catch (e) {
        if (cancelled) return;
        error = errorMessage(e);
        files = [];
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  async function choose(path: string): Promise<void> {
    if (await openVaultAtPath(path)) onclose();
  }
</script>

<Modal open={true} title={t("Find vault files")} {onclose}>
  {#if files === null}
    <p class="status" role="status"><span class="spinner" aria-hidden="true"></span>{t("Searching…")}</p>
  {:else if error}
    <p class="error" role="alert">{error}</p>
  {:else if files.length === 0}
    <p class="status">{t("No vault files found")}</p>
  {:else}
    <ul>
      {#each files as file (file.path)}
        <li>
          <button type="button" class="row" onclick={() => void choose(file.path)}>
            <span class="name">{fileNameOf(file.path)}</span>
            <span class="folder">{folderOf(file.path)}</span>
            <span class="meta">
              {#if file.size !== null}<span>{formatBytes(file.size)}</span>{/if}
              {#if file.modified !== null}<span>{formatDate(file.modified)}</span>{/if}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  {#snippet footer()}
    <Button onclick={onclose}>{t("Cancel")}</Button>
  {/snippet}
</Modal>

<style>
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 320px;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0 12px;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
  }
  .row:hover {
    background: var(--hover);
  }
  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .folder {
    grid-column: 1;
    color: var(--text-2);
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .meta {
    grid-column: 2;
    grid-row: 1 / span 2;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    color: var(--text-2);
    font-size: 12px;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    color: var(--text-2);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--separator);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
