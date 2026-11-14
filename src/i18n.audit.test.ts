// The final string audit: the guards that keep the catalogs and the source in step. Completeness
// of every key in uk and ru and the placeholder sets are pinned by scripts/i18n-completeness.test.mjs
// (plain Node); the checks here need the source scan of the UI.
import { describe, expect, it } from "vitest";
import en from "./lib/i18n/locales/en.json";
import ru from "./lib/i18n/locales/ru.json";
import uk from "./lib/i18n/locales/uk.json";
import { sourceFiles } from "./test/sourceScan";
import { UNTRANSLATED_TEXT_ALLOW_LIST, untranslatedFindings } from "./test/textScan";

const catalogs = { en, uk, ru } as Record<string, Record<string, string>>;

// Only `t("literal")` calls are seen. That is complete by construction: `every_t_call_takes_a_string_literal_key`
// (i18n.test.ts) fails on any other first argument, so a key cannot reach `t()` through a variable.
function usedKeys(): Set<string> {
  const keys = new Set<string>();
  const re = /\bt\(\s*(["'`])((?:\\.|(?!\1).)*)\1/g;
  for (const f of sourceFiles(["ts", "svelte"])) {
    for (const m of f.text.matchAll(re)) keys.add((m[2] ?? "").replace(/\\(["'`\\])/g, "$1"));
  }
  return keys;
}

// Proper names and the like that read the same in every language.
const SAME_IN_EVERY_LANGUAGE = new Set(["MyPass", "URL", "English", "Bitwarden", "1Password", "iCloud Passwords"]);

describe("string audit", () => {
  it("no_literal_user_text_survives_in_svelte_files", () => {
    // The same scan as `no_untranslated_text_nodes_in_svelte_files`, run here with nothing exempted.
    expect(UNTRANSLATED_TEXT_ALLOW_LIST).toEqual([]);
    const files = sourceFiles(["svelte"]);
    expect(files.length).toBeGreaterThan(30);
    const bad: Record<string, string[]> = {};
    for (const f of files) {
      const found = untranslatedFindings(f.text);
      if (found.length > 0) bad[f.path] = found;
    }
    expect(bad).toEqual({});
  });

  it("every_key_used_in_source_is_translated_in_uk_and_ru", () => {
    const offenders: string[] = [];
    for (const key of usedKeys()) {
      for (const lang of ["en", "uk", "ru"]) {
        const value = catalogs[lang]?.[key];
        if (typeof value !== "string" || value === "") offenders.push(`${lang}: ${key}`);
        else if (lang !== "en" && value === en[key as keyof typeof en] && !SAME_IN_EVERY_LANGUAGE.has(key) && /[A-Za-z]{3,}/.test(key)) {
          offenders.push(`${lang} (still English): ${key}`);
        }
      }
    }
    expect(offenders.slice(0, 20), `${offenders.length} offenders`).toEqual([]);
  });

  it("plural_keys_carry_every_form_their_language_needs", () => {
    const forms = (value: string) =>
      [...value.matchAll(/(?:^|[\s{}])(zero|one|two|few|many|other)\s*\{/g)].map((m) => m[1] as string);
    const wanted: Record<string, string[]> = {
      en: ["one", "other"],
      uk: ["few", "many", "one", "other"],
      ru: ["few", "many", "one", "other"],
    };
    const bad: string[] = [];
    for (const [lang, catalog] of Object.entries(catalogs)) {
      for (const [key, value] of Object.entries(catalog)) {
        if (!/\{\w+,\s*plural,/.test(value)) continue;
        if (forms(value).sort().join() !== wanted[lang]?.join()) bad.push(`${lang}: ${key}`);
      }
    }
    expect(bad).toEqual([]);
  });
});
