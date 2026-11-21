<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "../lib/i18n/index.svelte";

  let { onclick, children }: { onclick: () => void; children: Snippet } = $props();
</script>

<button type="button" class="value" title={t("Click to copy")} {onclick}>
  {@render children()}
</button>

<style>
  .value {
    max-width: 100%;
    padding: var(--space-1) var(--space-2);
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
    overflow-wrap: anywhere;
    cursor: pointer;
  }
  .value:hover {
    background: var(--hover);
  }
</style>
