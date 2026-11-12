import { IntlMessageFormat } from "intl-messageformat";
import en from "./locales/en.json";
import ru from "./locales/ru.json";
import uk from "./locales/uk.json";

export type Lang = "en" | "uk" | "ru";

/** Reactive language state: any component whose render calls `t` re-renders when `lang` changes. */
export const i18n: { lang: Lang } = $state({ lang: "en" });

const original: Record<Lang, Record<string, string>> = { en, uk, ru };
let catalogs: Record<Lang, Record<string, string>> = { en, uk, ru };
const formatters = new Map<string, IntlMessageFormat | null>();
const warned = new Set<string>();

/** `uk*` -> uk, `ru*` -> ru, anything else (or an empty list) -> en; the first entry decides. */
export function resolveSystemLanguage(languages: readonly string[]): Lang {
  const first = (languages[0] ?? "").toLowerCase();
  if (first.startsWith("uk")) return "uk";
  if (first.startsWith("ru")) return "ru";
  return "en";
}

/**
 * Applies the settings language. `system` resolves from `navigator.languages`; the second argument
 * injects the list for tests. Also sets `<html lang>`.
 */
export function setLanguage(
  setting: "system" | Lang,
  languages: readonly string[] = typeof navigator === "undefined" ? [] : navigator.languages,
): void {
  const lang = setting === "system" ? resolveSystemLanguage(languages) : setting;
  i18n.lang = lang;
  document.documentElement.lang = lang;
}

function lookup(lang: Lang, key: string): string | undefined {
  const own = catalogs[lang];
  return Object.hasOwn(own, key) ? own[key] : undefined;
}

type Args = Record<string, unknown> | unknown[] | number;

// A number fills both `{0}` and `{count}` so plural and plain `%lld` keys both work; a record that
// carries only `count` also fills `{0}`; an array fills `{0}`, `{1}`, ...
function toValues(args: Args | undefined): Record<string, unknown> {
  if (args === undefined) return {};
  if (typeof args === "number") return { 0: args, count: args };
  if (Array.isArray(args)) return Object.fromEntries(args.map((v, i) => [String(i), v]));
  if ("count" in args && !("0" in args)) return { 0: args.count, ...args };
  return args;
}

function formatter(lang: Lang, key: string, template: string): IntlMessageFormat | null {
  const id = `${lang}\u0000${key}`;
  if (formatters.has(id)) return formatters.get(id) ?? null;
  let f: IntlMessageFormat | null = null;
  try {
    f = new IntlMessageFormat(template, lang);
  } catch {
    f = null;
  }
  formatters.set(id, f);
  return f;
}

/**
 * Translates an English source string: active catalog, then English, then the key itself. A
 * formatting error returns the raw template and never throws into the UI.
 */
export function t(key: string, args?: Args): string {
  const lang = i18n.lang;
  let used = lang;
  let template = lookup(lang, key);
  if (template === undefined && lang !== "en") {
    used = "en";
    template = lookup("en", key);
  }
  if (template === undefined) {
    if (import.meta.env.DEV && !warned.has(key)) {
      warned.add(key);
      console.warn(`i18n: missing key "${key}"`);
    }
    return key;
  }
  const f = formatter(used, key, template);
  if (!f) return template;
  try {
    const out = f.format(toValues(args));
    return typeof out === "string" ? out : template;
  } catch {
    return template;
  }
}

/** Test seam: replaces the whole catalog of one language and drops cached formatters and warnings. */
export function __setCatalogForTest(lang: Lang, entries: Record<string, string>): void {
  catalogs = { ...catalogs, [lang]: entries };
  formatters.clear();
  warned.clear();
}

/** Test seam: restores the shipped catalogs. */
export function __resetCatalogsForTest(): void {
  catalogs = { ...original };
  formatters.clear();
  warned.clear();
}
