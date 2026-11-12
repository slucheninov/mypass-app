<script lang="ts">
  // The main window. Until the first session answer arrives nothing is drawn (no flash of the
  // unlock screen for an already open vault). The vault shell replaces the placeholder in Task 6.
  import { session } from "../lib/stores/session.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import OpenGuardHost from "../screens/OpenGuardHost.svelte";
  import OpenVaultSheet from "../screens/OpenVaultSheet.svelte";
  import Unlock from "../screens/Unlock.svelte";
</script>

<main data-testid="main-app">
  {#if session.loaded}
    {#if session.status.state === "unlocked"}
      <div data-testid="vault-shell"></div>
    {:else}
      <Unlock />
    {/if}
  {/if}
</main>
{#if session.pendingOpenPath !== null}
  <OpenVaultSheet path={session.pendingOpenPath} onclose={() => session.clearPendingOpen()} />
{/if}
<!-- After the sheets, so a warning is drawn above a dialog that is still open. -->
<OpenGuardHost />
<ToastHost />
