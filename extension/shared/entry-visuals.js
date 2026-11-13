// letterFromTitle and colorForTitle decide an entry's letter and colour. If the popup and
// the inline panel each kept their own copy, the same entry could show a different colour
// in each surface — a mismatch a user notices and no test would catch. So this is the single
// source of truth for both, including the palette colorForTitle hashes into (moving with it
// is required: colorForTitle cannot function without it, and the panel does not load popup.js
// to borrow it from there).
//
// Everything else about how a row is drawn stays per-surface, because the popup styles
// through popup.css classes while the panel styles inside its own closed shadow root.
//
// This file runs in three worlds: as a classic script in the content-script world (loaded
// before content.js per manifest.json), as a classic script in the popup (loaded before
// popup.js), and under `require` in node tests — hence the module.exports guard at the
// bottom, matching the idiom the rest of this codebase already uses (content.js, popup.js).

const LETTER_AVATAR_PALETTE = [
  '#007AFF',
  '#34C759',
  '#FF9500',
  '#AF52DE',
  '#FF2D55',
  '#30B0C7',
  '#5856D6',
  '#00C7BE',
  '#32ADE6',
  '#A2845E'
];

function letterFromTitle(title) {
  const trimmed = title.trim();
  if (!trimmed) return '?';
  return trimmed[0].toUpperCase();
}

function colorForTitle(title) {
  const MASK64 = (1n << 64n) - 1n;
  const SIGNBIT = 1n << 63n;
  const wrap64 = (v) => {
    v &= MASK64;
    return v & SIGNBIT ? v - (1n << 64n) : v;
  };
  let hash = 0n;
  for (const byte of new TextEncoder().encode(title)) {
    hash = wrap64(wrap64(hash + BigInt(byte)) * 31n);
  }
  const len = BigInt(LETTER_AVATAR_PALETTE.length);
  let index = hash % len;
  if (index < 0n) index += len;
  return LETTER_AVATAR_PALETTE[Number(index)];
}

if (typeof module !== 'undefined') {
  module.exports = { LETTER_AVATAR_PALETTE, letterFromTitle, colorForTitle };
}
