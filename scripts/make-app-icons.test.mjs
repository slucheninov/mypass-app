import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { OUTPUTS, renderIcon } from "./make-app-icons.mjs";
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

test("both tiles carry the same green reference key", () => {
  const at = (img, x, y) => [...img.rgba.subarray((y * 64 + x) * 4, (y * 64 + x) * 4 + 4)];
  const lum = ([r, g, b]) => 0.2126 * r + 0.7152 * g + 0.0722 * b;
  const green = ([r, g, b]) => g > r + 30 && g > b + 30;
  for (const variant of ["light", "dark"]) {
    const img = renderIcon(variant, 64);
    const tile = lum(at(img, 12, 32));
    assert.ok(variant === "light" ? tile > 200 : tile < 80, "light or dark tile");
    assert.ok(green(at(img, 34, 36)), "green shaft");
    assert.ok(!green(at(img, 27, 21)), "the offset bow hole reveals the tile");
    assert.ok(green(at(img, 24, 25)), "the bow remains green around the hole");
    assert.equal(at(img, 0, 0)[3], 0, "the corner outside the tile is transparent");
  }
});

test("the key shadow fades past the tip without leaving a streak", () => {
  const { rgba, width } = renderIcon("light", 1024);
  const red = (x, y) => rgba[(y * width + x) * 4];
  assert.ok(red(608, 791) < red(500, 791), "a shadow remains near the tip");
  assert.equal(red(622, 814), red(500, 814), "the tile is clear beyond the shadow");
});
