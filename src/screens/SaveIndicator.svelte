<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { session } from "../lib/stores/session.svelte";
  import Icon from "../lib/ui/Icon.svelte";

  const state = $derived(session.status.save.state);
</script>

<!-- The live region stays in the DOM so a change is announced; idle shows nothing. -->
<span class="indicator" role="status" data-testid="save-indicator" data-state={state}>
  {#if state === "pending" || state === "saving"}
    <span class="pending">{t("Saving…")}</span>
  {:else if state === "failed"}
    <span class="failed" role="img" aria-label={t("Saving failed. MyPass will retry.")} title={t("Saving failed. MyPass will retry.")}>
      <Icon name="security" />
    </span>
  {/if}
</span>

<style>
  .indicator {
    display: inline-flex;
    align-items: center;
    min-width: 0;
    color: var(--text-2);
    font-size: 12px;
    white-space: nowrap;
  }
  .failed {
    display: inline-flex;
    color: var(--danger);
  }
</style>
