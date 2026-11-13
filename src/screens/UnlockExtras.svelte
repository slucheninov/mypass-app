<script lang="ts">
  // An empty slot in this stage: Stage 5 fills it with the device-unlock button. The props and the
  // test id are a contract.
  let { available }: { available: boolean } = $props();
</script>

<div data-testid="unlock-extras" data-available={available ? "true" : "false"}></div>
