// Static guards over the frontend sources plus the window-label mounting test. The guards read the
// source tree through `sourceFiles`, so a new file is covered the moment it exists.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { clearMocks, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { sourceFiles } from "./test/sourceScan";

// ---------------------------------------------------------------------------------------------
// Mounting
// ---------------------------------------------------------------------------------------------

describe("main.ts", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="app"></div>';
    vi.resetModules();
  });
  afterEach(() => {
    clearMocks();
    document.body.innerHTML = "";
  });

  it.each([
    ["main", "main-app"],
    ["tray-panel", "tray-panel"],
    ["settings", "settings-app"],
    ["something-else", "main-app"],
  ])("main_mounts_root_for_each_window_label (%s)", async (label, testId) => {
    mockWindows(label);
    await import("./main");
    expect(document.querySelector(`[data-testid="${testId}"]`)).not.toBeNull();
  });
});

// ---------------------------------------------------------------------------------------------
// Source scans
// ---------------------------------------------------------------------------------------------

const SOURCE_EXTS = ["ts", "svelte"];

function offenders(
  files: { path: string; text: string }[],
  test: (f: { path: string; text: string }) => boolean,
): string[] {
  return files.filter(test).map((f) => f.path);
}

describe("source scan coverage", () => {
  it("sees the placeholder windows (the guards below are not vacuous)", () => {
    const paths = sourceFiles(["svelte"]).map((f) => f.path);
    expect(paths).toEqual(
      expect.arrayContaining([
        "src/windows/MainApp.svelte",
        "src/windows/SettingsApp.svelte",
        "src/windows/TrayPanel.svelte",
      ]),
    );
    const all = sourceFiles(["ts", "svelte", "css"]).map((f) => f.path);
    expect(all.some((p) => p.endsWith("bindings.ts"))).toBe(false);
    expect(all.some((p) => p.includes("/test/"))).toBe(false);
    expect(all.some((p) => p.endsWith(".test.ts"))).toBe(false);
  });
});

