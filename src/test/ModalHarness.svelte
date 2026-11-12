<script lang="ts">
  import Modal from "../lib/ui/Modal.svelte";
  let { onclose }: { onclose?: () => void } = $props();
  let open = $state(false);
</script>

<button onclick={() => (open = true)}>opener</button>
<Modal {open} title="Dialog title" onclose={() => { open = false; onclose?.(); }}>
  <button>first</button>
  <button>second</button>
  {#snippet footer()}<button>last</button>{/snippet}
</Modal>
