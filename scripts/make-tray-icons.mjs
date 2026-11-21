// Generates the tray (menu bar) icons from one vector key: a single upright key, the bow on top and
// the teeth on the right, the same silhouette as the app icon's key.
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

export const TEMPLATE_SIZE = 36;
export const WINDOWS_SIZE = 32;

const SAMPLES = 8;
// The key is drawn on an 18-unit grid (one unit is one point of the 18 pt template).
const GRID = 18;
const BOW = { x: 9, y: 5.2, outer: 4.2, inner: 1.7 };
const SHAFT = { left: 7.75, right: 10.25, top: 8.4, bottom: 16.8, corner: 0.7 };
const TEETH = [
  { left: 10, right: 12.3, top: 11.5, bottom: 13.1 },
  { left: 10, right: 13.1, top: 14.1, bottom: 15.9 },
];

const GREEN = [116, 179, 89];
const OUTLINE_GREEN = [44, 92, 30];
// The outline width in grid units (about one pixel at 32 x 32).
const OUTLINE = 0.6;

// Signed distances (negative inside), in grid units. Only `Math.sqrt` and plain arithmetic, so
// the output is the same on every machine.
function length(x, y) {
  return Math.sqrt(x * x + y * y);
}

function box(x, y, { left, right, top, bottom }, corner = 0) {
  const cx = (left + right) / 2;
  const cy = (top + bottom) / 2;
  const dx = Math.abs(x - cx) - ((right - left) / 2 - corner);
  const dy = Math.abs(y - cy) - ((bottom - top) / 2 - corner);
  return length(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - corner;
}

/** The signed distance from the point (in grid units) to the key's outline. */
export function keyDistance(x, y) {
  const fromCenter = length(x - BOW.x, y - BOW.y);
  // The bow is a ring: inside the outer circle and outside the hole.
  const bow = Math.max(fromCenter - BOW.outer, BOW.inner - fromCenter);
  let shape = Math.min(bow, box(x, y, SHAFT, SHAFT.corner));
  for (const tooth of TEETH) shape = Math.min(shape, box(x, y, tooth));
  return shape;
}

/** Counts, for one pixel, the samples inside the key and inside its outline band. */
function coverage(px, py, size, outline) {
  let fill = 0;
  let band = 0;
  for (let row = 0; row < SAMPLES; row++) {
    for (let col = 0; col < SAMPLES; col++) {
      const distance = keyDistance(
        ((px + (col + 0.5) / SAMPLES) / size) * GRID,
        ((py + (row + 0.5) / SAMPLES) / size) * GRID,
      );
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
