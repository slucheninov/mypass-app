<script lang="ts">
  import { update } from "../lib/stores/update.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import TrayMenuRow from "./TrayMenuRow.svelte";

  let { active, onhover }: { active: boolean; onhover: () => void } = $props();
</script>
<div data-testid="tray-update-row">
  {#if update.status.banner && settings.value?.updateCheckEnabled !== false}
    <TrayMenuRow key="update" {active} {onhover} label={t("MyPass %@ available…", [update.status.banner.version])} onclick={() => void update.open()} />
  {/if}
</div>
