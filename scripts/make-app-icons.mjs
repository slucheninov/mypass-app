// Generates the MyPass app icon in a light and a dark variant from one key drawing.
//
// The green key uses the same signed distance geometry as the tray icons.
//
// Outputs (committed; a Node test compares them with this generator pixel for pixel):
// - `src-tauri/icons/source.png`, 1024 px light: the input of `pnpm tauri icon` (bundle icons).
// - `src-tauri/icons/source-dark.png`, 1024 px dark: the same tile for a dark appearance.
// - `src-tauri/icons/app-light.png`, `app-dark.png`, 512 px: embedded in the app, which shows the
//   variant selected by the App icon setting in the Dock (macOS) and on its windows (Windows).
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { keyDistanceAt } from "./key-shape.mjs";
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
    keyTop: [134, 196, 108],
    keyBottom: [92, 150, 68],
    keyShadow: 0.22,
  },
  dark: {
    tileTop: [60, 60, 64],
    tileBottom: [22, 22, 24],
    edge: [255, 255, 255, 0.12],
    keyTop: [134, 196, 108],
    keyBottom: [92, 150, 68],
    keyShadow: 0.5,
  },
};

// Layout on the 1024 grid of a macOS icon: an 824 px tile with 100 px margins.
const TILE_MIN = 100;
const TILE_MAX = 924;
const TILE_RADIUS = 185;
const TILE_SHADOW = { dy: 10, blur: 26, alpha: 0.3 };

// Place the bow above and left of centre on the tile, leaving room for the tilted shaft.
const KEY = { cx: 451, cy: 386, scale: 6 };
const KEY_SHADOW = { dy: 12, sigma: 10 };

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

/** Signed distance to the key in 1024-grid pixels. */
function keyPixels(x, y) {
  return keyDistanceAt(x, y, KEY) * KEY.scale;
}

function keyShadowMask(size, scale) {
  const sigma = KEY_SHADOW.sigma / scale;
  const radius = Math.ceil(3 * sigma);
  const weights = Array.from({ length: 2 * radius + 1 }, (_, index) =>
    Math.exp(-0.5 * ((index - radius) / sigma) ** 2));
  const sum = weights.reduce((total, weight) => total + weight, 0);
  const kernel = weights.map((weight) => weight / sum);
  const mask = new Float32Array(size * size);
  for (let row = 0; row < size; row++) {
    const y = (row + 0.5) * scale - KEY_SHADOW.dy;
    for (let col = 0; col < size; col++) {
      const x = (col + 0.5) * scale;
      mask[row * size + col] = clamp01(0.5 - keyPixels(x, y) / scale);
    }
  }

  // Blur the silhouette in two passes so corners and teeth cast the same soft shadow.
  const horizontal = new Float32Array(mask.length);
  const blurred = new Float32Array(mask.length);
  for (let row = 0; row < size; row++) {
    for (let col = 0; col < size; col++) {
      let alpha = 0;
      for (let x = Math.max(0, col - radius); x <= Math.min(size - 1, col + radius); x++) {
        alpha += mask[row * size + x] * kernel[x - col + radius];
      }
      horizontal[row * size + col] = alpha;
    }
  }
  for (let row = 0; row < size; row++) {
    for (let col = 0; col < size; col++) {
      let alpha = 0;
      for (let y = Math.max(0, row - radius); y <= Math.min(size - 1, row + radius); y++) {
        alpha += horizontal[y * size + col] * kernel[y - row + radius];
      }
      blurred[row * size + col] = alpha;
    }
  }
  return blurred;
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
  const keyShadow = keyShadowMask(size, scale);
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
        over(layer, [0, 0, 0], palette.keyShadow * keyShadow[row * size + col]);
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
