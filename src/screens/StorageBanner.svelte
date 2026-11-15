<script lang="ts">
  import { sync } from "../lib/stores/sync.svelte";
  import { t } from "../lib/i18n/index.svelte";
</script>
{#if sync.storage.fallbackActive || sync.storage.cacheAhead}
  <p role="status" class="fallback">{t("iCloud Drive is not available. Working from a local copy; changes will be merged when it comes back.")}</p>
{/if}
{#if sync.storage.downloading}
  <p role="status">{t("Waiting for iCloud to download your vault…")}</p>
{/if}
<style>p { margin: 0; padding: 10px 16px; background: var(--list-bg); } .fallback { border-left: 4px solid #d98a00; }</style>
