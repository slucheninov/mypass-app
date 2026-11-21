import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { composeLocked, decodePng, encodePng, SIZES } from "./make-locked-icons.mjs";

const icon = (size, locked = false) => readFileSync(new URL(
  `../extension/icons/icon${locked ? "-locked" : ""}${size}.png`, import.meta.url,
));

test("decode and encode round-trip every source icon", () => {
  assert.deepEqual(SIZES, [16, 32, 48, 128]);
  for (const size of SIZES) {
    const source = decodePng(icon(size));
    assert.equal(source.width, size);
    assert.equal(source.height, size);
    assert.deepEqual(decodePng(encodePng(source)), source);
  }
});

test("committed locked icons match the generator pixel for pixel", () => {
  for (const size of SIZES) {
    assert.deepEqual(decodePng(icon(size, true)), composeLocked(decodePng(icon(size))));
  }
});

test("the original key remains intact outside the lower-right badge", () => {
  for (const size of SIZES) {
    const source = decodePng(icon(size));
    const locked = composeLocked(source);
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        if (x >= size / 2 && y >= size / 2) continue;
        const offset = (y * size + x) * 4;
        assert.deepEqual(locked.rgba.slice(offset, offset + 4), source.rgba.slice(offset, offset + 4));
      }
    }
    const center = (Math.floor(size * 0.82) * size + Math.floor(size * 0.75)) * 4;
    assert.deepEqual(Array.from(locked.rgba.slice(center, center + 4)), [31, 41, 55, 255]);
  }
});

test("decodePng refuses interlaced and non-RGBA input", () => {
  for (const [offset, value] of [[25, 2], [28, 1]]) {
    const image = Buffer.from(icon(16));
    image[offset] = value;
    assert.throws(() => decodePng(image), /RGBA/);
  }
});
