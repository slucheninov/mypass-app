<script lang="ts">
  import { update } from "../lib/stores/update.svelte";
  import { settings } from "../lib/stores/settings.svelte";
  import { platform } from "../lib/platform.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import Button from "../lib/ui/Button.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
</script>
{#if update.status.banner && settings.public?.updateCheckEnabled !== false}
  <aside role="status" class="banner">
    <span>{t("MyPass %@ is available (you have %@)", [update.status.banner.version, platform.version ?? ""] )}</span>
    <Button onclick={() => void update.dismiss().catch(e => toasts.reportError(e))}>{t("Later")}</Button>
    <Button onclick={() => void update.open()}>{t("View release")}</Button>
  </aside>
{/if}
<style>.banner { display: flex; align-items: center; gap: 12px; padding: 10px 16px; background: var(--list-bg); border-bottom: var(--sep); }</style>
