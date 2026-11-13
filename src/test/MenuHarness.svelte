<script lang="ts">
  import Menu from "../lib/ui/Menu.svelte";
  let { onselect }: { onselect: (id: string) => void } = $props();
  const items = [
    { id: "copy", label: "Copy" },
    { id: "cut", label: "Cut", disabled: true },
    { id: "delete", label: "Delete", danger: true },
    { id: "rename", label: "Rename" },
  ];
</script>

<Menu {items} {onselect} label="Actions">
  {#snippet trigger()}<span>More</span>{/snippet}
</Menu>
