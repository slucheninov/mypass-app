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

  let { variant = "secondary", disabled = false, type = "button", onclick, children, class: className = "", ...rest }: Props = $props();
</script>

<button {...rest} class="btn {variant} {className}" {type} {disabled} {onclick}>
  {@render children()}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    min-height: 28px;
    padding: var(--space-2) var(--space-6);
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
