<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    icon,
    label,
    onclick,
    pressed,
    disabled = false,
  }: {
    icon: IconName;
    /** Accessible name and tooltip, already translated. */
    label: string;
    onclick?: (event: MouseEvent) => void;
    pressed?: boolean;
    disabled?: boolean;
  } = $props();
</script>

<button class="icon-btn" type="button" aria-label={label} title={label} aria-pressed={pressed} {disabled} {onclick}>
  <Icon name={icon} />
</button>

<style>
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-2);
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .icon-btn[aria-pressed="true"] {
    background: var(--selected);
    color: var(--accent);
  }
  .icon-btn:disabled {
    opacity: 0.4;
  }
</style>
