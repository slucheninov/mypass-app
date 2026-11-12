<script lang="ts">
  // The main window. Until the first session answer arrives nothing is drawn (no flash of the
  // unlock screen for an already open vault).
  import { session } from "../lib/stores/session.svelte";
  import ToastHost from "../lib/ui/ToastHost.svelte";
  import OpenGuardHost from "../screens/OpenGuardHost.svelte";
  import OpenVaultSheet from "../screens/OpenVaultSheet.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import EntryDetail from "../screens/EntryDetail.svelte";
  import EntryEditor from "../screens/EntryEditor.svelte";
  import EntryList from "../screens/EntryList.svelte";
  import History from "../screens/History.svelte";
  import Shell from "../screens/Shell.svelte";
  import Unlock from "../screens/Unlock.svelte";

  // The history modal belongs to the selected entry of one unlocked stretch: another selection, a
  // lock, or an editor opening (a restore must never land under unsaved edits) closes it.
  // It also belongs to the vault it was opened in: the path is captured with the id.
  let historyId = $state<string | null>(null);
  let historyVault: string | null = null;
  $effect(() => {
    if (
      historyId !== null &&
      (session.status.state !== "unlocked" ||
        session.status.vaultPath !== historyVault ||
        navigation.selectedId !== historyId ||
        (navigation.presentation !== "browse" && navigation.editModel !== null))
    ) {
      historyId = null;
    }
  });
</script>

<main data-testid="main-app">
  {#if session.loaded}
    {#if session.status.state === "unlocked"}
      <!-- Keyed by vault: a switch while unlocked drops every piece of component-local state. -->
      {#key session.status.vaultPath}
        <Shell>
          {#snippet list()}<EntryList />{/snippet}
          {#snippet detail()}
            {#if navigation.presentation !== "browse" && navigation.editModel}
              <!-- One editor per model; selecting another row or scope leaves it open. -->
              {#key navigation.editModel}<EntryEditor bind:model={navigation.editModel} />{/key}
            {:else if navigation.selectedId !== null}
              <!-- Keyed by id: every revealed value and dialog of a pane dies with its selection. -->
              {#key navigation.selectedId}<EntryDetail id={navigation.selectedId} onopenhistory={(id) => {
                historyVault = session.status.vaultPath;
                historyId = id;
              }} />{/key}
            {/if}
          {/snippet}
        </Shell>
        {#if historyId !== null}
          <History entryId={historyId} open={true} onclose={() => (historyId = null)} />
        {/if}
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
