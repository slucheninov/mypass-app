<script lang="ts">
  // An uncontrolled password input: the value lives only in the DOM element and is read once, on
  // submit, through `take()` (which also empties the field). Nothing here is `$state`, so a secret
  // never reaches a store or a reactive expression; the parent hears only the length.
  import { t } from "../i18n/index.svelte";
  import IconButton from "./IconButton.svelte";

  let {
    label,
    autocomplete = "current-password",
    toggle = false,
    focusOnMount = false,
    large = false,
    oninput,
  }: {
    /** Visible label, already translated. */
    label: string;
    autocomplete?: "current-password" | "new-password";
    /** Adds the Show / Hide password button. */
    toggle?: boolean;
    focusOnMount?: boolean;
    /** A taller field, for a sheet whose one job is this password. */
    large?: boolean;
    /** Called with the current length (never the value) after every edit. */
    oninput?: (length: number) => void;
  } = $props();

  const id = $props.id();
  let input: HTMLInputElement | undefined = $state();
  let shown = $state(false);

  $effect(() => {
    if (focusOnMount) input?.focus();
  });

  export function focus(): void {
    input?.focus();
  }
  /** The current value, left in place (used for validation before anything is sent). */
  export function peek(): string {
    return input?.value ?? "";
  }
  /** Empties the field. */
  export function clear(): void {
    if (input) input.value = "";
    oninput?.(0);
  }
  /** The current value; the field is emptied at once. */
  export function take(): string {
    const value = peek();
    clear();
    // A reveal never survives an attempt: the next one is typed hidden again.
    shown = false;
    return value;
  }
</script>

<div class="field">
  <label for={id}>{label}</label>
  <!-- The bordered box holds the input and, inside it on the right, the reveal button. -->
  <div class="control" class:large class:with-toggle={toggle}>
    <input
      {id}
      bind:this={input}
      type={shown ? "text" : "password"}
      {autocomplete}
      autocapitalize="off"
      spellcheck="false"
      oninput={() => oninput?.(input?.value.length ?? 0)}
    />
    {#if toggle}
      <IconButton
        icon="eye"
        label={shown ? t("Hide password") : t("Show password")}
        onclick={() => (shown = !shown)}
      />
    {/if}
  </div>
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  label {
    color: var(--text-2);
    font-size: var(--font-size-sm);
  }
  .control {
    display: flex;
    align-items: center;
    min-height: var(--field-height);
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--bg);
  }
  .control.large {
    min-height: 36px;
  }
  .control.with-toggle {
    padding-right: 3px;
  }
  .control:focus-within {
    border-color: var(--accent);
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  input {
    flex: 1;
    align-self: stretch;
    min-width: 0;
    padding: var(--space-3) var(--space-5);
    border: 0;
    border-radius: var(--radius);
    outline: none;
    background: transparent;
  }
</style>
