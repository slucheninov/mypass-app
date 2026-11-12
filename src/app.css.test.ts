import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

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
