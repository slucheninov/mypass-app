<script lang="ts">
  import { t } from "../i18n/index.svelte";

  let {
    value,
    min,
    max,
    step,
    label,
    unit,
    onchange,
  }: {
    value: number;
    min: number;
    max: number;
    step: number;
    label: string;
    unit?: string;
    onchange: (value: number) => void;
  } = $props();

  let input: HTMLInputElement | undefined = $state();
  let shown = $state("");
  $effect(() => {
    shown = String(value);
  });

  /** Rounds to `min + k * step` and keeps the result inside `[min, max]` on that grid. */
  function normalise(v: number): number {
    const lastStep = Math.floor((max - min) / step);
    const k = Math.min(lastStep, Math.max(0, Math.round((v - min) / step)));
    return k * step + min;
  }

  function send(next: number) {
    shown = String(next);
    if (input) input.value = shown;
    if (next !== value) onchange(next);
  }

  function commit() {
    const parsed = Number((input?.value ?? "").trim());
    if (!Number.isFinite(parsed) || (input?.value ?? "").trim() === "") {
      shown = String(value);
      if (input) input.value = shown;
      return;
    }
    send(normalise(parsed));
  }

  function bump(delta: number) {
    send(normalise(value + delta));
  }

  function onkeydown(event: KeyboardEvent) {
    switch (event.key) {
      case "ArrowUp":
        bump(step);
        break;
      case "ArrowDown":
        bump(-step);
        break;
      case "PageUp":
        bump(step * 5);
        break;
      case "PageDown":
        bump(-step * 5);
        break;
      case "Home":
        send(normalise(min));
        break;
      case "End":
        send(normalise(max));
        break;
      case "Enter":
        commit();
        break;
      default:
        return;
    }
    event.preventDefault();
  }
</script>

<div class="stepper">
  <button type="button" aria-label={t("Decrease")} disabled={value <= min} onclick={() => bump(-step)}>{"\u2212"}</button>
  <input
    bind:this={input}
    role="spinbutton"
    type="text"
    inputmode="numeric"
    aria-label={label}
    aria-valuemin={min}
    aria-valuemax={max}
    aria-valuenow={value}
    value={shown}
    onchange={commit}
    {onkeydown}
  />
  {#if unit}<span class="unit">{unit}</span>{/if}
  <button type="button" aria-label={t("Increase")} disabled={value >= max} onclick={() => bump(step)}>+</button>
</div>

<style>
  .stepper {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  input {
    width: 52px;
    min-height: 26px;
    padding: 2px 6px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    text-align: right;
  }
  button {
    width: 26px;
    height: 26px;
    padding: 0;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
  }
  button:disabled {
    opacity: 0.4;
  }
  .unit {
    color: var(--text-2);
  }
</style>
