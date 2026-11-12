<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "../i18n/index.svelte";

  let {
    widths,
    min,
    onresize,
    first,
    second,
    third,
  }: {
    widths: [number, number];
    min: [number, number, number];
    onresize: (widths: [number, number]) => void;
    first: Snippet;
    second: Snippet;
    third: Snippet;
  } = $props();

  let container: HTMLElement | undefined = $state();
  let drag: { index: 0 | 1; startX: number; startWidths: [number, number] } | null = null;
  const KEY_STEP = 16;

  /** The widths after moving handle `index` by `delta`, clamped so every pane keeps its minimum. */
  function clamp(index: 0 | 1, base: [number, number], delta: number): [number, number] {
    const total = container?.clientWidth ?? 0;
    const [a, b] = base;
    if (index === 0) {
      const room = total > 0 ? total - b - min[2] : Infinity;
      return [Math.max(min[0], Math.min(a + delta, Math.max(min[0], room))), b];
    }
    const room = total > 0 ? total - a - min[2] : Infinity;
    return [a, Math.max(min[1], Math.min(b + delta, Math.max(min[1], room)))];
  }

  function down(index: 0 | 1, event: PointerEvent) {
    drag = { index, startX: event.clientX, startWidths: [widths[0], widths[1]] };
    event.preventDefault();
  }

  function move(event: PointerEvent) {
    if (!drag) return;
    onresize(clamp(drag.index, drag.startWidths, event.clientX - drag.startX));
  }

  function up() {
    drag = null;
  }

  function key(index: 0 | 1, event: KeyboardEvent) {
    const delta = event.key === "ArrowLeft" ? -KEY_STEP : event.key === "ArrowRight" ? KEY_STEP : 0;
    if (delta === 0) return;
    event.preventDefault();
    onresize(clamp(index, [widths[0], widths[1]], delta));
  }
</script>

<svelte:window onpointermove={move} onpointerup={up} onpointercancel={up} />

<div class="split" bind:this={container}>
  <div class="pane" style="width: {widths[0]}px">{@render first()}</div>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="handle"
    role="separator"
    aria-orientation="vertical"
    aria-label={t("Resize sidebar")}
    aria-valuenow={widths[0]}
    aria-valuemin={min[0]}
    tabindex="0"
    onpointerdown={(e) => down(0, e)}
    onkeydown={(e) => key(0, e)}
  ></div>
  <div class="pane" style="width: {widths[1]}px">{@render second()}</div>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="handle"
    role="separator"
    aria-orientation="vertical"
    aria-label={t("Resize list")}
    aria-valuenow={widths[1]}
    aria-valuemin={min[1]}
    tabindex="0"
    onpointerdown={(e) => down(1, e)}
    onkeydown={(e) => key(1, e)}
  ></div>
  <div class="pane rest">{@render third()}</div>
</div>

<style>
  .split {
    display: flex;
    height: 100%;
    min-width: 0;
  }
  .pane {
    flex: none;
    min-width: 0;
    overflow: hidden;
  }
  .rest {
    flex: 1;
  }
  .handle {
    flex: none;
    width: 1px;
    position: relative;
    background: var(--separator);
    cursor: col-resize;
  }
  /* A wider invisible hit area around the 1 px separator. */
  .handle::after {
    content: "";
    position: absolute;
    inset: 0 -3px;
  }
</style>
