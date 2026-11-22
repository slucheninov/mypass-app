<script lang="ts">
  import type { FullAutoFill } from "svelte/elements";

  // A labelled single-line input for the editors. The label wraps the input, so the accessible
  // name is the label; `error` is announced under the field and wired through aria-describedby.
  let {
    label,
    value = $bindable(),
    type = "text",
    placeholder,
    autocomplete = "off",
    inputmode,
    error = null,
    input = $bindable(),
  }: {
    label: string;
    value: string;
    type?: "text" | "password" | "date";
    placeholder?: string;
    autocomplete?: FullAutoFill;
    inputmode?: "numeric" | "text" | "tel" | "email";
    error?: string | null;
    input?: HTMLInputElement | undefined;
  } = $props();

  const uid = $props.id();
</script>

<div class="text-field">
  <label>
    <span class="label">{label}</span>
    <input
      bind:this={input}
      bind:value
      {type}
      {placeholder}
      {autocomplete}
      {inputmode}
      spellcheck="false"
      aria-invalid={error ? "true" : undefined}
      aria-describedby={error ? `${uid}-error` : undefined}
    />
  </label>
  {#if error}<p class="error" id="{uid}-error" role="alert">{error}</p>{/if}
</div>

<style>
  .text-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .label {
    color: var(--text-2);
    font-size: var(--font-size-sm);
  }
  input {
    min-height: var(--field-height);
    padding: var(--space-3) var(--space-5);
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
    font: inherit;
  }
  input[aria-invalid="true"] {
    border-color: var(--danger);
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--font-size-sm);
  }
</style>
