<script lang="ts">
  let { message }: { message: string | null } = $props();
</script>

{#if message}<p class="error" role="alert">{message}</p>{/if}

<style>
  .error {
    margin: 8px 0 0;
    color: var(--danger);
  }
</style>
