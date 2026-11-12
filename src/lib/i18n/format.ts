import { i18n } from "./index.svelte";

/** Formats a Unix-millisecond timestamp (the backend's unit) with `Intl` in the active UI language. */
export function formatDate(ms: number, style: "date" | "datetime" = "date"): string {
  const opts: Intl.DateTimeFormatOptions =
    style === "date"
      ? { dateStyle: "medium" }
      : { dateStyle: "medium", timeStyle: "short" };
  return new Intl.DateTimeFormat(i18n.lang, opts).format(new Date(ms));
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Binary (1024) sizes with at most one fraction digit, grouped per the active language. */
export function formatBytes(n: number): string {
  let value = Math.max(0, n);
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const num = new Intl.NumberFormat(i18n.lang, { maximumFractionDigits: unit === 0 ? 0 : 1 }).format(value);
  return `${num} ${UNITS[unit]}`;
}
