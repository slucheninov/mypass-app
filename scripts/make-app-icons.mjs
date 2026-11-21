// Generates the MyPass app icon in a light and a dark variant from one key drawing.
//
// The key is the shape of the old app's brand key (round bow with an eye near the top, a straight
// shaft with three teeth on the right and a pointed tip), drawn as signed distance fields so every
// size is antialiased without a rasterizer dependency. The same key-unit geometry is the outline
// SVG of the empty detail pane (`src/screens/DetailEmpty.svelte`).
//
// Outputs (committed; a Node test compares them with this generator pixel for pixel):
// - `src-tauri/icons/source.png`, 1024 px light: the input of `pnpm tauri icon` (bundle icons).
// - `src-tauri/icons/source-dark.png`, 1024 px dark: the same tile for a dark appearance.
// - `src-tauri/icons/app-light.png`, `app-dark.png`, 512 px: embedded in the app, which shows the
//   one that matches the effective theme in the Dock (macOS) and on its windows (Windows).
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { encodePng } from "./make-locked-icons.mjs";

/** Every committed output: file name, variant and pixel size. */
export const OUTPUTS = [
  { file: "source.png", variant: "light", size: 1024 },
  { file: "source-dark.png", variant: "dark", size: 1024 },
  { file: "app-light.png", variant: "light", size: 512 },
  { file: "app-dark.png", variant: "dark", size: 512 },
];

const PALETTES = {
  light: {
    tileTop: [255, 255, 255],
    tileBottom: [226, 226, 232],
    edge: [0, 0, 0, 0.07],
    keyTop: [64, 64, 70],
    keyBottom: [28, 28, 30],
    keyShadow: 0.22,
  },
  dark: {
    tileTop: [60, 60, 64],
    tileBottom: [22, 22, 24],
    edge: [255, 255, 255, 0.12],
    keyTop: [255, 255, 255],
    keyBottom: [208, 208, 216],
    keyShadow: 0.5,
  },
};

// Layout on the 1024 grid of a macOS icon: an 824 px tile with 100 px margins.
const TILE_MIN = 100;
const TILE_MAX = 924;
const TILE_RADIUS = 185;
const TILE_SHADOW = { dy: 10, blur: 26, alpha: 0.3 };

// The key, in key units (the outline SVG uses the same numbers), placed 6 px per unit around the
// tile centre. Bow: circle; eye: circle cut out; shaft, teeth and tip: one polygon, rounded.
const KEY_SCALE = 6;
const KEY_CENTER = [50, 49.25];
const BOW = { x: 50, y: 26, r: 24 };
const EYE = { x: 50, y: 19.5, r: 6.5 };
const ROUNDING = 1.2;
export const SHAFT = [
  [56, 40], [56, 54], [63, 60], [57, 65], [63, 70], [57, 75], [63, 80], [56, 86],
  [56, 89], [48.5, 96.5], [41, 89], [41, 40],
];
const KEY_SHADOW = { dy: 12, blur: 30 };

const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);
const length = (x, y) => Math.sqrt(x * x + y * y);

function smoothstep(edge0, edge1, x) {
  const t = clamp01((x - edge0) / (edge1 - edge0));
  return t * t * (3 - 2 * t);
}

function roundedBox(x, y) {
  const half = (TILE_MAX - TILE_MIN) / 2;
  const centre = (TILE_MAX + TILE_MIN) / 2;
  const dx = Math.abs(x - centre) - (half - TILE_RADIUS);
  const dy = Math.abs(y - centre) - (half - TILE_RADIUS);
  return length(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - TILE_RADIUS;
}

function polygon(px, py, points) {
  let best = Infinity;
  let inside = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const [ax, ay] = points[j];
    const [bx, by] = points[i];
    const ex = bx - ax;
    const ey = by - ay;
    const t = clamp01(((px - ax) * ex + (py - ay) * ey) / (ex * ex + ey * ey));
    const d = length(px - ax - ex * t, py - ay - ey * t);
    if (d < best) best = d;
    if ((ay > py) !== (by > py) && px < ax + ((py - ay) * ex) / ey) inside = !inside;
  }
  return inside ? -best : best;
}

