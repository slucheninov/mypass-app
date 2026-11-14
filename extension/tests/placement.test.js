const assert = require('node:assert');
const placement = require('../shared/placement.js');

// --- placeIcon: viewport coordinates only, same gates as before ---
const field = { top: 100, left: 50, width: 200, height: 30 };
const icon = placement.placeIcon(field, [], null);
assert.deepStrictEqual(icon, { fits: true, size: 26, top: 102, left: 218 },
  'the 26 px button is centred vertically and sits 6 px inside the trailing edge');

const wide = { top: 100, left: 50, width: 300, height: 48 };
const reveal = { top: 108, left: 310, width: 32, height: 32 };
const clear = { top: 108, left: 276, width: 30, height: 32 };
assert.strictEqual(placement.placeIcon(wide, [reveal, clear], null).left, 244, 'clears both trailing controls');
assert.strictEqual(placement.placeIcon(wide, [clear, reveal], null).left, 244, 'DOM order does not matter');
assert.strictEqual(placement.placeIcon(wide, [{ top: 160, left: 300, width: 40, height: 32 }], null).left, 316,
  'a control on another row is ignored');
assert.strictEqual(placement.placeIcon({ top: 0, left: 0, width: 48, height: 32 }, [], null).fits, false,
  'a tiny field keeps its typing space');
assert.strictEqual(placement.placeIcon({ top: 0, left: 0, width: 300, height: 16 }, [], null).fits, false,
  'a short field never gets an oversized button');
assert.strictEqual(placement.placeIcon({ top: 0, left: 0, width: 300, height: 24 }, [], null).fits, true,
  'a 24 px field is the smallest that still gets a 20 px button');

// A slim <input> inside a tall visible field box: the pz.gov.pl login is a 22 px input inside a
// 48 px Angular Material box, so the input alone gives an 18 px icon and the icon never showed.
// Measured on the live page: input 59,317 516x22; field box 42,304 550x48 (overflow:hidden).
const slim = { top: 317, left: 59, width: 516, height: 22 };
const slimBox = { top: 304, left: 42, width: 550, height: 48 };
const slimClip = { left: 42, top: 304, right: 592, bottom: 352 };
assert.strictEqual(placement.placeIcon(slim, [], slimClip).fits, false, 'without a box the 22 px input is too short');
assert.deepStrictEqual(placement.placeIcon(slim, [], slimClip, slimBox), { fits: true, size: 28, top: 314, left: 558 },
  'a slim input is sized from the visible field box, inside the box clip');
assert.strictEqual(placement.placeIcon(slim, [{ top: 318, left: 561, width: 14, height: 14 }], slimClip, slimBox).left, 529,
  'a visible clear button still moves the icon left');
assert.strictEqual(placement.placeIcon(slim, [], slimClip, { ...slimBox, top: 317, height: 22 }).fits, false,
  'a box that is no taller than the input changes nothing');
assert.deepStrictEqual(placement.placeIcon(field, [], null, { top: 90, left: 40, width: 220, height: 60 }), icon,
  'an input that already fits ignores the box');

// The same box is only wanted for an input too short to carry an icon; content.js asks before
// it measures any ancestor.
assert.strictEqual(placement.needsBox(slim), true, 'a 22 px input needs the box');
assert.strictEqual(placement.needsBox({ ...slim, height: 24 }), false, 'a 24 px input already fits the smallest icon');
assert.strictEqual(placement.needsBox({ ...slim, height: 23 }), true);
// pz.gov.pl password field, typed: a 24 px reveal button and a 14 px clear button sit in the end
// 54 px of the container. The icon steps left of both and keeps a lot of the field for typing.
const slimPassword = { top: 460, left: 59, width: 462, height: 22 };
const passwordBox = { top: 447, left: 42, width: 550, height: 48 };
const passwordClip = { left: 42, top: 447, right: 592, bottom: 495 };
const revealButton = { top: 460, left: 527, width: 24, height: 25 };
const clearButton = { top: 465, left: 561, width: 14, height: 14 };
assert.deepStrictEqual(placement.placeIcon(slimPassword, [], passwordClip, passwordBox), { fits: true, size: 28, top: 457, left: 558 },
  'untyped: the icon sits at the end of the visible box');
assert.deepStrictEqual(placement.placeIcon(slimPassword, [revealButton, clearButton], passwordClip, passwordBox), { fits: true, size: 28, top: 457, left: 495 },
  'typed: the icon steps left of the reveal and clear buttons without covering either');

const clip = { left: 42, top: 71, right: 238, bottom: 169 };
const inClip = { left: 50, top: 100, width: 180, height: 48 };
assert.strictEqual(placement.placeIcon(inClip, [], clip).fits, true);
assert.strictEqual(placement.placeIcon({ ...inClip, top: 55 }, [], clip).fits, false, 'a partially clipped icon is hidden');
assert.strictEqual(placement.placeIcon({ ...inClip, left: 100 }, [], clip).fits, false, 'horizontal clipping counts too');

