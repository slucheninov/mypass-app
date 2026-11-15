<script lang="ts">
  import { device } from "../lib/stores/device.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import Button from "../lib/ui/Button.svelte";
  let { variant = "main" }: { variant?: "main" | "panel" } = $props();
  const status = $derived(device.status);
</script>
{#if status?.available && status.canOfferNow}
  <Button variant={variant === "main" ? "primary" : "plain"} disabled={device.pending} onclick={() => void device.unlock()}>
    {status.touchIdEnabled && status.macPasswordEnabled ? t("Unlock with Touch ID or Mac password") : status.touchIdEnabled ? t("Unlock with Touch ID") : t("Unlock with Mac password")}
  </Button>
{:else if status?.reason === "deviceWindowExpired" || status?.reason === "accountWindowExpired"}
  <p>{t("Enter your master password to use Touch ID again.")}</p>
{/if}
{#if device.error}<p role="alert">{device.error}</p>{/if}
