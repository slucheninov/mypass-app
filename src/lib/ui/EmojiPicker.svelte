<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import Button from "./Button.svelte";

  let {
    value,
    onchange,
  }: {
    /** The chosen emoji, or `null` for none (the default folder icon). */
    value: string | null;
    onchange: (value: string | null) => void;
  } = $props();

  /** A curated set for vault groups: 48, six rows of eight. */
  const EMOJI = [
    "💼", "🏠", "🏦", "💳", "🛒", "✈️", "🚗", "🎓",
    "🏥", "⚕️", "📱", "💻", "🖥️", "🌐", "📧", "☁️",
    "🔑", "🔒", "🛡️", "⭐", "❤️", "🎮", "🎬", "🎵",
    "📚", "📷", "🧾", "📦", "🛠️", "⚙️", "🧪", "🏢",
    "👪", "👤", "🐱", "🐶", "🌱", "🔥", "💡", "🎁",
    "🍔", "☕", "⚽", "🚲", "🏖️", "🌍", "📅", "📌",
  ];

  const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

  /** The first user-perceived character: a flag, a family or a skin-tone emoji stays whole. */
  function firstGrapheme(text: string): string {
    for (const part of segmenter.segment(text)) return part.segment;
    return "";
  }

  function typed(event: Event & { currentTarget: HTMLInputElement }) {
    const input = event.currentTarget;
    const one = firstGrapheme(input.value);
    // The state may not change (typing a second character), so the DOM is corrected by hand.
    input.value = one;
    onchange(one === "" ? null : one);
  }
</script>

<div class="picker">
  <div class="grid" role="group" aria-label={t("Emoji")}>
    {#each EMOJI as emoji (emoji)}
      <button type="button" class="cell" aria-pressed={value === emoji} onclick={() => onchange(emoji)}>{emoji}</button>
    {/each}
  </div>
  <div class="free">
    <input class="text" type="text" aria-label={t("Icon")} autocomplete="off" value={value ?? ""} oninput={typed} />
    <Button variant="plain" disabled={value === null} onclick={() => onchange(null)}>{t("Remove icon")}</Button>
  </div>
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 2px;
  }
  .cell {
    padding: 4px 0;
    border: 1px solid transparent;
    border-radius: var(--radius);
    background: transparent;
    font-size: 18px;
    line-height: 1.3;
  }
  .cell:hover {
    background: var(--hover);
  }
  .cell[aria-pressed="true"] {
    border-color: var(--accent);
    background: var(--selected);
  }
  .free {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .text {
    width: 64px;
    padding: 4px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius);
    background: var(--control-bg);
    color: var(--text);
    text-align: center;
    font-size: 16px;
  }
</style>
