// The final string audit: the guards that keep the catalogs and the source in step. Completeness
// of every key in uk and ru and the placeholder sets are pinned by scripts/i18n-completeness.test.mjs
// (plain Node); the checks here need the source scan of the UI.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
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

// Keys that no `t("...")` call of the UI references because the native side (src-tauri/src: tray, quit
// dialogs, startup errors, the passkey presence prompt) reads them straight from the shared catalog
// through `i18n::t`, `i18n::t_resolved` or a `key` variable. Every entry is checked below to still
// appear as a string literal in the Rust sources, so the list cannot outlive its use.
const NATIVE_KEYS = [
  "Cancel",
  "Create a passkey for {0}",
  "Lock",
  "MyPass %@ available\u2026",
  "MyPass could not start",
  "Open MyPass",
  "Quit",
  "Quit Anyway",
  "Quit MyPass",
  "Settings\u2026",
  "Sign in to {0} with a passkey",
  "The entry editor has unsaved changes. If you quit, they will be lost.",
  "The vault could not be saved (%@). If you quit anyway, MyPass tries to keep your unsaved changes as a recovery copy in the backups folder.",
  "Unlock",
  "Unlock MyPass",
  "startup.failure.message",
  "startup.webview2.missing",
];

// Key prefixes that code builds at run time. Empty on purpose: `every_t_call_takes_a_string_literal_key`
// (i18n.test.ts) rejects `t(variable)` and a template literal key, so the UI cannot build a key. Add a
// prefix here, with the reason, only if that rule is ever relaxed.
const DYNAMIC_KEY_PREFIXES: string[] = [];

function rustSources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) rustSources(full, out);
    else if (entry.name.endsWith(".rs")) out.push(readFileSync(full, "utf8"));
  }
  return out;
}

// Proper names and the like that read the same in every language.
const SAME_IN_EVERY_LANGUAGE = new Set(["MyPass", "URL", "English", "Bitwarden", "1Password", "iCloud Passwords", "iCloud Drive"]);

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

  it("every_catalog_key_is_referenced", () => {
    // A key that nothing references is dead weight in every language: delete it from all catalogs.
    const used = usedKeys();
    const native = new Set(NATIVE_KEYS);
    const unreferenced = Object.keys(en).filter(
      (key) => !used.has(key) && !native.has(key) && !DYNAMIC_KEY_PREFIXES.some((p) => key.startsWith(p)),
    );
    expect(unreferenced, `${unreferenced.length} unreferenced keys`).toEqual([]);
  });

  it("native_keys_are_still_read_by_the_rust_side", () => {
    const rust = rustSources(join(process.cwd(), "src-tauri", "src"))
      .join("\n")
      .replace(/\\u\{([0-9a-fA-F]+)\}/g, (_, hex: string) => String.fromCodePoint(parseInt(hex, 16)));
    const stale = NATIVE_KEYS.filter((key) => !rust.includes(`"${key}"`) || !(key in en));
    expect(stale).toEqual([]);
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
