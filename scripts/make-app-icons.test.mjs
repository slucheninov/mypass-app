import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { keyDistance, OUTPUTS, renderIcon } from "./make-app-icons.mjs";
import { decodePng } from "./make-locked-icons.mjs";

const committed = (file) => decodePng(readFileSync(new URL(`../src-tauri/icons/${file}`, import.meta.url)));

test("committed app icons match the generator pixel for pixel", () => {
  for (const { file, variant, size } of OUTPUTS) {
    const expected = renderIcon(variant, size);
    const actual = committed(file);
    assert.equal(actual.width, size, file);
    assert.equal(actual.height, size, file);
    assert.ok(Buffer.from(actual.rgba).equals(expected.rgba), `${file} differs from the generator`);
  }
});

test("the key has an eye, a solid shaft and nothing outside its outline", () => {
  assert.ok(keyDistance(50, 19.5) > 0, "the eye is cut out");
  assert.ok(keyDistance(50, 40) < 0, "the bow is solid");
  assert.ok(keyDistance(48.5, 70) < 0, "the shaft is solid");
  assert.ok(keyDistance(62, 60) < 0, "a tooth sticks out to the right");
  assert.ok(keyDistance(35, 70) > 0, "nothing left of the shaft");
});

test("the light and dark variants differ in tile and key colours", () => {
  const light = renderIcon("light", 64);
  const dark = renderIcon("dark", 64);
  const at = (img, x, y) => [...img.rgba.subarray((y * 64 + x) * 4, (y * 64 + x) * 4 + 4)];
  const lum = ([r, g, b]) => 0.2126 * r + 0.7152 * g + 0.0722 * b;
  // A tile pixel well away from the key, and one inside the shaft.
  const tile = [12, 32];
  const key = [31, 44];
  assert.ok(lum(at(light, ...tile)) > 200 && lum(at(dark, ...tile)) < 80, "tile is light / dark");
  assert.ok(lum(at(light, ...key)) < 80 && lum(at(dark, ...key)) > 180, "key is dark / light");
  assert.equal(at(light, 0, 0)[3], 0, "the corner outside the tile is transparent");
});
