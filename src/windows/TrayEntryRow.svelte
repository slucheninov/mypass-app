<script lang="ts">
  // One entry of the tray panel's list: the letter avatar the main list uses, the title in bold,
  // the subtitle (user name) below it and a chevron. Only summary fields, never a secret. The
  // accessible name is the title; the subtitle is its description.
  import type { EntrySummary } from "../lib/bindings";
  import Avatar from "../lib/ui/Avatar.svelte";
  import Icon from "../lib/ui/Icon.svelte";

  let { entry, onopen }: { entry: EntrySummary; onopen: (id: string) => void } = $props();
  const subtitleId = $props.id();
</script>

<button
  type="button"
  class="entry"
  aria-label={entry.title}
  aria-describedby={entry.subtitle !== "" ? subtitleId : undefined}
  onclick={() => onopen(entry.id)}
>
  <Avatar text={entry.title} size={34} />
  <span class="text">
    <span class="title">{entry.title}</span>
    {#if entry.subtitle !== ""}<span class="subtitle" id={subtitleId}>{entry.subtitle}</span>{/if}
  </span>
  <span class="chevron"><Icon name="chevron" size={14} /></span>
</button>

<style>
  .entry {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 7px 10px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: default;
  }
  .entry:hover,
  .entry:focus-visible {
    background: var(--hover);
    outline: none;
  }
  .entry:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .title,
  .subtitle {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title {
    font-weight: 600;
    font-size: var(--font-size-md);
  }
  .subtitle {
    color: var(--text-2);
    font-size: var(--font-size-sm);
  }
  .chevron {
    flex: none;
    display: inline-flex;
    color: var(--text-2);
  }
</style>
