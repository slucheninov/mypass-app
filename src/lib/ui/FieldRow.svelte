<script lang="ts">
  import type { Snippet } from "svelte";

  let { label, children, actions }: { label: string; children: Snippet; actions?: Snippet } = $props();
</script>

<div class="field-row">
  <div class="label">{label}</div>
  <div class="value">{@render children()}</div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .field-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 0;
    border-bottom: var(--sep);
  }
  .label {
    flex: none;
    width: 110px;
    color: var(--text-2);
  }
  .value {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .actions {
    display: flex;
    gap: 2px;
  }
</style>