/** Signed distance to the key in key units (negative inside). */
export function keyDistance(u, v) {
  const bow = length(u - BOW.x, v - BOW.y) - BOW.r;
  const shaft = polygon(u, v, SHAFT) - ROUNDING;
  const eye = length(u - EYE.x, v - EYE.y) - EYE.r;
  return Math.max(Math.min(bow, shaft), -eye);
}

/** Signed distance to the key in 1024-grid pixels. */
function keyPixels(x, y) {
  const u = (x - 512) / KEY_SCALE + KEY_CENTER[0];
  const v = (y - 512) / KEY_SCALE + KEY_CENTER[1];
  return keyDistance(u, v) * KEY_SCALE;
}

const mix = (a, b, t) => a + (b - a) * t;

/** Source-over of a straight-alpha colour onto a straight-alpha pixel. */
function over(dst, rgb, alpha) {
  if (alpha <= 0) return;
  const outA = alpha + dst[3] * (1 - alpha);
  for (let c = 0; c < 3; c++) {
    dst[c] = (rgb[c] * alpha + dst[c] * dst[3] * (1 - alpha)) / outA;
  }
  dst[3] = outA;
}

/** Renders one variant at `size` pixels, as straight RGBA. */
export function renderIcon(variant, size) {
  const palette = PALETTES[variant];
  if (!palette) throw new Error(`Unknown variant ${variant}`);
  const scale = 1024 / size;
  const rgba = Buffer.alloc(size * size * 4);
  const pixel = [0, 0, 0, 0];
  for (let row = 0; row < size; row++) {
    for (let col = 0; col < size; col++) {
      const x = (col + 0.5) * scale;
      const y = (row + 0.5) * scale;
      pixel[0] = 0;
      pixel[1] = 0;
      pixel[2] = 0;
      pixel[3] = 0;

      // Tile shadow, then the tile with its vertical gradient and a hairline inner edge.
      const shadow = roundedBox(x, y - TILE_SHADOW.dy);
      over(pixel, [0, 0, 0], TILE_SHADOW.alpha * (1 - smoothstep(-TILE_SHADOW.blur / 2, TILE_SHADOW.blur, shadow)));
      const tile = roundedBox(x, y);
      const tileCover = clamp01(0.5 - tile / scale);
      if (tileCover > 0) {
        const t = clamp01((y - TILE_MIN) / (TILE_MAX - TILE_MIN));
        const fill = palette.tileTop.map((top, c) => mix(top, palette.tileBottom[c], t));
        const layer = [fill[0], fill[1], fill[2], 1];
        const [er, eg, eb, ea] = palette.edge;
        over(layer, [er, eg, eb], ea * clamp01(1 - Math.abs(tile + 2 * scale) / (1.5 * scale)));
        // Key shadow and key, clipped to the tile.
        const keyShadow = keyPixels(x, y - KEY_SHADOW.dy);
        over(layer, [0, 0, 0], palette.keyShadow * (1 - smoothstep(-KEY_SHADOW.blur / 3, KEY_SHADOW.blur, keyShadow)));
        const key = keyPixels(x, y);
        const keyCover = clamp01(0.5 - key / scale);
        if (keyCover > 0) {
          const k = clamp01((y - 230) / 570);
          over(layer, palette.keyTop.map((top, c) => mix(top, palette.keyBottom[c], k)), keyCover);
        }
        over(pixel, layer, tileCover);
      }

      const offset = (row * size + col) * 4;
      for (let c = 0; c < 3; c++) rgba[offset + c] = Math.round(pixel[c]);
      rgba[offset + 3] = Math.round(pixel[3] * 255);
    }
  }
  return { width: size, height: size, rgba };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  for (const { file, variant, size } of OUTPUTS) {
    const output = new URL(`../src-tauri/icons/${file}`, import.meta.url);
    writeFileSync(output, encodePng(renderIcon(variant, size)));
  }
}
