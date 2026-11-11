#!/usr/bin/env node
// One-time converter from the old Swift app's String Catalogs to the flat ICU MessageFormat JSON
// catalogs of the Tauri UI, plus per-language InfoPlist.strings files for the macOS bundle.
//
//   node scripts/convert-xcstrings.mjs --in <Localizable.xcstrings> --out src/lib/i18n/locales
//        [--infoplist-in <InfoPlist.xcstrings>] [--infoplist-out src-tauri/resources/macos]
//        [--merge] [--strict]
//
// Output: `<out>/{en,uk,ru}.json`, each a flat `{ "<English source string>": "<ICU message>" }`
// (keys sorted, 2-space JSON, LF). After the one-time migration the JSON files are the source of
// truth and are edited by hand; `--merge` keeps the keys already present in them, so hand edits
// survive a re-run.
//
// Conversion rules:
//   * `%@ %d %i %u %f %s` (with `l`/`ll` length, `.N` precision) and `%1$@`-style specifiers become
//     positional `{0}`, `{1}` in specifier order; an explicit `%n$` index is honoured (`%2$@` is
//     `{1}`). Mixed use follows NSString: an unnumbered specifier takes the next sequential
//     argument counted among the unnumbered ones only, so `%2$@ and %@` is `{1} and {0}` and
//     `%@ and %2$@` is `{0} and {1}`. Any other `%` (`100% sure`, `Rate 5%`) stays a literal `%`.
//   * A plural variation set becomes `{count, plural, one {...} few {...} many {...} other {...}}`.
//     The plural argument is always named `count` (the UI passes `{ count }`); it is the numeric
//     specifier with the lowest argument index, written `#` inside the branches. The other
//     specifiers keep their positional names. When any language of a key has a plural variation,
//     the numeric specifier is called `{count}` in every language of that key, so the placeholder
//     sets agree across languages.
//   * `%%` is a literal `%`. Output targets ICU apostrophe mode DOUBLE_OPTIONAL (default of ICU4J
//     and intl-messageformat): `{`/`}` in source text are wrapped in `'...'`, a literal `#` inside
//     a plural branch is quoted, and a literal `'` is doubled only where a single one would be
//     read as syntax (before a brace, `#`, `'` or placeholder, or right after a quoted span);
//     otherwise it stays a lone `'` (`Don't`).
//   * `device` variations take the `mac` branch.
//   * A string whose xcstrings state is not `translated` counts as missing. A translatable key that
//     is missing `uk` or `ru` fails `--strict` (exit 1, stderr names the key and language, nothing
//     is written). Without `--strict` the key is left out of that language's file.
//   * Keys marked `shouldTranslate: false` are copied verbatim into every language.
//   * SFTP-only keys are dropped by pattern and printed as a `dropped:` list on stdout, so the
//     owner can review what did not migrate.
//   * `InfoPlist.xcstrings` becomes `<infoplist-out>/<lang>.lproj/InfoPlist.strings`.
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const LANGS = ['en', 'uk', 'ru'];
const DROP_PATTERN = /sftp|tofu|host key|private key|certificate|passphrase/i;
const PLURAL_ORDER = ['zero', 'one', 'two', 'few', 'many', 'other'];
// Only what xcstrings actually use: %@ %d %i %u %f %s with an optional `N$` index, precision and
// `l`/`ll` length. No printf flags or width, so `100% sure` keeps its percent sign.
const SPECIFIER = /%%|%(?:(\d+)\$)?(?:\.\d+)?(?:ll|l)?([@diufs])/g;
const NUMERIC = new Set('diuf'.split(''));

function parseArgs(argv) {
  const args = { merge: false, strict: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = () => {
      if (i + 1 >= argv.length) throw new Error(`${arg} needs a value`);
      return argv[++i];
    };
    if (arg === '--in') args.in = value();
    else if (arg === '--out') args.out = value();
    else if (arg === '--infoplist-in') args.infoplistIn = value();
    else if (arg === '--infoplist-out') args.infoplistOut = value();
    else if (arg === '--merge') args.merge = true;
    else if (arg === '--strict') args.strict = true;
    else throw new Error(`unknown argument: ${arg}`);
  }
  if (!args.in || !args.out) throw new Error('--in and --out are required');
  return args;
}

