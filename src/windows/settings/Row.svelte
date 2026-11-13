<script lang="ts">
  import type { Snippet } from "svelte";

  let { label, children, hint }: { label: string; children: Snippet; hint?: Snippet } = $props();
</script>

<div class="row">
  <div class="label">{label}</div>
  <div class="control">{@render children()}</div>
  {#if hint}<div class="hint">{@render hint()}</div>{/if}
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 16px;
    padding: 9px 0;
    border-bottom: var(--sep);
  }
  .hint {
    grid-column: 1 / -1;
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-2);
  }
</style>
