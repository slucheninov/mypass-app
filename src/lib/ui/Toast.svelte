<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import type { Toast } from "../stores/toasts.svelte";
  import Button from "./Button.svelte";

  let { toast, ondismiss }: { toast: Toast; ondismiss: (id: number) => void } = $props();
</script>

<div class="toast {toast.kind}" role={toast.kind === "error" ? "alert" : "status"}>
  <div class="body">
    <span class="text">{toast.text}</span>
    {#if toast.detail}
      <details>
        <summary>{t("Details")}</summary>
        <p class="detail">{toast.detail}</p>
      </details>
    {/if}
  </div>
  <Button variant="plain" onclick={() => ondismiss(toast.id)}>{t("Close")}</Button>
</div>

<style>
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    max-width: 360px;
    padding: 8px 8px 8px 12px;
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text);
    box-shadow: var(--shadow);
    border-left: 3px solid var(--accent);
  }
  .error {
    border-left-color: var(--danger);
  }
  .success {
    border-left-color: var(--text-2);
  }
  .body {
    flex: 1;
    min-width: 0;
    padding-top: 3px;
  }
  .detail {
    margin: 4px 0 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
    user-select: text;
  }
  summary {
    color: var(--text-2);
  }
</style>
