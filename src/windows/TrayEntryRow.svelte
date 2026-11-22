<script lang="ts">
  import type { EntrySummary_Serialize } from "../lib/bindings";
  import TrayMenuRow from "./TrayMenuRow.svelte";

  let { entry, key, active, onopen, onhover }: {
    entry: EntrySummary_Serialize;
    key: string;
    active: boolean;
    onopen: (id: string) => void;
    onhover: () => void;
  } = $props();
</script>

<TrayMenuRow
  {key}
  label={entry.title}
  subtitle={entry.subtitle}
  avatar
  chevron
  {active}
  onclick={() => onopen(entry.id)}
  {onhover}
/>
