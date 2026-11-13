import type { IconName } from "./icons";

/** A menu row. A `separator` row draws a rule and takes no focus (its `label` is unused). */
export type MenuItem = {
  id: string;
  label: string;
  icon?: IconName;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
};

const MARGIN = 4;

/** The top-left corner that keeps a `width` x `height` box inside the viewport (4 px margin). */
export function clampToViewport(
  x: number,
  y: number,
  width: number,
  height: number,
  viewportWidth: number,
  viewportHeight: number,
): { x: number; y: number } {
  return {
    x: Math.max(MARGIN, Math.min(x, viewportWidth - width - MARGIN)),
    y: Math.max(MARGIN, Math.min(y, viewportHeight - height - MARGIN)),
  };
}

/**
 * Svelte action for an absolutely positioned popup anchored at `left: 0` of its parent: shifts it
 * left when it would run past the right edge of the window (a menu near the toolbar's end).
 */
export function fitInViewport(node: HTMLElement) {
  const rect = node.getBoundingClientRect();
  const over = rect.right - (window.innerWidth - MARGIN);
  if (over > 0) node.style.left = `${-Math.min(over, Math.max(0, rect.left - MARGIN))}px`;
}
