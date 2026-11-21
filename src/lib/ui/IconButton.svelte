<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    icon,
    label,
    onclick,
    pressed,
    expanded,
    controls,
    keyshortcuts,
    tooltip,
    disabled = false,
    tone = "accent",
    iconSize = 16,
  }: {
    icon: IconName;
    /** Accessible name and tooltip, already translated. */
    label: string;
    onclick?: (event: MouseEvent) => void;
    pressed?: boolean;
    /** `aria-expanded`, for a button that shows and hides a region (`controls` names it). */
    expanded?: boolean;
    controls?: string;
    /** `aria-keyshortcuts`, in its own syntax (`Control+Shift+S`). */
    keyshortcuts?: string;
    /** Tooltip when it should say more than the label (a shortcut); defaults to the label. */
    tooltip?: string;
    disabled?: boolean;
    /** How a pressed button shows: `accent` tints it, `favorite` draws a filled amber icon. */
    tone?: "accent" | "favorite";
    /** Glyph size; a light glyph (the sidebar outline) takes a larger one to match its neighbours. */
    iconSize?: number;
  } = $props();
</script>

<button
  class="icon-btn"
  class:favorite={tone === "favorite"}
  type="button"
  aria-label={label}
  title={tooltip ?? label}
  aria-pressed={pressed}
  aria-expanded={expanded}
  aria-controls={controls}
  aria-keyshortcuts={keyshortcuts}
  {disabled}
  {onclick}
>
  <Icon name={icon} size={iconSize} filled={tone === "favorite" && pressed === true} />
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
  .icon-btn.favorite[aria-pressed="true"] {
    background: transparent;
    color: var(--favorite);
  }
  .icon-btn.favorite[aria-pressed="true"]:hover:not(:disabled) {
    background: var(--hover);
  }
  .icon-btn:disabled {
    opacity: 0.4;
  }
</style>
