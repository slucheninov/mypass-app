<script lang="ts">
  import { ICON_PATHS, type IconName } from "./icons";

  let {
    name,
    size = 16,
    stroke = 1.5,
    filled = false,
  }: {
    name: IconName;
    size?: number;
    /** Stroke width in the 16-unit view box; large decorative icons pass a thinner one. */
    stroke?: number;
    /** Fills the outline with the current colour too (a set favorite star). */
    filled?: boolean;
  } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill={filled ? "currentColor" : "none"}
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  <path d={ICON_PATHS[name]} />
</svg>
