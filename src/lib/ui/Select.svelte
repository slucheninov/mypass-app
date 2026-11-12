<script lang="ts" generics="T extends string | number">
  let {
    value,
    options,
    label,
    onchange,
  }: { value: T; options: { value: T; label: string }[]; label: string; onchange: (value: T) => void } = $props();

  function handle(event: Event) {
    const picked = options[(event.currentTarget as HTMLSelectElement).selectedIndex];
    if (picked) onchange(picked.value);
  }
</script>

<select class="select" aria-label={label} value={String(value)} onchange={handle}>
  {#each options as option (option.value)}
    <option value={String(option.value)}>{option.label}</option>
  {/each}
</select>

<style>
  .select {
    min-height: 26px;
    padding: 2px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
  }
</style>
