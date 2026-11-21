<script lang="ts">
  // The password step for a chosen vault file. Shared by "Open existing vault", "Open another
  // vault…", recents, the finder and the toolbar entry; the open guard has already run.
  import { errorMessage } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { session } from "../lib/stores/session.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import PasswordField from "../lib/ui/PasswordField.svelte";
  import { fileNameOf, folderOf } from "./vault-status";

  let { path, onclose }: { path: string; onclose: () => void } = $props();

  const uid = $props.id();
  const formId = `open-vault-${uid}`;
  let field: PasswordField | undefined = $state();
  let hasPassword = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  const fileName = $derived(fileNameOf(path));
  const folder = $derived(folderOf(path));

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy || !field) return;
    const password = field.take();
    if (password === "") return;
    busy = true;
    error = null;
    try {
      await session.open(path, password);
      onclose();
    } catch (e) {
      error = errorMessage(e);
      field?.focus();
    } finally {
      busy = false;
    }
  }
</script>

<Modal open={true} title={t("Open vault")} subtitle={fileName} {onclose}>
  {#snippet icon()}<span class="tile"><Icon name="lock" size={22} /></span>{/snippet}
  <form id={formId} onsubmit={submit} aria-busy={busy}>
    {#if folder}
      <!-- One line, cut at the end; the full path is in the tooltip. -->
      <span class="folder" title={folder}><Icon name="folder" size={13} /><span class="path">{folder}</span></span>
    {/if}
    <PasswordField
      bind:this={field}
      label={t("Master password")}
      toggle
      large
      oninput={(length) => {
        hasPassword = length > 0;
        error = null;
      }}
    />
    {#if error}
      <p class="error" role="alert"><Icon name="warning" size={14} /><span>{error}</span></p>
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={onclose}>{t("Cancel")}</Button>
    <Button type="submit" form={formId} variant="primary" disabled={!hasPassword || busy}>
      {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}{t("Open")}
    </Button>
  {/snippet}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    width: min(400px, calc(100vw - 96px));
  }
  .tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 44px;
    border-radius: 10px;
    background: var(--selected);
    color: var(--accent);
  }
  .folder {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    align-self: flex-start;
    max-width: 100%;
    padding: var(--space-1) var(--space-4);
    border-radius: 999px;
    background: var(--hover);
    color: var(--text-2);
    font-size: var(--font-size-sm);
  }
  .folder :global(svg) {
    flex: none;
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin: 0;
    color: var(--danger);
  }
  .error :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
