// The old MyPass key, with the origin at the bow and +v toward the tip.
const BOW_RADIUS = 24;
const TIP = 70;
export const KEY_BOUNDS = { width: 2 * BOW_RADIUS, height: TIP + BOW_RADIUS };
// In canvas coordinates the positive tilt takes the downward shaft to the right.
export const TILT_DEG = 22;
const angle = TILT_DEG * Math.PI / 180;
const cosine = Math.cos(angle);
const sine = Math.sin(angle);

const length = (x, y) => Math.sqrt(x * x + y * y);

function box(u, v, left, right, top, bottom) {
  const x = Math.max(left - u, u - right);
  const y = Math.max(top - v, v - bottom);
  return length(Math.max(x, 0), Math.max(y, 0)) + Math.min(Math.max(x, y), 0);
}

/** Signed distance to the upright key in key units (negative inside). */
export function uprightKeyDistance(u, v, { simplified = false } = {}) {
  const shaft = box(u, v, -8, 8, 14, 62);
  const tip = Math.max(61 - v, (Math.abs(u) + v - TIP) / Math.sqrt(2));
  const reach = simplified ? 2 : 0;
  const teeth = Math.min(
    box(u, v, 7, 14 + reach, 29, 37),
    box(u, v, 7, 13 + reach, 43, 51),
    box(u, v, 7, 12 + reach, 57, 63),
  );
  const bow = length(u, v) - BOW_RADIUS;
  const hole = length(u, v + 7) - (simplified ? 8 : 6.5);
  return Math.max(Math.min(bow, shaft, teeth, tip), -hole);
}

/** Signed distance in key units after mapping the canvas point to upright key space. */
export function keyDistanceAt(x, y, { cx, cy, scale, simplified = false }) {
  const dx = x - cx;
  const dy = y - cy;
  return uprightKeyDistance(
    (dx * cosine - dy * sine) / scale,
    (dx * sine + dy * cosine) / scale,
    { simplified },
  );
}
