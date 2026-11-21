import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { decodePng } from "./make-locked-icons.mjs";
import { OUTPUTS, renderTemplate, renderWindows, TEMPLATE_SIZE, WINDOWS_SIZE } from "./make-tray-icons.mjs";

const committed = (file) => decodePng(readFileSync(new URL(`../src-tauri/icons/${file}`, import.meta.url)));

/** The bounding box of the pixels with any coverage. */
function bounds({ width, height, rgba }) {
  let left = width;
  let right = -1;
  let top = height;
  let bottom = -1;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (rgba[(y * width + x) * 4 + 3] === 0) continue;
      left = Math.min(left, x);
      right = Math.max(right, x);
      top = Math.min(top, y);
      bottom = Math.max(bottom, y);
    }
  }
  return { width: right - left + 1, height: bottom - top + 1 };
}

test("committed tray icons match the generator pixel for pixel", () => {
  assert.deepEqual(OUTPUTS.map((o) => o.file), ["tray-template.png", "tray-windows.png"]);
  const plain = ({ width, height, rgba }) => ({ width, height, rgba: Buffer.from(rgba) });
  for (const { file, render } of OUTPUTS) assert.deepEqual(plain(committed(file)), plain(render()));
});

test("the macOS tray icon is an 18 pt @2x template: black, shape in alpha only", () => {
  const icon = renderTemplate();
  assert.equal(icon.width, TEMPLATE_SIZE);
  assert.equal(icon.height, 36);
  let opaque = 0;
  for (let i = 0; i < icon.rgba.length; i += 4) {
    assert.deepEqual([icon.rgba[i], icon.rgba[i + 1], icon.rgba[i + 2]], [0, 0, 0]);
    if (icon.rgba[i + 3] === 255) opaque++;
  }
  assert.ok(opaque > 100, "a solid key, not a faint outline");
});

test("both icons draw one upright key, taller than wide", () => {
  for (const icon of [renderTemplate(), renderWindows()]) {
    const box = bounds(icon);
    assert.ok(box.height > box.width * 1.5, `${box.width} x ${box.height}`);
    // The bow has a hole: the pixel at its centre is empty.
    const cx = Math.floor((9 / 18) * icon.width);
    const cy = Math.floor((5.2 / 18) * icon.height);
    assert.equal(icon.rgba[(cy * icon.width + cx) * 4 + 3], 0);
  }
});

test("the Windows tray icon is a coloured 32 px key", () => {
  const icon = renderWindows();
  assert.equal(icon.width, WINDOWS_SIZE);
  assert.equal(icon.height, WINDOWS_SIZE);
  let green = 0;
  for (let i = 0; i < icon.rgba.length; i += 4) {
    const [r, g, b, a] = [icon.rgba[i], icon.rgba[i + 1], icon.rgba[i + 2], icon.rgba[i + 3]];
    if (a === 255 && g > r && g > b) green++;
  }
  assert.ok(green > 50, "a green fill");
});
