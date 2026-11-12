<script lang="ts">
  // One global shortcut: a recorder button (focus it and press the combination), Clear, and the
  // inline conflict message. The recorder captures `keydown`, calls `preventDefault` so the combo
  // does not act in the window, and sends the canonical string; the key comes from the physical key.
  import { t } from "../../lib/i18n/index.svelte";
  import { platform } from "../../lib/platform.svelte";
  import { formatShortcut, recordShortcut, type ShortcutRefusal } from "../../lib/shortcuts";
  import Button from "../../lib/ui/Button.svelte";
  import Row from "./Row.svelte";

  let {
    label,
    value,
    conflict,
    onrecord,
    onclear,
  }: {
    label: string;
    value: string;
    conflict: boolean;
    onrecord: (shortcut: string) => void;
    onclear: () => void;
  } = $props();

  // Why the last press was refused; shown until the next accepted shortcut or Clear.
  let refused = $state<ShortcutRefusal | null>(null);

  function onkeydown(event: KeyboardEvent) {
    // Plain Tab keeps moving focus, so the recorder is never a keyboard trap.
    if (event.key === "Tab") return;
    const result = recordShortcut(event, platform.os);
    const shortcut = result !== null && "shortcut" in result ? result.shortcut : null;
    if (shortcut !== null || event.metaKey || event.ctrlKey || event.altKey) event.preventDefault();
    if (result === null) return;
    if ("refused" in result) {
      refused = result.refused;
      return;
    }
    refused = null;
    onrecord(result.shortcut);
  }
</script>

<div role="group" aria-label={label}>
  <Row {label}>
    <span class="controls">
      <button type="button" class="recorder" aria-label={label} title={t("Press shortcut…")} {onkeydown}>
        {value === "" ? t("Record shortcut") : formatShortcut(value, platform.os)}
      </button>
      <Button variant="plain" disabled={value === ""} onclick={() => { refused = null; onclear(); }}>{t("Clear")}</Button>
    </span>
    {#snippet hint()}
      <span class="refused" role="status" aria-live="polite">
        {#if refused === "reserved"}{t("That shortcut is reserved by the system")}{:else if refused === "modifiers"}{t("That shortcut needs Command, Ctrl or Alt with a key")}{/if}
      </span>
      {#if conflict}<span class="conflict" role="alert">{t("That shortcut is already in use")}</span>{/if}
    {/snippet}
  </Row>
</div>

<style>
  .controls {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .recorder {
    min-width: 96px;
    min-height: 26px;
    padding: 2px 10px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: inherit;
  }
  .recorder:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .refused {
    color: var(--danger);
  }
  .conflict {
    color: var(--danger);
  }
</style>
