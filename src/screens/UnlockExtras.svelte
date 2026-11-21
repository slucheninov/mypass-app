<script lang="ts">
  import UnlockDeviceButton from "./UnlockDeviceButton.svelte";
  let { available, variant = "main" }: { available: boolean; variant?: "main" | "panel" } = $props();
</script>
<div data-testid="unlock-extras" data-available={available ? "true" : "false"}>
  {#if available}<UnlockDeviceButton {variant} />{/if}
</div>