// --- placeFloating: below by default, flip above, clamp, scroll inside ---
const viewport = { innerWidth: 800, innerHeight: 600 };
const anchor = { top: 100, bottom: 134, left: 50, width: 200 };
const below = placement.placeFloating(anchor, { width: 240, height: 200 }, viewport);
assert.deepStrictEqual([below.top, below.left, below.flipped], [138, 50, false], 'opens below the field with a 4 px gap');

const lowAnchor = { top: 520, bottom: 554, left: 50, width: 200 };
const flipped = placement.placeFloating(lowAnchor, { width: 240, height: 200 }, viewport);
assert.strictEqual(flipped.flipped, true, 'flips above when there is not enough room below');
assert.strictEqual(flipped.top, 520 - 4 - 200, 'the flipped box ends 4 px above the field');

const tall = placement.placeFloating(anchor, { width: 240, height: 2000 }, viewport);
assert.ok(tall.maxHeight <= 600 - 134 - 4 - 4, 'a tall list is capped to the room that exists');
assert.ok(tall.top + tall.maxHeight <= 600, 'a capped box stays inside the viewport');

const rightEdge = placement.placeFloating({ top: 100, bottom: 134, left: 700, width: 90 }, { width: 240, height: 100 }, viewport);
assert.strictEqual(rightEdge.left, 800 - 240 - 4, 'clamped horizontally into the viewport');
const leftEdge = placement.placeFloating({ top: 100, bottom: 134, left: -50, width: 90 }, { width: 240, height: 100 }, viewport);
assert.strictEqual(leftEdge.left, 4, 'never starts left of the viewport');
const narrow = placement.placeFloating(anchor, { width: 240, height: 100 }, { innerWidth: 200, innerHeight: 600 });
assert.strictEqual(narrow.width, 192, 'wider than the viewport shrinks to fit');

// --- correction: containing-block mismatch measured once ---
assert.strictEqual(placement.correction({ left: 10, top: 20 }, { left: 10.5, top: 19.6 }, 1), null, 'within tolerance');
assert.deepStrictEqual(placement.correction({ left: 10, top: 20 }, { left: 18, top: 39 }, 1), { dx: -8, dy: -19 },
  'html/body transform offsets are measured, not enumerated');

// A scaled containing block: local pixels move the host by local * scale on screen, so the
// viewport gap is divided by the scale. Without it a 50 px gap at scale .5 closes by only 25 px.
assert.deepStrictEqual(placement.correction({ left: 100, top: 80 }, { left: 50, top: 30 }, 1, { x: 0.5, y: 0.5 }), { dx: 100, dy: 100 },
  'a 50 px viewport gap at scale 0.5 needs a 100 px local shift');
assert.deepStrictEqual(placement.correction({ left: 100, top: 80 }, { left: 50, top: 30 }, 1, { x: 2, y: 2 }), { dx: 25, dy: 25 },
  'a 50 px viewport gap at scale 2 needs a 25 px local shift');
assert.deepStrictEqual(placement.correction({ left: 100, top: 80 }, { left: 50, top: 30 }, 1, { x: 0, y: NaN }), { dx: 50, dy: 50 },
  'an unusable scale reads as 1 instead of dividing by zero');
assert.strictEqual(placement.correction({ left: 10, top: 20 }, { left: 10.4, top: 20.4 }, 1, { x: 0.5, y: 0.5 }), null,
  'tolerance applies to the viewport gap, whatever the scale');
assert.deepStrictEqual(placement.scaleFrom({ width: 14, height: 13 }, 28, 26), { x: 0.5, y: 0.5 }, 'rendered over layout size');
assert.deepStrictEqual(placement.scaleFrom({ width: 56, height: 52 }, 28, 26), { x: 2, y: 2 });
assert.deepStrictEqual(placement.scaleFrom({ width: 0, height: 0 }, 28, 26), { x: 1, y: 1 }, 'collapsed box reads as 1');
assert.deepStrictEqual(placement.scaleFrom({ width: 10, height: 10 }, 0, 0), { x: 1, y: 1 }, 'no layout size reads as 1');
assert.deepStrictEqual(placement.scaleFrom({ width: 1, height: 1 }, 1000, 1000), { x: 1, y: 1 }, 'an absurd ratio is ignored');
// offsetWidth/offsetHeight are rounded: a 74.73 px box reads 75, which is no scale at all and
// used to move an unscaled picker down by 0.9 px. Sizes within a pixel of each other read as 1.
assert.deepStrictEqual(placement.scaleFrom({ width: 320, height: 74.73 }, 320, 75), { x: 1, y: 1 }, 'rounding is not a scale');
assert.deepStrictEqual(placement.scaleFrom({ width: 160, height: 37.37 }, 320, 75), { x: 0.5, y: 37.37 / 75 }, 'a real scale survives');
assert.deepStrictEqual(placement.scaleFrom({ width: 14, height: 56 }, 28, 28), { x: 0.5, y: 2 }, 'axes are independent');
assert.strictEqual(placement.usableScale(0.5), 0.5);
assert.strictEqual(placement.usableScale(0), 1);

console.log('placement.test.js: placeIcon / placeFloating / correction OK');
