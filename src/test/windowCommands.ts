// Test-only, conservative source scan. Follow relative imports, but visit shared store methods
// only when referenced: importing device status must not imply the settings-only configure call.
import { readFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import ts from "typescript";

export function windowCommands(root: string): Set<string> {
  const seen = new Set<string>();
  const methods = new Map<string, Map<string, string>>();
  const references = new Set<string>();
  const commands = new Set<string>();
  const pending = [resolve(root)];
  const enqueue = (from: string, name: string) => {
    if (!name.startsWith(".") || /\.(json|css)$/.test(name)) return;
    const base = resolve(dirname(from), name);
    const file = [base, `${base}.ts`, `${base}.svelte`, `${base}/index.ts`].find(p => existsSync(p) && /\.(ts|svelte)$/.test(p));
    if (!file) throw new Error(`Unresolved import ${name} from ${from}`);
    if (!file.endsWith("/api.ts") && !file.endsWith("/bindings.ts")) pending.push(file);
  };
  const scan = (text: string) => {
    for (const match of text.matchAll(/\bapi\s*\.\s*(\w+)/g)) commands.add(match[1]!);
    for (const match of text.matchAll(/\b(\w+)\s*\.\s*(\w+)/g)) references.add(`${match[1]}.${match[2]}`);
  };
  while (pending.length) {
    const path = pending.pop()!;
    if (seen.has(path)) continue;
    seen.add(path);
    const text = readFileSync(path, "utf8");
    for (const match of text.matchAll(/\bimport\s+(?!type\b)[\s\S]*?\bfrom\s*["']([^"']+)["']/g)) enqueue(path, match[1]!);
    const store = /export const (\w+) = new \w+Store\(/.exec(text)?.[1];
    if (store) {
      const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true);
      const bodies = new Map<string, string>();
      for (const node of source.statements) if (ts.isClassDeclaration(node)) {
        for (const member of node.members) if (ts.isMethodDeclaration(member) && member.body) {
          // Rewrite this.method (including private methods) into singleton.method references.
          bodies.set(member.name.getText(source), member.body.getText(source).replace(/\bthis\./g, `${store}.`));
        }
      }
      methods.set(store, bodies);
    } else scan(text);
  }
  const visited = new Set<string>();
  let changed = true;
  while (changed) {
    changed = false;
    for (const [store, bodies] of methods) for (const [name, body] of bodies) {
      const key = `${store}.${name}`;
      // Private helper calls use '#'; the reference scan below preserves that prefix too.
      if (!references.has(key) || visited.has(key)) continue;
      visited.add(key);
      scan(body);
      for (const match of body.matchAll(/\b(\w+)\.(#\w+)/g)) references.add(`${match[1]}.${match[2]}`);
      changed = true;
    }
  }
  return commands;
}
