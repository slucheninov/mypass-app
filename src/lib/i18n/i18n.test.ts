import { render, screen } from "@testing-library/svelte";
import { flushSync } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import I18nProbe from "../../test/I18nProbe.svelte";
import { sourceFiles } from "../../test/sourceScan";
import { formatBytes, formatDate } from "./format";
import { type Lang, __resetCatalogsForTest, __setCatalogForTest, i18n, resolveSystemLanguage, setLanguage, t } from "./index.svelte";
import en from "./locales/en.json";
import ru from "./locales/ru.json";
import uk from "./locales/uk.json";

const cat = { en, uk, ru } as Record<string, Record<string, string>>;

beforeEach(() => {
  __resetCatalogsForTest();
  setLanguage("en");
});
afterEach(() => vi.restoreAllMocks());

describe("t", () => {
  it("english_is_the_default_and_returns_source_text", () => {
    expect(t("Cancel")).toBe("Cancel");
  });

  it("switching_language_is_reactive", () => {
    render(I18nProbe, { text: "Cancel" });
    expect(screen.getByTestId("probe").textContent).toBe("Cancel");
    setLanguage("uk");
    flushSync();
    expect(screen.getByTestId("probe").textContent).toBe(cat.uk!["Cancel"]);
    expect(cat.uk!["Cancel"]).not.toBe("Cancel");
  });

  it("missing_key_falls_back_to_english_then_to_key", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    __setCatalogForTest("en", { "Only english": "Only english text" });
    setLanguage("ru");
    expect(t("Only english")).toBe("Only english text");
    expect(t("No such key anywhere")).toBe("No such key anywhere");
  });

  it("missing_key_warns_once_per_key_in_dev", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    t("Warn me once");
    t("Warn me once");
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("positional_args_array", () => {
    __setCatalogForTest("en", { "Hello %@, %lld": "Hello {0}, {1}" });
    expect(t("Hello %@, %lld", ["Ann", 3])).toBe("Hello Ann, 3");
  });

  it("number_arg_fills_zero_and_count", () => {
    __setCatalogForTest("en", {
      "%lld": "{0}",
      "%lld files": "{count, plural, one {# file} other {# files}}",
    });
    expect(t("%lld", 7)).toBe("7");
    expect(t("%lld files", 1)).toBe("1 file");
    expect(t("%lld files", 2)).toBe("2 files");
  });

  it("record_with_count_also_fills_zero", () => {
    __setCatalogForTest("en", { "Length: %lld": "Length: {0}" });
    expect(t("Length: %lld", { count: 12 })).toBe("Length: 12");
  });

  it("plural_ru_uk_forms", () => {
    const msg = "{count, plural, one {# \u0444\u0430\u0439\u043b} few {# \u0444\u0430\u0439\u043b\u0430} many {# \u0444\u0430\u0439\u043b\u043e\u0432} other {# \u0444\u0430\u0439\u043b\u0430}}";
    const f = (n: number, w: string) => `${n} \u0444\u0430\u0439\u043b${w}`;
    for (const lang of ["ru", "uk"] as const) {
      __setCatalogForTest(lang, { files: msg });
      setLanguage(lang);
      expect(t("files", 1)).toBe(f(1, ""));
      expect(t("files", 2)).toBe(f(2, "\u0430"));
      expect(t("files", 5)).toBe(f(5, "\u043e\u0432"));
      expect(t("files", 11)).toBe(f(11, "\u043e\u0432"));
      expect(t("files", 21)).toBe(f(21, ""));
      expect(t("files", 22)).toBe(f(22, "\u0430"));
    }
  });

  it("plural_en_one_other", () => {
    __setCatalogForTest("en", { files: "{count, plural, one {# file} other {# files}}" });
    expect(t("files", 1)).toBe("1 file");
    expect(t("files", 2)).toBe("2 files");
  });

  it("real_catalog_plural_key_works_in_ru", () => {
    setLanguage("ru");
    expect(t("%lld files", 5)).toBe("5 \u0444\u0430\u0439\u043b\u043e\u0432");
  });

  it("real_catalog_plural_forms_are_pinned_in_uk_and_ru", () => {
    const word = (n: number, w: string) => `${n} \u0444\u0430\u0439\u043b${w}`;
    const cases: [Lang, number, string][] = [
      ["uk", 1, ""], ["uk", 2, "\u0438"], ["uk", 5, "\u0456\u0432"], ["uk", 21, ""], ["uk", 22, "\u0438"],
      ["ru", 1, ""], ["ru", 2, "\u0430"], ["ru", 5, "\u043e\u0432"], ["ru", 21, ""],
    ];
    for (const [lang, n, w] of cases) {
      setLanguage(lang);
      expect(t("%lld files", n)).toBe(word(n, w));
    }
  });

  it("missing_argument_returns_the_raw_template", () => {
    __setCatalogForTest("en", { Needs: "Hi {0}" });
    expect(t("Needs")).toBe("Hi {0}");
  });

  it("argument_values_are_output_literally", () => {
    __setCatalogForTest("en", { Show: "Name: {0}" });
    expect(t("Show", ["{0}"])).toBe("Name: {0}");
    expect(t("Show", ["{x, plural, one {a} other {b}}"])).toBe("Name: {x, plural, one {a} other {b}}");
  });

  it("format_error_returns_template_not_throw", () => {
    __setCatalogForTest("en", { Broken: "Hello {name, plural, " });
    expect(t("Broken")).toBe("Hello {name, plural, ");
    __setCatalogForTest("en", { Needs: "Hi {0}" });
    expect(() => t("Needs")).not.toThrow();
  });
});

