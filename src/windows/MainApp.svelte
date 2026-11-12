<script lang="ts">
  // The main window. Until the first session answer arrives nothing is drawn (no flash of the
  // unlock screen for an already open vault).
  import { session } from "../lib/stores/session.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import OpenGuardHost from "../screens/OpenGuardHost.svelte";
  import OpenVaultSheet from "../screens/OpenVaultSheet.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import EntryDetail from "../screens/EntryDetail.svelte";
  import EntryList from "../screens/EntryList.svelte";
  import Shell from "../screens/Shell.svelte";
  import Unlock from "../screens/Unlock.svelte";
</script>

<main data-testid="main-app">
  {#if session.loaded}
    {#if session.status.state === "unlocked"}
      <!-- Keyed by vault: a switch while unlocked drops every piece of component-local state. -->
      {#key session.status.vaultPath}
        <Shell>
          {#snippet list()}<EntryList />{/snippet}
          {#snippet detail()}
            <!-- Keyed by id: every revealed value and dialog of a pane dies with its selection. -->
            {#if navigation.selectedId !== null}
              {#key navigation.selectedId}<EntryDetail id={navigation.selectedId} />{/key}
            {/if}
          {/snippet}
        </Shell>
      {/key}
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
