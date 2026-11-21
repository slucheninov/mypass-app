<script lang="ts">
  // The locked vault card: the file name and folder, the file status and size, and the master
  // password form. Shared by the main window's unlock screen and the tray panel (`compact`), so both
  // unlock the same way. The password is an uncontrolled input read once on submit and emptied at
  // once; it is never kept in state.
  import { tick } from "svelte";
  import { errorMessage } from "../lib/api";
  import { formatBytes, formatDate } from "../lib/i18n/format";
  import { t } from "../lib/i18n/index.svelte";
  import { platform } from "../lib/platform.svelte";
  import { session } from "../lib/stores/session.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Icon from "../lib/ui/Icon.svelte";
  import PasswordField from "../lib/ui/PasswordField.svelte";
  import KeyboardLayoutBadge from "./KeyboardLayoutBadge.svelte";
  import UnlockExtras from "./UnlockExtras.svelte";
  import { connectedInfo, fileNameOf, folderOf, statusKind, statusLabel } from "./vault-status";

  let { compact = false }: { compact?: boolean } = $props();

  let field: PasswordField | undefined = $state();
  let hasPassword = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const status = $derived(session.status);
  const path = $derived(status.vaultPath ?? "");
  const kind = $derived(statusKind(status.fileStatus));
  const info = $derived(connectedInfo(status.fileStatus));
  // A file that is gone cannot be unlocked; an unreadable one is left to the backend to explain.
  const canUnlock = $derived(hasPassword && !busy && kind !== "missing");

  /** Empties a half-typed password and the last error (the tray panel calls it when it hides). */
  export function clear(): void {
    field?.take();
    hasPassword = false;
    error = null;
  }

  export function focus(): void {
    field?.focus();
  }

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
</script>

<section class="card" class:compact>
  <div class="vault">
    <Icon name="lock" size={compact ? 22 : 28} />
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
  <UnlockExtras available={platform.deviceUnlockAvailable} variant={compact ? "panel" : "main"} />
</section>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: min(360px, 100%);
    padding: 20px;
    border: var(--sep);
    border-radius: 10px;
    background: var(--list-bg);
    box-sizing: border-box;
  }
  .card.compact {
    width: 100%;
    gap: 10px;
    padding: 14px;
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
  .dot.downloading {
    background: #2f7fd9;
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
</style>
