<script lang="ts">
  import Shell from "../screens/Shell.svelte";
</script>

<Shell>
  {#snippet list()}<p>list pane</p>{/snippet}
  {#snippet detail()}<p>detail pane</p>{/snippet}
</Shell>
