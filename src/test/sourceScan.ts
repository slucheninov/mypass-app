import { readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";

/** The source tree the static guards scan: everything under `src/` that ships to the webview. */
const SRC = join(process.cwd(), "src");

// Generated, translated or test-only code is not subject to the source guards.
function excluded(rel: string): boolean {
  return (
    rel === "lib/bindings.ts" ||
    rel.startsWith("lib/i18n/locales/") ||
    rel.startsWith("test/") ||
    rel.endsWith(".test.ts")
  );
}

function walk(dir: string, out: string[]): void {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full, out);
    else out.push(full);
  }
}

/** Every shipped source file with one of the extensions (without the dot), `path` relative to the repo root with `/`. */
export function sourceFiles(exts: string[]): { path: string; text: string }[] {
  const all: string[] = [];
  walk(SRC, all);
  return all
    .filter((f) => exts.some((e) => f.endsWith(`.${e}`)))
    .filter((f) => !excluded(relative(SRC, f).split(sep).join("/")))
    .sort()
    .map((f) => ({
      path: relative(process.cwd(), f).split(sep).join("/"),
      text: readFileSync(f, "utf8"),
    }));
}
