<script lang="ts">
  import { device } from "../lib/stores/device.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import Button from "../lib/ui/Button.svelte";
  let { variant = "main" }: { variant?: "main" | "panel" } = $props();
  const offer = $derived(device.offer);
</script>
{#if offer?.canOfferNow && offer.label && !device.blocked}
  <Button variant={variant === "main" ? "primary" : "plain"} disabled={device.pending} onclick={() => void device.unlock()}>
    {offer.label === "touchIdOrMacPassword" ? t("Unlock with Touch ID or Mac password") : offer.label === "touchId" ? t("Unlock with Touch ID") : t("Unlock with Mac password")}
  </Button>
{:else if offer?.label === "masterPasswordRequired"}
  <p>{t("Enter your master password to use Touch ID again.")}</p>
{/if}
{#if device.error}<p role="alert">{device.error}</p>{/if}