// Split a format string into literal and specifier tokens.
function tokenize(text) {
  const tokens = [];
  let last = 0;
  let sequential = 0;
  for (const match of text.matchAll(SPECIFIER)) {
    if (match.index > last) tokens.push({ literal: text.slice(last, match.index) });
    last = match.index + match[0].length;
    if (match[0] === '%%') {
      tokens.push({ literal: '%' });
      continue;
    }
    const index = match[1] ? Number(match[1]) - 1 : sequential++;
    tokens.push({ index, numeric: NUMERIC.has(match[2]) });
  }
  if (last < text.length) tokens.push({ literal: text.slice(last) });
  return tokens;
}

// Escape one literal run for ICU. Target mode: DOUBLE_OPTIONAL (the default of ICU4J and
// intl-messageformat): `'` starts a quote only right before `{` or `}` (or `#` inside a plural
// branch), `''` is a literal apostrophe everywhere, and a lone `'` elsewhere is literal. So:
// `{...}` and stray braces (and `#` in plurals) are wrapped in quotes; an apostrophe is doubled
// when it touches syntax (next char is `'`, `{`, `}`, `#`-in-plural, a placeholder follows, or it
// comes straight after a quoted span, where a single `'` would close the quote early).
function escapeLiteral(literal, { inPlural, beforePlaceholder }) {
  const isSyntax = (c) => c === '{' || c === '}' || (inPlural && c === '#');
  let out = '';
  let afterQuote = false;
  let i = 0;
  while (i < literal.length) {
    const c = literal[i];
    const group = /^\{[^{}]*\}/.exec(literal.slice(i));
    if (group) {
      out += `'${group[0].replace(/'/g, "''")}'`;
      i += group[0].length;
      afterQuote = true;
    } else if (isSyntax(c)) {
      out += `'${c}'`;
      i++;
      afterQuote = true;
    } else if (c === "'") {
      const next = literal[i + 1];
      const atEnd = next === undefined;
      const double =
        afterQuote || (atEnd ? beforePlaceholder : next === "'" || isSyntax(next));
      out += double ? "''" : "'";
      i++;
      afterQuote = false;
    } else {
      out += c;
      i++;
      afterQuote = false;
    }
  }
  return out;
}

// `countName` is set when a numeric specifier is to be named `count`; `inPlural` writes it as `#`.
function toIcu(text, { countName = false, inPlural = false } = {}) {
  const tokens = tokenize(text);
  const numerics = tokens.filter((t) => t.numeric).map((t) => t.index);
  const countIndex = countName && numerics.length > 0 ? Math.min(...numerics) : -1;
  let out = '';
  tokens.forEach((token, i) => {
    if (token.literal === undefined) {
      if (token.index === countIndex) out += inPlural ? '#' : '{count}';
      else out += `{${token.index}}`;
      return;
    }
    const next = tokens[i + 1];
    const beforePlaceholder = next !== undefined && next.literal === undefined;
    out += escapeLiteral(token.literal, { inPlural, beforePlaceholder });
  });
  return out;
}

const isTranslated = (unit) => unit?.stringUnit?.state === 'translated';

// Resolve one localization node to { kind: 'text'|'plural', ... } or null when missing.
function resolveNode(node) {
  if (!node) return null;
  if (node.stringUnit) {
    return isTranslated(node) ? { text: node.stringUnit.value } : null;
  }
  const variations = node.variations;
  if (variations?.device) {
    const device = variations.device;
    return resolveNode(device.mac ?? device.other ?? Object.values(device)[0]);
  }
  if (variations?.plural) {
    const forms = {};
    for (const [category, unit] of Object.entries(variations.plural)) {
      if (!isTranslated(unit)) return null;
      forms[category] = unit.stringUnit.value;
    }
    if (Object.keys(forms).length === 0) return null;
    return { plural: forms };
  }
  return null;
}

