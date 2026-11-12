<script lang="ts">
  import MenuList from "./MenuList.svelte";
  import type { MenuItem } from "./menu";

  let {
    items,
    onselect,
    x,
    y,
    onclose,
  }: { items: MenuItem[]; onselect: (id: string) => void; x: number; y: number; onclose: () => void } = $props();

  let holder: HTMLElement | undefined = $state();

  function outside(event: Event) {
    if (holder && event.target instanceof Node && !holder.contains(event.target)) onclose();
  }
</script>

<svelte:window onpointerdown={outside} onblur={onclose} />

<div class="context" style="left: {x}px; top: {y}px" bind:this={holder}>
  <MenuList
    {items}
    onselect={(id) => {
      onclose();
      onselect(id);
    }}
    {onclose}
  />
</div>

<style>
  .context {
    position: fixed;
    z-index: 200;
  }
</style>
