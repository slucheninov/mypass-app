<script lang="ts">
  import UnlockDeviceButton from "./UnlockDeviceButton.svelte";
  let { available }: { available: boolean } = $props();
</script>
<div data-testid="unlock-extras" data-available={available ? "true" : "false"}>
  {#if available}<UnlockDeviceButton />{/if}
</div>
