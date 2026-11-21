<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import type { MenuItem } from "./menu";

  let {
    items,
    onselect,
    onclose,
  }: { items: MenuItem[]; onselect: (id: string) => void; onclose: () => void } = $props();

  const uid = $props.id();
  const headingId = (id: string) => `menu-heading-${uid}-${id}`;
  const focusable = (item: MenuItem) => !item.separator && !item.heading && !item.disabled;
  const heading = $derived(items.find((i) => i.heading));
  // The left icon column is drawn only when some row has an icon, so text-only menus stay compact.
  const withIcons = $derived(items.some((i) => i.icon));

  let list: HTMLElement | undefined = $state();
  let active = $state("");
  let typed = "";
  let typedAt = 0;

  function enabled(): HTMLElement[] {
    return list
      ? Array.from(
          list.querySelectorAll<HTMLElement>(
            '[role="menuitem"]:not([aria-disabled="true"]), [role="menuitemradio"]:not([aria-disabled="true"])',
          ),
        )
      : [];
  }

  // Opens on the checked choice of a radio menu, otherwise on the first row.
  onMount(() => {
    const all = enabled();
    (all.find((el) => el.getAttribute("aria-checked") === "true") ?? all[0])?.focus();
  });

  function step(delta: number) {
    const all = enabled();
    if (all.length === 0) return;
    const at = all.findIndex((el) => el === document.activeElement);
    const next = at === -1 ? (delta > 0 ? 0 : all.length - 1) : (at + delta + all.length) % all.length;
    all[next]?.focus();
  }

  function onkeydown(event: KeyboardEvent) {
    const all = enabled();
    if (event.key === "ArrowDown") step(1);
    else if (event.key === "ArrowUp") step(-1);
    else if (event.key === "Home") all[0]?.focus();
    else if (event.key === "End") all[all.length - 1]?.focus();
    else if (event.key === "Escape") onclose();
    else if (event.key === "Tab") {
      onclose();
      return;
    } else if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey && event.key !== " ") {
      const now = Date.now();
      typed = now - typedAt > 700 ? event.key.toLowerCase() : typed + event.key.toLowerCase();
      typedAt = now;
      all.find((el) => (el.textContent ?? "").trim().toLowerCase().startsWith(typed))?.focus();
    } else return;
    event.preventDefault();
    event.stopPropagation();
  }

  function pick(item: MenuItem) {
    if (item.disabled) return;
    onselect(item.id);
  }
</script>

<div
  class="menu"
  role="menu"
  tabindex="-1"
  aria-labelledby={heading ? headingId(heading.id) : undefined}
  bind:this={list}
  {onkeydown}
>
  {#each items as item (item.id)}
    {#if item.separator}
      <div class="rule" role="separator"></div>
    {:else if item.heading}
      <div class="heading" role="presentation" id={headingId(item.id)}>{item.label}</div>
    {:else}
    <button
      type="button"
      role={item.checked === undefined ? "menuitem" : "menuitemradio"}
      aria-checked={item.checked === undefined ? undefined : item.checked}
      class:danger={item.danger}
      aria-disabled={item.disabled ? "true" : undefined}
      tabindex={active === item.id || (active === "" && item.id === items.find(focusable)?.id) ? 0 : -1}
      onfocus={() => (active = item.id)}
      onclick={() => pick(item)}
    >
      {#if withIcons}<span class="icon">{#if item.icon}<Icon name={item.icon} />{/if}</span>{/if}
      <span class="text">{item.label}</span>
      {#if item.checked !== undefined}<span class="check">{#if item.checked}<Icon name="check" size={14} />{/if}</span>{/if}
    </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    display: flex;
    flex-direction: column;
    min-width: 180px;
    padding: 4px;
    background: var(--bg);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }
  button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    text-align: left;
  }
  button:focus-visible,
  button:hover:not([aria-disabled="true"]) {
    background: var(--accent);
    color: var(--accent-text);
    outline: none;
  }
  button[aria-disabled="true"] {
    opacity: 0.4;
  }
  .rule {
    height: 1px;
    margin: 4px 8px;
    background: var(--separator);
  }
  .danger {
    color: var(--danger);
  }
  .heading {
    padding: var(--space-2) var(--space-4) var(--space-1);
    color: var(--text-2);
    font-size: var(--font-size-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .text {
    flex: 1;
  }
  .check {
    display: inline-flex;
    width: 14px;
    margin-left: var(--space-4);
  }
  .icon {
    display: inline-flex;
    width: 16px;
  }
</style>