describe("no_direct_invoke_outside_bindings", () => {
  it("only src/lib/bindings.ts and src/test/ touch the core IPC", () => {
    // `sourceFiles` already excludes bindings.ts and src/test/: nothing else may match.
    const bad = offenders(
      sourceFiles(SOURCE_EXTS),
      (f) => /\binvoke\s*\(/.test(f.text) || f.text.includes("@tauri-apps/api/core"),
    );
    expect(bad).toEqual([]);
  });
});

describe("invoke scanner", () => {
  const flags = (text: string) => /\binvoke\s*\(/.test(text) || text.includes("@tauri-apps/api/core");

  it("flags an invoke import and a bare invoke call", () => {
    expect(flags('import { invoke } from "@tauri-apps/api/core";')).toBe(true);
    expect(flags('const x = await invoke("app_info");')).toBe(true);
    expect(flags("await commands.appInfo();")).toBe(false);
  });

  it("exempts bindings.ts and src/test/ from the scan", () => {
    // bindings.ts really contains the import, so the exemption is what keeps the guard green.
    const bindings = readFileSync(resolve(process.cwd(), "src/lib/bindings.ts"), "utf8");
    expect(flags(bindings)).toBe(true);
    const scanned = sourceFiles(["ts", "svelte"]).map((f) => f.path);
    expect(scanned).not.toContain("src/lib/bindings.ts");
    expect(scanned.some((p) => p.startsWith("src/test/"))).toBe(false);
  });
});

describe("no_remote_urls_in_sources", () => {
  const ALLOWED = [
    "https://github.com/slucheninov/mypass-app",
    "https://www.apache.org/licenses/LICENSE-2.0",
  ];
  const URL_RE = /https?:\/\/[A-Za-z0-9]/g;

  it("only the allow-listed URLs appear", () => {
    const bad: string[] = [];
    for (const f of sourceFiles(["ts", "svelte", "css"])) {
      let text = f.text;
      for (const ok of ALLOWED) text = text.split(ok).join("");
      if (URL_RE.test(text)) bad.push(f.path);
      URL_RE.lastIndex = 0;
    }
    expect(bad).toEqual([]);
  });

  it("a bare scheme placeholder is not a URL (R23)", () => {
    expect(URL_RE.test("https://")).toBe(false);
    URL_RE.lastIndex = 0;
    expect(URL_RE.test("https://example.org")).toBe(true);
    URL_RE.lastIndex = 0;
  });
});

describe("html_directive_only_in_markdown_view", () => {
  it("{@html is confined to MarkdownView.svelte", () => {
    const bad = offenders(
      sourceFiles(["svelte", "ts"]),
      (f) => f.text.includes("{@html") && f.path !== "src/screens/MarkdownView.svelte",
    );
    expect(bad).toEqual([]);
  });
});

describe("no_other_html_sinks", () => {
  it("innerHTML, outerHTML and insertAdjacentHTML never appear", () => {
    const bad = offenders(sourceFiles(SOURCE_EXTS), (f) => /\b(innerHTML|outerHTML|insertAdjacentHTML)\b/.test(f.text));
    expect(bad).toEqual([]);
  });
});

describe("no_secret_sinks", () => {
  it("web storage and the document title are not used", () => {
    // The future preferences module may be added to this allow-list when it lands.
    const bad = offenders(
      sourceFiles(SOURCE_EXTS),
      (f) => /\b(localStorage|sessionStorage)\b/.test(f.text) || /document\.title\s*=/.test(f.text),
    );
    expect(bad).toEqual([]);
  });

  it("console is used only by the i18n dev warning", () => {
    const bad = offenders(
      sourceFiles(SOURCE_EXTS),
      (f) => /\bconsole\./.test(f.text) && f.path !== "src/lib/i18n/index.svelte.ts",
    );
    expect(bad).toEqual([]);
  });
});

describe("no_eval_or_function_constructor", () => {
  it("neither eval( nor new Function( appears", () => {
    const bad = offenders(
      sourceFiles(SOURCE_EXTS),
      (f) => /\beval\s*\(/.test(f.text) || /new\s+Function\s*\(/.test(f.text),
    );
    expect(bad).toEqual([]);
  });
});

// ---- untranslated text -------------------------------------------------------------------

// Files that may carry literal text. Empty on purpose: every user-visible string goes through t().
const UNTRANSLATED_TEXT_ALLOW_LIST: string[] = [];

/** Removes `<script>` and `<style>` blocks and HTML comments. */
function markupOf(text: string): string {
  return text
    .replace(/<script\b[\s\S]*?<\/script>/g, "")
    .replace(/<style\b[\s\S]*?<\/style>/g, "")
    .replace(/<!--[\s\S]*?-->/g, "");
}

/** Index just after the `}` that closes the `{` at `from` (nested braces and string literals). */
function skipBraces(src: string, from: number): number {
  let depth = 0;
  let quote: string | null = null;
  for (let i = from; i < src.length; i++) {
    const c = src.charAt(i);
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "{") depth++;
    else if (c === "}" && --depth === 0) return i + 1;
  }
  return src.length;
}

/** Splits markup into the text nodes and the tags (the raw text of each `<...>`). */
function scanMarkup(markup: string): { texts: string[]; tags: string[] } {
  const texts: string[] = [];
  const tags: string[] = [];
  let text = "";
  let i = 0;
  const flush = () => {
    if (text.trim() !== "") texts.push(text.trim());
    text = "";
  };
  while (i < markup.length) {
    const c = markup.charAt(i);
    if (c === "{") {
      i = skipBraces(markup, i); // {expression}, {#if}, {/if}, {@html} ...: not literal text
      continue;
    }
    if (c === "<" && /[A-Za-z/]/.test(markup.charAt(i + 1))) {
      flush();
      let j = i + 1;
      while (j < markup.length && markup.charAt(j) !== ">") {
        const d = markup.charAt(j);
        if (d === "{") j = skipBraces(markup, j);
        else if (d === '"' || d === "'") {
          const end = markup.indexOf(d, j + 1);
          j = end === -1 ? markup.length : end + 1;
        } else j++;
      }
      tags.push(markup.slice(i, j + 1));
      i = j + 1;
      continue;
    }
    text += c;
    i++;
  }
  flush();
  return { texts, tags };
}

// The negative look-behind anchors the name: `data-title="..."` or `x:title="..."` is not `title`.
const LITERAL_ATTR_RE = /(?<![\w:.-])(aria-label|title|placeholder|alt)\s*=\s*(?:"([^"]*)"|'([^']*)')/g;

/** A quoted value that is exactly one `{expression}` is translated text, not a literal. */
function isSingleExpression(value: string): boolean {
  const v = value.trim();
  return v.startsWith("{") && skipBraces(v, 0) === v.length;
}

function untranslatedFindings(svelte: string): string[] {
  const { texts, tags } = scanMarkup(markupOf(svelte));
  const found = texts.filter((t) => /\p{L}/u.test(t)).map((t) => `text: ${t}`);
  for (const tag of tags) {
    for (const m of tag.matchAll(LITERAL_ATTR_RE)) {
      if (!isSingleExpression(m[2] ?? m[3] ?? "")) found.push(`attribute: ${m[0]}`);
    }
  }
  return found;
}

describe("no_untranslated_text_nodes_in_svelte_files", () => {
  it("every .svelte file is free of literal text and literal aria-label/title/placeholder", () => {
    const bad: Record<string, string[]> = {};
    for (const f of sourceFiles(["svelte"])) {
      if (UNTRANSLATED_TEXT_ALLOW_LIST.includes(f.path)) continue;
      const found = untranslatedFindings(f.text);
      if (found.length > 0) bad[f.path] = found;
    }
    expect(bad).toEqual({});
  });

  it("the scanner flags literal text and attributes and accepts expressions", () => {
    expect(untranslatedFindings("<p>Hello</p>")).toEqual(["text: Hello"]);
    expect(untranslatedFindings('<button aria-label="Close">{t("x")}</button>')).toEqual([
      'attribute: aria-label="Close"',
    ]);
    expect(untranslatedFindings('<input placeholder="Search" />')).toEqual([
      'attribute: placeholder="Search"',
    ]);
    expect(untranslatedFindings('<a title="Go">{x}</a>')).toEqual(['attribute: title="Go"']);
    expect(untranslatedFindings('<img alt="Logo" />')).toEqual(['attribute: alt="Logo"']);
    expect(untranslatedFindings('<b aria-label="Close {x}">{y}</b>')).toEqual([
      'attribute: aria-label="Close {x}"',
    ]);
    expect(untranslatedFindings('<b title="{a} and {b}">{y}</b>')).toEqual([
      'attribute: title="{a} and {b}"',
    ]);
    // A quoted attribute that is exactly one expression is fine; data-* names are not guarded.
    expect(untranslatedFindings(`<b aria-label="{t('x')}" alt='{t("y")}'>{z}</b>`)).toEqual([]);
    expect(
      untranslatedFindings('<b data-title="x" data-placeholder="y" x:title="z" class="title">{z}</b>'),
    ).toEqual([]);
    // Expressions, blocks, arrow functions inside tags and script/style are not literal text.
    const ok =
      "<script>let a = 1 > 0 ? 'text' : 'more';</script>" +
      "<style>p { color: red }</style>" +
      '<button onclick={() => go(1)} aria-label={t("Close")} title={t("Go")}>{t("Hi")}</button>' +
      '{#if a}<span>{a}</span>{:else}<b data-x="1">{b}</b>{/if}' +
      "<p>  42  </p><!-- comment words -->";
    expect(untranslatedFindings(ok)).toEqual([]);
  });
});

describe("no_legacy_svelte_syntax", () => {
  it("runes only: no export let, no $: labels, no on: directives", () => {
    const bad = offenders(
      sourceFiles(["svelte"]),
      (f) =>
        /\bexport\s+let\b/.test(f.text) ||
        /^\s*\$:/m.test(f.text) ||
        /\son:[a-z]+\s*[={]/i.test(markupOf(f.text)),
    );
    expect(bad).toEqual([]);
  });
});

describe("no_os_name_checks_outside_window_controls_and_reveal_label", () => {
  const ALLOWED = ["WindowControls.svelte", "settings/Vault.svelte"];
  const OS_CHECK = /\.os\s*[!=]==?|\bos\s*[!=]==?\s*["']/;

  it("OS names are compared only where the design allows", () => {
    const bad = offenders(
      sourceFiles(["svelte"]),
      (f) => OS_CHECK.test(f.text) && !ALLOWED.some((a) => f.path.endsWith(a)),
    );
    expect(bad).toEqual([]);
  });

  it("the pattern catches the usual spellings", () => {
    expect(OS_CHECK.test('if (platform.os === "windows")')).toBe(true);
    expect(OS_CHECK.test("info.os == 'macos'")).toBe(true);
    expect(OS_CHECK.test("const cost = 3")).toBe(false);
  });
});

// ---------------------------------------------------------------------------------------------
// Tauri platform configuration
// ---------------------------------------------------------------------------------------------

type Win = { label: string } & Record<string, unknown>;

function conf(name: string): { app?: { windows?: Win[] } } {
  return JSON.parse(readFileSync(resolve(process.cwd(), "src-tauri", name), "utf8"));
}
const base = () => conf("tauri.conf.json").app?.windows ?? [];
const windowsConf = () => conf("tauri.windows.conf.json").app?.windows ?? [];
const macosConf = () => conf("tauri.macos.conf.json").app?.windows ?? [];

function byLabel(list: Win[], label: string): Win {
  const w = list.find((x) => x.label === label);
  if (!w) throw new Error(`window ${label} missing`);
  return w;
}

describe("tauri platform configuration", () => {
  it("the base file carries neither decorations nor a title bar style for main, only a min size", () => {
    const main = byLabel(base(), "main");
    expect(main).not.toHaveProperty("decorations");
    expect(main).not.toHaveProperty("titleBarStyle");
    expect(typeof main.minWidth).toBe("number");
    expect(typeof main.minHeight).toBe("number");
    // The three panes need 800 px plus separators (preflight ruling R18).
    expect(main.minWidth as number).toBeGreaterThanOrEqual(820);
    expect(main.minHeight as number).toBeGreaterThanOrEqual(520);
  });

  it("tauri_windows_conf_disables_decorations_for_main_only", () => {
    const win = windowsConf();
    const main = byLabel(win, "main");
    expect(main.decorations).toBe(false);
    // The min size stays the base one (a platform file must not shrink or grow it).
    expect(main.minWidth).toBe(byLabel(base(), "main").minWidth);
    expect(main.minHeight).toBe(byLabel(base(), "main").minHeight);
    // main is the base main plus `decorations: false` and nothing else.
    expect(main).toEqual({ ...byLabel(base(), "main"), decorations: false });
    for (const w of win.filter((x) => x.label !== "main")) {
      expect(w).toEqual(byLabel(base(), w.label));
    }
  });

  it("tauri_macos_conf_uses_overlay_title_bar_for_main", () => {
    const mac = macosConf();
    const main = byLabel(mac, "main");
    expect(main.titleBarStyle).toBe("Overlay");
    expect(main.hiddenTitle).toBe(true);
    expect(main.trafficLightPosition).toEqual(
      expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
    );
    expect(main).not.toHaveProperty("decorations");
    const { titleBarStyle: _a, hiddenTitle: _b, trafficLightPosition: _c, ...rest } = main;
    expect(rest).toEqual(byLabel(base(), "main"));
    for (const w of mac.filter((x) => x.label !== "main")) {
      expect(w).toEqual(byLabel(base(), w.label));
      expect(w).not.toHaveProperty("titleBarStyle");
    }
  });

  it("tauri_platform_confs_list_the_same_window_labels_as_base", () => {
    // JSON Merge Patch replaces arrays: a missing label would silently delete a window.
    const labels = (list: Win[]) => list.map((w) => w.label);
    expect(labels(windowsConf())).toEqual(labels(base()));
    expect(labels(macosConf())).toEqual(labels(base()));
  });
});
