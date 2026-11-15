<script lang="ts">
  import { update } from "../lib/stores/update.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import Button from "../lib/ui/Button.svelte";
</script>
<div data-testid="tray-update-row">
  {#if update.status.banner && settings.value?.updateCheckEnabled !== false}
    <Button variant="plain" onclick={() => void update.open()}>{t("MyPass %@ available…", [update.status.banner.version])}</Button>
  {/if}
</div>
