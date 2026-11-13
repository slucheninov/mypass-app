// The scanner behind `no_untranslated_text_nodes_in_svelte_files` (scaffold.test.ts), shared with the
// string audit (i18n.audit.test.ts).

// Files that may carry literal text. Empty on purpose: every user-visible string goes through t().
export const UNTRANSLATED_TEXT_ALLOW_LIST: string[] = [];

/** Removes `<script>` and `<style>` blocks and HTML comments. */
export function markupOf(text: string): string {
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

export function untranslatedFindings(svelte: string): string[] {
  const { texts, tags } = scanMarkup(markupOf(svelte));
  const found = texts.filter((t) => /\p{L}/u.test(t)).map((t) => `text: ${t}`);
  for (const tag of tags) {
    for (const m of tag.matchAll(LITERAL_ATTR_RE)) {
      if (!isSingleExpression(m[2] ?? m[3] ?? "")) found.push(`attribute: ${m[0]}`);
    }
  }
  return found;
}
