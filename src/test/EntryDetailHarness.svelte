<script lang="ts">
  // Mounts EntryDetail on its own with the optional history callback (Task 10 wires the real one).
  import EntryDetail from "../screens/EntryDetail.svelte";

  let { id, onopenhistory }: { id: string; onopenhistory: (id: string) => void } = $props();
</script>

<EntryDetail {id} {onopenhistory} />
