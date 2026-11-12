<script lang="ts">
  // New vault: name, master password and its confirmation. The location is chosen last, with the
  // native save dialog, and goes through the open guard before `vault_create` is called.
  import { errorMessage } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { confirmOpen } from "../lib/open-guard";
  import { pickSavePath } from "../lib/open-flow";
  import { session } from "../lib/stores/session.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";
  import PasswordField from "../lib/ui/PasswordField.svelte";
  import { fileNameOf } from "./vault-status";

  let { onclose }: { onclose: () => void } = $props();

  const uid = $props.id();
  const formId = `create-vault-${uid}`;
  const nameId = `${formId}-name`;
  let name = $state("");
  let password: PasswordField | undefined = $state();
  let confirm: PasswordField | undefined = $state();
  let passwordLength = $state(0);
  let error = $state<string | null>(null);
  let busy = $state(false);

  // A plain-language hint only: a short password is allowed.
  const weak = $derived(passwordLength > 0 && passwordLength < 8);

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy || !password || !confirm) return;
    error = null;
    const first = password.peek();
    if (first === "") {
      error = t("Choose a password");
      return;
    }
    if (first !== confirm.peek()) {
      error = t("Passwords do not match");
      return;
    }
    busy = true;
    try {
      const path = await pickSavePath();
      if (path === null || !(await confirmOpen(path))) return;
      const secret = password.take();
      confirm.clear();
      await session.create(path, secret, name.trim() || fileNameOf(path).replace(/\.kdbx$/i, ""));
      onclose();
    } catch (e) {
      password.clear();
      confirm.clear();
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal open={true} title={t("Create vault")} {onclose}>
  <form id={formId} onsubmit={submit}>
    <div class="field">
      <label for={nameId}>{t("Vault name")}</label>
      <input id={nameId} bind:value={name} autocomplete="off" spellcheck="false" />
    </div>
    <PasswordField
      bind:this={password}
      label={t("Master password")}
      autocomplete="new-password"
      toggle
      oninput={(length) => {
        passwordLength = length;
        error = null;
      }}
    />
    {#if weak}<p class="hint" role="status">{t("That password is short. Longer is safer.")}</p>{/if}
    <PasswordField
      bind:this={confirm}
      label={t("Confirm password")}
      autocomplete="new-password"
      oninput={() => (error = null)}
    />
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </form>
  {#snippet footer()}
    <Button onclick={onclose}>{t("Cancel")}</Button>
    <Button type="submit" form={formId} variant="primary" disabled={busy}>{t("Create vault")}</Button>
  {/snippet}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  label {
    color: var(--text-2);
    font-size: 12px;
  }
  input {
    padding: 5px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--bg);
  }
  .hint {
    margin: 0;
    color: var(--text-2);
    font-size: 12px;
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
</style>
