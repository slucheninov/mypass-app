<script lang="ts">
  // The password step for a chosen vault file. Shared by "Open existing vault", "Open another
  // vault…", recents, the finder and the toolbar entry; the open guard has already run.
  import { errorMessage } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { session } from "../lib/stores/session.svelte";
  import Button from "../lib/ui/Button.svelte";
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

<Modal open={true} title={t("Open vault")} {onclose}>
  <form id={formId} onsubmit={submit}>
    <div class="file">
      <strong>{fileNameOf(path)}</strong>
      <span class="folder">{folderOf(path)}</span>
    </div>
    <PasswordField
      bind:this={field}
      label={t("Master password")}
      toggle
      oninput={(length) => {
        hasPassword = length > 0;
        error = null;
      }}
    />
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </form>
  {#snippet footer()}
    <Button onclick={onclose}>{t("Cancel")}</Button>
    <Button type="submit" form={formId} variant="primary" disabled={!hasPassword || busy}>{t("Open")}</Button>
  {/snippet}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .file {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-wrap: anywhere;
  }
  .folder {
    color: var(--text-2);
    font-size: 12px;
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
</style>
