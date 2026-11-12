import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { contrast } from "./test/contrast";

const css = readFileSync(resolve(process.cwd(), "src/app.css"), "utf8");

/** The body of the first rule whose selector text equals `selector`, or the empty string. */
function block(selector: string, from = 0): string {
  const start = css.indexOf(selector, from);
  if (start === -1) return "";
  const open = css.indexOf("{", start);
  const close = css.indexOf("}", open);
  return css.slice(open + 1, close);
}

describe("design tokens", () => {
  it("tokens_match_style_a", () => {
    const root = block(":root {");
    expect(root).toContain("--accent: #2f6feb");
    expect(root).toContain("--radius: 6px");
    expect(root).toContain("--sidebar-bg: #f5f5f7");
    expect(root).toContain("--bg: #ffffff");
    expect(root).toContain("--list-bg: #fbfbfc");
    expect(root).toContain("--separator: #e3e3e6");
    expect(root).toContain("--text: #1d1d1f");
    expect(root).toContain("--text-2: #6e6e73");
    expect(root).toContain("--sep: 1px solid var(--separator)");
    const dark = block(':root[data-theme="dark"]');
    expect(dark).toContain("--bg: #1e1e20");
    expect(dark).toContain("--sidebar-bg: #252528");
    expect(dark).toContain("--list-bg: #212124");
    expect(dark).toContain("--separator: #38383c");
    expect(dark).toContain("--text: #f2f2f4");
    expect(dark).toContain("--text-2: #9a9aa0");
  });

  it("dark_follows_system_only_when_theme_unset", () => {
    const media = css.indexOf("@media (prefers-color-scheme: dark)");
    expect(media).toBeGreaterThan(-1);
    const guarded = block(":root:not([data-theme])", media);
    expect(guarded).toContain("--bg: #1e1e20");
    expect(css).toContain(':root[data-theme="dark"]');
    // The forced-dark block sits outside the media query.
    expect(css.indexOf(':root[data-theme="dark"]')).toBeGreaterThan(css.indexOf("}\n}", media));
  });

  it("font_stack_starts_with_system_ui_fonts", () => {
    const m = /--font:\s*([^;]+);/.exec(css);
    expect(m?.[1]).toBe('-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif');
  });

  it("has_one_visible_focus_rule_on_focus_visible", () => {
    expect(css).toContain(":focus-visible");
  });
});

// ---- contrast -------------------------------------------------------------------------------

function token(body: string, name: string): string {
  const m = new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`).exec(body);
  if (!m?.[1]) throw new Error(`token ${name} missing`);
  return m[1];
}

describe("contrast", () => {
  const light = block(":root {");
  const dark = block(':root[data-theme="dark"]');
  const media = block(":root:not([data-theme])", css.indexOf("@media (prefers-color-scheme: dark)"));

  it("contrast_helper_matches_known_values", () => {
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 0);
    expect(contrast("#777777", "#777777")).toBeCloseTo(1, 5);
  });

  it("accent_text_on_accent_reaches_aa_in_light_and_both_dark_blocks", () => {
    for (const body of [light, dark, media]) {
      expect(contrast(token(body, "--accent-text"), token(body, "--accent"))).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("text_and_secondary_text_reach_aa_on_the_backgrounds", () => {
    for (const body of [light, dark]) {
      for (const bg of ["--bg", "--sidebar-bg", "--list-bg"]) {
        expect(contrast(token(body, "--text"), token(body, bg))).toBeGreaterThanOrEqual(4.5);
        expect(contrast(token(body, "--text-2"), token(body, bg))).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("danger_text_and_close_hover_reach_aa", () => {
    expect(contrast(token(light, "--danger"), token(light, "--bg"))).toBeGreaterThanOrEqual(4.5);
    expect(contrast(token(dark, "--danger"), token(dark, "--bg"))).toBeGreaterThanOrEqual(4.5);
    expect(contrast(token(light, "--danger-hover-text"), token(light, "--danger-hover"))).toBeGreaterThanOrEqual(4.5);
  });

  it("the_two_dark_blocks_declare_identical_tokens", () => {
    const norm = (b: string) => b.split(";").map((x) => x.trim()).filter(Boolean).sort();
    expect(norm(media)).toEqual(norm(dark));
  });
});
