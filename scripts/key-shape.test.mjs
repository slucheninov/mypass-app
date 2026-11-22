import assert from "node:assert/strict";
import test from "node:test";
import { keyDistanceAt, uprightKeyDistance, TILT_DEG } from "./key-shape.mjs";

test("the upright key has a holed bow, three right teeth and a tip", () => {
  for (const simplified of [false, true]) {
    const distance = (u, v) => uprightKeyDistance(u, v, { simplified });
    assert.ok(distance(0, -7) > 0, "hole near the top");
    assert.ok(distance(0, 12) < 0, "solid bow");
    for (const v of [33, 47, 61]) assert.ok(distance(10, v) < 0, "right tooth");
    for (const v of [40, 54]) assert.ok(distance(10, v) > 0, "notch");
    assert.ok(distance(-10, 47) > 0, "straight left edge");
    assert.ok(distance(0, 69.9) < 0, "pointed tip");
  }
});

test("canvas geometry projects the offset hole and shaft at the shared tilt", () => {
  const angle = TILT_DEG * Math.PI / 180;
  const transform = { cx: 40, cy: 30, scale: 2 };
  const at = (v) => keyDistanceAt(
    transform.cx + Math.sin(angle) * v * transform.scale,
    transform.cy + Math.cos(angle) * v * transform.scale,
    transform,
  );
  assert.ok(at(-7) > 0);
  assert.ok(at(36) < 0);
});
