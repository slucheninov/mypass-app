<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  type Props = {
    variant?: "primary" | "secondary" | "danger" | "plain";
    disabled?: boolean;
    type?: "button" | "submit";
    onclick?: (event: MouseEvent) => void;
    children: Snippet;
  } & Omit<HTMLButtonAttributes, "onclick" | "children" | "type" | "disabled">;

  let { variant = "secondary", disabled = false, type = "button", onclick, children, ...rest }: Props = $props();
</script>

<button class="btn {variant}" {type} {disabled} {onclick} {...rest}>
  {@render children()}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 26px;
    padding: 3px 12px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    cursor: default;
  }
  .btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .btn:disabled {
    opacity: 0.45;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
  }
  .primary:hover:not(:disabled) {
    background: var(--accent);
    filter: brightness(1.08);
  }
  .danger {
    color: var(--danger);
  }
  .plain {
    border-color: transparent;
    background: transparent;
  }
</style>