function pluralToIcu(forms) {
  const categories = Object.keys(forms).sort(
    (a, b) => PLURAL_ORDER.indexOf(a) - PLURAL_ORDER.indexOf(b),
  );
  const parts = categories.map((c) => `${c} {${toIcu(forms[c], { countName: true, inPlural: true })}}`);
  if (!forms.other) {
    const fallback = forms.many ?? forms.few ?? forms.one ?? forms[categories[0]];
    parts.push(`other {${toIcu(fallback, { countName: true, inPlural: true })}}`);
  }
  return `{count, plural, ${parts.join(' ')}}`;
}

function convertLocalizable(catalog) {
  const result = Object.fromEntries(LANGS.map((l) => [l, {}]));
  const dropped = [];
  const missing = [];
  for (const [key, entry] of Object.entries(catalog.strings ?? {})) {
    if (key === '') continue;
    if (DROP_PATTERN.test(key)) {
      dropped.push(key);
      continue;
    }
    const locs = entry.localizations ?? {};
    const verbatim = entry.shouldTranslate === false;
    const resolved = {};
    for (const lang of LANGS) {
      let node = resolveNode(locs[lang]);
      if (!node && (lang === 'en' || verbatim)) node = { text: key };
      resolved[lang] = node;
    }
    const keyIsPlural = LANGS.some((l) => resolved[l]?.plural);
    for (const lang of LANGS) {
      const node = resolved[lang];
      if (!node) {
        missing.push(`${lang}: ${JSON.stringify(key)}`);
        continue;
      }
      result[lang][key] = node.plural
        ? pluralToIcu(node.plural)
        : toIcu(node.text, { countName: keyIsPlural });
    }
  }
  return { result, dropped, missing };
}

const sortObject = (object) =>
  Object.fromEntries(Object.entries(object).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));

function escapeStrings(text) {
  return text.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n');
}

function convertInfoPlist(catalog) {
  const result = Object.fromEntries(LANGS.map((l) => [l, {}]));
  for (const [key, entry] of Object.entries(catalog.strings ?? {})) {
    for (const lang of LANGS) {
      const node = resolveNode(entry.localizations?.[lang]);
      if (node?.text !== undefined) result[lang][key] = node.text;
      else if (lang === 'en') result[lang][key] = key;
    }
  }
  return result;
}

function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`error: ${error.message}`);
    process.exit(2);
  }
  const catalog = JSON.parse(readFileSync(args.in, 'utf8'));
  const { result, dropped, missing } = convertLocalizable(catalog);
  if (args.strict && missing.length > 0) {
    console.error(`error: ${missing.length} missing translations (--strict):`);
    for (const line of missing) console.error(`  ${line}`);
    process.exit(1);
  }
  mkdirSync(args.out, { recursive: true });
  for (const lang of LANGS) {
    let entries = result[lang];
    const file = join(args.out, `${lang}.json`);
    if (args.merge && existsSync(file)) {
      entries = { ...entries, ...JSON.parse(readFileSync(file, 'utf8')) };
    }
    writeFileSync(file, `${JSON.stringify(sortObject(entries), null, 2)}\n`);
  }
  const infoplistIn =
    args.infoplistIn ?? (existsSync(join(dirname(args.in), 'InfoPlist.xcstrings'))
      ? join(dirname(args.in), 'InfoPlist.xcstrings')
      : null);
  if (infoplistIn) {
    const defaultOut = resolve(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri/resources/macos');
    const outRoot = args.infoplistOut ?? defaultOut;
    const strings = convertInfoPlist(JSON.parse(readFileSync(infoplistIn, 'utf8')));
    for (const lang of LANGS) {
      const dir = join(outRoot, `${lang}.lproj`);
      mkdirSync(dir, { recursive: true });
      const body = Object.entries(sortObject(strings[lang]))
        .map(([k, v]) => `"${escapeStrings(k)}" = "${escapeStrings(v)}";\n`)
        .join('');
      writeFileSync(join(dir, 'InfoPlist.strings'), body);
    }
  }
  const counts = LANGS.map((l) => `${l}=${Object.keys(result[l]).length}`).join(' ');
  console.log(`converted: ${counts}`);
  console.log(`dropped: ${dropped.length}`);
  for (const key of dropped) console.log(`  ${JSON.stringify(key)}`);
  if (missing.length > 0) {
    console.log(`missing (not strict): ${missing.length}`);
    for (const line of missing) console.log(`  ${line}`);
  }
}

main();
