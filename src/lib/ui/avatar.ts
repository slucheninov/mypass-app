/** Ten fixed colours; white text on each stays readable. */
export const AVATAR_PALETTE = [
  "#d9534f",
  "#e07b39",
  "#b58a00",
  "#4f9d3a",
  "#1f9d8a",
  "#2f6feb",
  "#6a5acd",
  "#a54fb8",
  "#d6457f",
  "#6b7280",
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
