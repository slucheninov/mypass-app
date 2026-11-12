<script lang="ts">
  // One global shortcut: a recorder button (focus it and press the combination), Clear, and the
  // inline conflict message. The recorder captures `keydown`, calls `preventDefault` so the combo
  // does not act in the window, and sends the canonical string; the key comes from the physical key.
  import { t } from "../../lib/i18n/index.svelte";
  import { platform } from "../../lib/platform.svelte";
  import { formatShortcut, shortcutFromEvent } from "../../lib/shortcuts";
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

  function onkeydown(event: KeyboardEvent) {
    // Plain Tab keeps moving focus, so the recorder is never a keyboard trap.
    if (event.key === "Tab") return;
    const shortcut = shortcutFromEvent(event, platform.os);
    if (shortcut !== null || event.metaKey || event.ctrlKey || event.altKey) event.preventDefault();
    if (shortcut !== null) onrecord(shortcut);
  }
</script>

<div role="group" aria-label={label}>
  <Row {label}>
    <span class="controls">
      <button type="button" class="recorder" aria-label={label} title={t("Press shortcut…")} {onkeydown}>
        {value === "" ? t("Record shortcut") : formatShortcut(value, platform.os)}
      </button>
      <Button variant="plain" disabled={value === ""} onclick={onclear}>{t("Clear")}</Button>
    </span>
    {#snippet hint()}
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
  .conflict {
    color: var(--danger);
  }
</style>
