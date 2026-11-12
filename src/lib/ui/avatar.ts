/** Ten fixed colours; each keeps at least 4.5:1 contrast with white text (pinned by a test). */
export const AVATAR_PALETTE = [
  "#c0392b",
  "#b4531a",
  "#8a6700",
  "#2f7d1f",
  "#177a6b",
  "#2f6feb",
  "#5b4bc4",
  "#9b3fae",
  "#c2336f",
  "#5b6270",
] as const;

/** First character uppercased when it is a letter, `#` otherwise (also for empty text). */
export function avatarLetter(text: string): string {
  const first = Array.from(text.trim())[0] ?? "";
  return /\p{L}/u.test(first) ? first.toUpperCase() : "#";
}

/** A stable colour from an FNV-1a hash of the lower-cased text. */
export function avatarColor(text: string): string {
  let h = 0x811c9dc5;
  for (const ch of text.toLowerCase()) {
    h ^= ch.codePointAt(0) ?? 0;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return AVATAR_PALETTE[h % AVATAR_PALETTE.length] ?? AVATAR_PALETTE[0];
}
