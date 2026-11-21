import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync, inflateSync } from "node:zlib";

export const SIZES = [16, 32, 48, 128];

const SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const RGBA = 4;

function paeth(left, above, diagonal) {
  const estimate = left + above - diagonal;
  const a = Math.abs(estimate - left);
  const b = Math.abs(estimate - above);
  const c = Math.abs(estimate - diagonal);
  if (a <= b && a <= c) return left;
  return b <= c ? above : diagonal;
}

const FILTERS = [
  () => 0,
  (left) => left,
  (_, above) => above,
  (left, above) => Math.floor((left + above) / 2),
  paeth,
];

export function decodePng(buffer) {
  if (!buffer.subarray(0, 8).equals(SIGNATURE) || buffer.toString("ascii", 12, 16) !== "IHDR") {
    throw new Error("Invalid PNG header");
  }
  if (buffer.length < 33 || buffer.readUInt32BE(8) !== 13
    || buffer[24] !== 8 || buffer[25] !== 6 || buffer[26] !== 0
    || buffer[27] !== 0 || buffer[28] !== 0) {
    throw new Error("Expected a non-interlaced 8-bit RGBA PNG");
  }
  const width = buffer.readUInt32BE(16);
  const height = buffer.readUInt32BE(20);
  if (width < 1 || height < 1 || width > 4096 || height > 4096) {
    throw new Error("Invalid PNG dimensions");
  }

  const data = [];
  let ended = false;
  for (let offset = 33; offset + 12 <= buffer.length;) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString("ascii", offset + 4, offset + 8);
    const dataEnd = offset + 8 + length;
    if (dataEnd + 4 > buffer.length) throw new Error("Truncated PNG chunk");
    if (type === "IDAT") data.push(buffer.subarray(offset + 8, dataEnd));
    if (type === "IEND") {
      ended = true;
      break;
    }
    offset = dataEnd + 4;
  }
  if (!ended || data.length === 0) throw new Error("Incomplete PNG");

  const stride = width * RGBA;
  const rawLength = (stride + 1) * height;
  const raw = inflateSync(Buffer.concat(data), { maxOutputLength: rawLength });
  if (raw.length !== rawLength) throw new Error("Invalid PNG scanline length");
  const rgba = new Uint8Array(stride * height);
  for (let y = 0; y < height; y++) {
    const scanline = y * (stride + 1);
    const filter = FILTERS[raw[scanline]];
    if (!filter) throw new Error("Unknown PNG filter");
    for (let x = 0; x < stride; x++) {
      const offset = y * stride + x;
      const left = x >= RGBA ? rgba[offset - RGBA] : 0;
      const above = y > 0 ? rgba[offset - stride] : 0;
      const diagonal = y > 0 && x >= RGBA ? rgba[offset - stride - RGBA] : 0;
      rgba[offset] = raw[scanline + 1 + x] + filter(left, above, diagonal);
    }
  }
  return { width, height, rgba };
}

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type, payload) {
  const chunk = Buffer.alloc(12 + payload.length);
  chunk.writeUInt32BE(payload.length);
  chunk.write(type, 4, "ascii");
  chunk.set(payload, 8);
  chunk.writeUInt32BE(crc32(chunk.subarray(4, -4)), chunk.length - 4);
  return chunk;
}

export function encodePng({ width, height, rgba }) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 6;
  const stride = width * RGBA;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    // Filter 0: four RGBA bytes per pixel, with no predictor.
    raw.set(rgba.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
  }
  return Buffer.concat([
    SIGNATURE, pngChunk("IHDR", header),
    pngChunk("IDAT", deflateSync(raw)), pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

const DARK = [31, 41, 55];
const WHITE = [255, 255, 255];
const OUTLINE = 0.046875;
const SAMPLES = 4;

// Signed distances to a rounded rectangle and the U-shaped shackle, in icon-width units.
function lockDistance(x, y) {
  const corner = 0.035;
  const dx = Math.abs(x - 0.75) - (0.185 - corner);
  const dy = Math.abs(y - 0.825) - (0.11 - corner);
  const body = Math.hypot(Math.max(0, dx), Math.max(0, dy)) + Math.min(Math.max(dx, dy), 0) - corner;
  const shackleX = Math.abs(x - 0.75);
  const shackle = y < 0.68
    ? Math.abs(Math.hypot(shackleX, y - 0.68) - 0.095)
    : Math.hypot(shackleX - 0.095, Math.max(0, y - 0.73));
  return Math.min(body, shackle - 0.028);
}

export function composeLocked({ width, height, rgba }) {
  const pixels = rgba.slice();
  const sampleCount = SAMPLES ** 2;
  for (let y = Math.floor(height / 2); y < height; y++) {
    for (let x = Math.floor(width / 2); x < width; x++) {
      let dark = 0;
      let white = 0;
      for (let row = 0; row < SAMPLES; row++) {
        for (let col = 0; col < SAMPLES; col++) {
          const distance = lockDistance(
            (x + (col + 0.5) / SAMPLES) / width,
            (y + (row + 0.5) / SAMPLES) / height,
          );
          if (distance <= 0) dark++;
          else if (distance <= OUTLINE) white++;
        }
      }
      if (dark + white === 0) continue;
      const offset = (y * width + x) * RGBA;
      const original = (sampleCount - dark - white) * rgba[offset + 3] / 255;
      const alpha = dark + white + original;
      for (let channel = 0; channel < 3; channel++) {
        pixels[offset + channel] = Math.round(
          (DARK[channel] * dark + WHITE[channel] * white + rgba[offset + channel] * original) / alpha,
        );
      }
      pixels[offset + 3] = Math.round(alpha * 255 / sampleCount);
    }
  }
  return { width, height, rgba: pixels };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  for (const size of SIZES) {
    const source = new URL(`../extension/icons/icon${size}.png`, import.meta.url);
    const output = new URL(`../extension/icons/icon-locked${size}.png`, import.meta.url);
    writeFileSync(output, encodePng(composeLocked(decodePng(readFileSync(source)))));
  }
}
