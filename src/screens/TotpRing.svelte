<script lang="ts">
  // The countdown of a one-time code: an SVG ring plus the seconds in the middle. The circle has
  // `pathLength="1"`, so the dash offset is the plain fraction `remaining / period`.
  import { t } from "../lib/i18n/index.svelte";

  let { remaining, period }: { remaining: number; period: number } = $props();

  const fraction = $derived(period > 0 ? Math.min(1, Math.max(0, remaining / period)) : 0);
</script>

<svg
  class="ring"
  viewBox="0 0 36 36"
  width="32"
  height="32"
  role="img"
  aria-label={t("%lld s left", remaining)}
>
  <circle class="track" cx="18" cy="18" r="15" fill="none" stroke-width="3" />
  <circle
    class="progress"
    data-part="progress"
    cx="18"
    cy="18"
    r="15"
    fill="none"
    stroke-width="3"
    stroke-linecap="round"
    pathLength="1"
    stroke-dasharray="1"
    stroke-dashoffset={fraction}
    transform="rotate(-90 18 18)"
  />
  <text x="18" y="22" text-anchor="middle" class="seconds">{remaining}</text>
</svg>

<style>
  .track {
    stroke: var(--separator);
  }
  .progress {
    stroke: var(--accent);
  }
  .seconds {
    font-size: 11px;
    fill: var(--text);
    font-variant-numeric: tabular-nums;
  }
</style>
