import { i18n, type Lang } from "./index.svelte";

const dateFormats = new Map<string, Intl.DateTimeFormat>();
const numberFormats = new Map<Lang, Intl.NumberFormat>();
const wholeFormats = new Map<Lang, Intl.NumberFormat>();

function dateFormat(lang: Lang, style: "date" | "datetime"): Intl.DateTimeFormat {
  const id = `${lang}:${style}`;
  let f = dateFormats.get(id);
  if (!f) {
    f = new Intl.DateTimeFormat(
      lang,
      style === "date" ? { dateStyle: "medium" } : { dateStyle: "medium", timeStyle: "short" },
    );
    dateFormats.set(id, f);
  }
  return f;
}

function numberFormat(lang: Lang, whole: boolean): Intl.NumberFormat {
  const cache = whole ? wholeFormats : numberFormats;
  let f = cache.get(lang);
  if (!f) {
    f = new Intl.NumberFormat(lang, { maximumFractionDigits: whole ? 0 : 1 });
    cache.set(lang, f);
  }
  return f;
}

/** Formats a Unix-millisecond timestamp (the backend's unit) with `Intl` in the active UI language. */
export function formatDate(ms: number, style: "date" | "datetime" = "date"): string {
  return dateFormat(i18n.lang, style).format(new Date(ms));
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
  return `${numberFormat(i18n.lang, unit === 0).format(value)} ${UNITS[unit]}`;
}
