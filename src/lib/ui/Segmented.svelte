<script lang="ts" generics="T extends string | number">
  let {
    value,
    options,
    label,
    onchange,
  }: { value: T; options: { value: T; label: string }[]; label: string; onchange: (value: T) => void } = $props();

  let group: HTMLElement | undefined = $state();

  function move(delta: number, from: number) {
    const next = options[(from + delta + options.length) % options.length];
    if (!next) return;
    onchange(next.value);
    group?.querySelectorAll<HTMLElement>('[role="radio"]')[options.indexOf(next)]?.focus();
  }

  function onkeydown(event: KeyboardEvent, index: number) {
    if (event.key === "ArrowRight" || event.key === "ArrowDown") move(1, index);
    else if (event.key === "ArrowLeft" || event.key === "ArrowUp") move(-1, index);
    else return;
    event.preventDefault();
  }
</script>

<div class="segmented" role="radiogroup" aria-label={label} bind:this={group}>
  {#each options as option, index (option.value)}
    <button
      type="button"
      role="radio"
      aria-checked={option.value === value}
      tabindex={option.value === value ? 0 : -1}
      onclick={() => onchange(option.value)}
      onkeydown={(e) => onkeydown(e, index)}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: inline-flex;
    padding: 2px;
    gap: 2px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--sidebar-bg);
  }
  button {
    padding: 2px 12px;
    border: 0;
    border-radius: 4px;
    background: transparent;
  }
  button[aria-checked="true"] {
    background: var(--control-bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }
</style>
