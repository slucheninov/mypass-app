// Generates the tray (menu bar) icons from one vector key: the tilted silhouette shared with the app icon.
//
// - `src-tauri/icons/tray-template.png`: the macOS template image, 36 x 36 pixels (18 pt at 2x;
//   the tray draws it 18 pt high). Black with the shape in the alpha channel only, so the menu bar
//   tints it for light, dark and highlighted states.
// - `src-tauri/icons/tray-windows.png`: the Windows notification-area icon, 32 x 32 pixels. A
//   taskbar can be light or dark and a template is not available there, so the key is the brand
//   green with a dark green outline that keeps it visible on both.
//
// Run `node scripts/make-tray-icons.mjs` after changing the shape; a Node test compares the
// committed pixels with this generator.
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { encodePng } from "./make-locked-icons.mjs";

import { KEY_BOUNDS, keyDistanceAt } from "./key-shape.mjs";

export const TEMPLATE_SIZE = 36;
export const WINDOWS_SIZE = 32;

const SAMPLES = 8;
// The key is drawn on an 18-unit grid (one unit is one point of the 18 pt template).
const GRID = 18;
const KEY = { cx: 7.55, cy: 4.65, scale: 15.4 / KEY_BOUNDS.height, simplified: true };

const GREEN = [116, 179, 89];
const OUTLINE_GREEN = [44, 92, 30];
// The outline width in grid units (about one pixel at 32 x 32).
const OUTLINE = 0.6;

/** Counts, for one pixel, the samples inside the key and inside its outline band. */
function coverage(px, py, size, outline) {
  let fill = 0;
  let band = 0;
  for (let row = 0; row < SAMPLES; row++) {
    for (let col = 0; col < SAMPLES; col++) {
      const distance = keyDistanceAt(
        ((px + (col + 0.5) / SAMPLES) / size) * GRID,
        ((py + (row + 0.5) / SAMPLES) / size) * GRID,
        KEY,
      ) * KEY.scale;
      if (distance <= -outline) fill++;
      else if (distance <= 0) band++;
    }
  }
  return { fill, band };
}

/** The macOS template: black, the key's coverage as alpha. */
export function renderTemplate(size = TEMPLATE_SIZE) {
  const rgba = Buffer.alloc(size * size * 4);
  const samples = SAMPLES * SAMPLES;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const { fill, band } = coverage(x, y, size, 0);
      rgba[(y * size + x) * 4 + 3] = Math.round(((fill + band) * 255) / samples);
    }
  }
  return { width: size, height: size, rgba };
}

/** The Windows icon: a green key with a dark green outline. */
export function renderWindows(size = WINDOWS_SIZE) {
  const rgba = Buffer.alloc(size * size * 4);
  const samples = SAMPLES * SAMPLES;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const { fill, band } = coverage(x, y, size, OUTLINE);
      const covered = fill + band;
      if (covered === 0) continue;
      const offset = (y * size + x) * 4;
      for (let channel = 0; channel < 3; channel++) {
        rgba[offset + channel] = Math.round(
          ((GREEN[channel] ?? 0) * fill + (OUTLINE_GREEN[channel] ?? 0) * band) / covered,
        );
      }
      rgba[offset + 3] = Math.round((covered * 255) / samples);
    }
  }
  return { width: size, height: size, rgba };
}

export const OUTPUTS = [
  { file: "tray-template.png", render: () => renderTemplate() },
  { file: "tray-windows.png", render: () => renderWindows() },
];

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  for (const { file, render } of OUTPUTS) {
    writeFileSync(new URL(`../src-tauri/icons/${file}`, import.meta.url), encodePng(render()));
  }
}
