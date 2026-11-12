<script lang="ts">
  import GroupPicker from "../screens/GroupPicker.svelte";

  let props: {
    onpick: (id: string) => void;
    onclose: () => void;
    current?: string | null;
    exclude?: string | null;
    includeRoot?: boolean;
  } = $props();
</script>

<GroupPicker open={true} title="Restore to" {...props} />
