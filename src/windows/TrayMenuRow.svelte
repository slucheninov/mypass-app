<script lang="ts">
  // One row of the tray panel's menu, drawn like a native menu item: plain text, the same size on
  // every row, a full-width accent highlight under the pointer or the keyboard focus, and the
  // row's global shortcut (when one is set) on the right. The shortcut is decoration only: the
  // accessible name stays the label.
  let { label, shortcut = "", onclick }: { label: string; shortcut?: string; onclick: () => void } = $props();
</script>

<button type="button" class="row" {onclick}>
  <span class="label">{label}</span>
  {#if shortcut !== ""}<span class="shortcut" aria-hidden="true">{shortcut}</span>{/if}
</button>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    min-height: 26px;
    padding: 3px 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--font-size-md);
    text-align: left;
    cursor: default;
  }
  .row:hover,
  .row:focus-visible {
    background: var(--accent);
    color: var(--accent-text);
    outline: none;
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .shortcut {
    flex: none;
    color: var(--text-2);
    letter-spacing: 0.04em;
  }
  .row:hover .shortcut,
  .row:focus-visible .shortcut {
    color: inherit;
    opacity: 0.85;
  }
</style>
