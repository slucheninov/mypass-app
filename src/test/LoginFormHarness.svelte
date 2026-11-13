<script lang="ts">
  // Mounts LoginForm alone with a generator slot that offers one fixed value ("abc"), the seam
  // Task 11 fills with the real Generator popover.
  import type { EntryEditModel_Serialize } from "../lib/bindings";
  import LoginForm from "../screens/editors/LoginForm.svelte";

  let { model: initial }: { model: EntryEditModel_Serialize } = $props();
  // The harness owns its own copy, like the editor owns the store's model.
  // svelte-ignore state_referenced_locally
  let model = $state(initial);
</script>

<LoginForm bind:model>
  {#snippet generatorSlot({ onuse })}
    <button type="button" onclick={() => onuse("abc")}>{"Use abc"}</button>
  {/snippet}
</LoginForm>
