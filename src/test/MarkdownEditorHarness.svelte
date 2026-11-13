<script lang="ts">
  // Mounts MarkdownEditor alone and reports its value, so a test can read what the textarea wrote.
  import MarkdownEditor from "../screens/editors/MarkdownEditor.svelte";

  let { initial = "", onvalue }: { initial?: string; onvalue?: (value: string) => void } = $props();
  // svelte-ignore state_referenced_locally
  let value = $state(initial);
  $effect(() => onvalue?.(value));
</script>

<MarkdownEditor bind:value label="Body" />