describe("language", () => {
  it("system_resolves_from_navigator_language", () => {
    expect(resolveSystemLanguage(["uk-UA"])).toBe("uk");
    expect(resolveSystemLanguage(["ru"])).toBe("ru");
    expect(resolveSystemLanguage(["de-DE", "uk"])).toBe("en");
    expect(resolveSystemLanguage([])).toBe("en");
    expect(resolveSystemLanguage(["UK"])).toBe("uk");
  });

  it("setLanguage_system_uses_injected_navigator_languages", () => {
    setLanguage("system", ["ru-RU"]);
    expect(i18n.lang).toBe("ru");
    setLanguage("system", ["fr"]);
    expect(i18n.lang).toBe("en");
  });

  it("html_lang_attribute_follows_language", () => {
    setLanguage("uk");
    expect(document.documentElement.lang).toBe("uk");
    setLanguage("ru");
    expect(document.documentElement.lang).toBe("ru");
  });
});

describe("format", () => {
  const ms = 1700000000000;
  it("date_formatting_follows_language", () => {
    expect(formatDate(ms)).toContain("2023");
    setLanguage("ru");
    expect(formatDate(ms)).toContain("2023");
    expect(formatDate(ms, "datetime")).toMatch(/\p{Script=Cyrillic}/u);
  });

  it("date_takes_milliseconds_not_seconds", () => {
    expect(formatDate(ms)).not.toContain("1970");
  });

  it("format_bytes_units", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1,023 B");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5 MB");
  });
});

describe("catalog scan", () => {
  it("every_t_call_key_in_source_exists_in_en_catalog", () => {
    const re = /\bt\(\s*(["'`])((?:\\.|(?!\1).)*)\1/g;
    const missing: string[] = [];
    for (const f of sourceFiles(["ts", "svelte"])) {
      for (const m of f.text.matchAll(re)) {
        const key = (m[2] ?? "").replace(/\\(["'`\\])/g, "$1");
        if (!Object.hasOwn(en, key)) missing.push(`${f.path}: ${key}`);
      }
    }
    expect(missing, `keys missing from en.json:\n${missing.join("\n")}`).toEqual([]);
  });

  it("every_t_call_takes_a_string_literal_key", () => {
    // `t(variable)` would slip past the catalog scan above. Method calls (`.t(`) are not ours.
    const re = /(?<![.\w$])(?<!function )t\(\s*([^\s)])/g;
    const dynamic: string[] = [];
    for (const f of sourceFiles(["ts", "svelte"])) {
      for (const m of f.text.matchAll(re)) {
        if (!["'", '"', "`"].includes(m[1] ?? "")) dynamic.push(`${f.path}: ${m[0]}`);
      }
    }
    expect(dynamic, `t() needs a literal key:\n${dynamic.join("\n")}`).toEqual([]);
  });
});
