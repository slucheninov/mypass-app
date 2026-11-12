<script lang="ts">
  import { onMount, untrack } from "svelte";
  import MenuList from "./MenuList.svelte";
  import { clampToViewport, type MenuItem } from "./menu";

  let {
    items,
    onselect,
    x,
    y,
    onclose,
    opener,
  }: {
    items: MenuItem[];
    onselect: (id: string) => void;
    x: number;
    y: number;
    onclose: () => void;
    /** Where focus goes back to when the menu closes by keyboard or by a choice; defaults to the
     *  element that had focus when the menu opened. */
    opener?: HTMLElement | null;
  } = $props();

  let holder: HTMLElement | undefined = $state();
  // Where the menu is drawn: the requested point, moved inside the viewport once it is measured.
  let left = $state<number | null>(null);
  let top = $state<number | null>(null);
  // Read while the component is being created: the list focuses its first item when it mounts,
  // which is before this component's own `onMount` runs.
  const returnTo: HTMLElement | null =
    untrack(() => opener) ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);

  onMount(() => {
    const rect = holder?.getBoundingClientRect();
    if (rect) {
      const at = clampToViewport(x, y, rect.width, rect.height, window.innerWidth, window.innerHeight);
      left = at.x;
      top = at.y;
    }
  });

  function outside(event: Event) {
    if (holder && event.target instanceof Node && !holder.contains(event.target)) onclose();
  }

  // Escape, Tab and a choice put focus back first, so a dialog opened by the choice remembers the
  // row (not the vanishing menu item) as its opener. A click elsewhere keeps the focus it chose.
  function restoreFocus() {
    returnTo?.focus();
  }
</script>

<svelte:window onpointerdown={outside} onblur={onclose} />

<div class="context" style="left: {left ?? x}px; top: {top ?? y}px" bind:this={holder}>
  <MenuList
    {items}
    onselect={(id) => {
      restoreFocus();
      onclose();
      onselect(id);
    }}
    onclose={() => {
      restoreFocus();
      onclose();
    }}
  />
</div>

<style>
  .context {
    position: fixed;
    z-index: 200;
  }
</style>
