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
    type?: "text" | "password";
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
    gap: 3px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .label {
    color: var(--text-2);
    font-size: 12px;
  }
  input {
    padding: 5px 8px;
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
    font-size: 12px;
  }
</style>
