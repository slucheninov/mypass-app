<script lang="ts">
  import { tick, type Snippet } from "svelte";
  import MenuList from "./MenuList.svelte";
  import type { MenuItem } from "./menu";

  let {
    items,
    onselect,
    trigger,
    label,
  }: {
    items: MenuItem[];
    onselect: (id: string) => void;
    trigger: Snippet;
    /** Accessible name of the trigger button, already translated. */
    label?: string;
  } = $props();

  let open = $state(false);
  let root: HTMLElement | undefined = $state();
  let button: HTMLButtonElement | undefined = $state();

  async function close(restoreFocus: boolean) {
    open = false;
    if (restoreFocus) {
      await tick();
      button?.focus();
    }
  }

  function outside(event: Event) {
    if (open && root && event.target instanceof Node && !root.contains(event.target)) void close(false);
  }
</script>

<svelte:window onpointerdown={outside} />

<span class="menu-root" bind:this={root}>
  <button
    type="button"
    class="trigger"
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label={label}
    bind:this={button}
    onclick={() => (open = !open)}
    onkeydown={(e) => {
      if (e.key === "ArrowDown" && !open) {
        e.preventDefault();
        open = true;
      }
    }}
  >
    {@render trigger()}
  </button>
  {#if open}
    <div class="popup">
      <MenuList
        {items}
        onselect={(id) => {
          void close(true);
          onselect(id);
        }}
        onclose={() => void close(true)}
      />
    </div>
  {/if}
</span>

<style>
  .menu-root {
    position: relative;
    display: inline-block;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: 26px;
    padding: 2px 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
  }
  .trigger:hover {
    background: var(--hover);
  }
  .popup {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 50;
    margin-top: 4px;
  }
</style>
