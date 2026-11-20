// The extension's text lives in `_locales/<lang>/messages.json` (chrome.i18n). This suite keeps
// the catalogs honest: valid files, one key set and one placeholder set across languages, every
// key the code, the pages and the manifest reference defined, no key left unused, and no
// user-visible English literal back in the UI files.

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const LANGUAGES = ['en', 'uk', 'ru', 'de'];

function read(relative) {
  return fs.readFileSync(path.join(root, relative), 'utf8');
}

const manifest = JSON.parse(read('manifest.json'));
assert.strictEqual(manifest.default_locale, 'en', 'English is the default locale');
assert.deepStrictEqual(
  fs.readdirSync(path.join(root, '_locales')).sort(),
  LANGUAGES.slice().sort(),
  'the shipped locales are exactly en, uk, ru, de'
);

// ---- catalog shape ---------------------------------------------------------------------------
const catalogs = {};
for (const language of LANGUAGES) {
  const file = path.join('_locales', language, 'messages.json');
  catalogs[language] = JSON.parse(read(file));
  for (const [key, entry] of Object.entries(catalogs[language])) {
    const where = `${language}/${key}`;
    assert.match(key, /^[A-Za-z0-9_]+$/, `${where}: key name`);
    assert.strictEqual(typeof entry.message, 'string', `${where}: message`);
    assert.ok(entry.message.trim().length > 0, `${where}: message is empty`);
    assert.ok(
      Object.keys(entry).every((field) => ['message', 'description', 'placeholders'].includes(field)),
      `${where}: unknown field`
    );
    // A lone `$` would be read as a placeholder; a literal dollar must be `$$`.
    assert.ok(
      !entry.message.replace(/\$\$/g, '').replace(/\$[A-Za-z0-9_@]+\$/g, '').includes('$'),
      `${where}: stray $`
    );
  }
}

const english = catalogs.en;
const englishKeys = Object.keys(english).sort();
assert.ok(englishKeys.length > 100, 'the English catalog is populated');

function placeholdersOf(entry) {
  const used = new Set();
  for (const match of entry.message.replace(/\$\$/g, '').matchAll(/\$([A-Za-z0-9_@]+)\$/g)) {
    used.add(match[1].toLowerCase());
  }
  const defined = Object.keys(entry.placeholders || {}).map((name) => name.toLowerCase());
  return { used: [...used].sort(), defined: defined.sort() };
}

for (const language of LANGUAGES) {
  assert.deepStrictEqual(Object.keys(catalogs[language]).sort(), englishKeys, `${language} has exactly the English keys`);
  for (const key of englishKeys) {
    const entry = catalogs[language][key];
    const { used, defined } = placeholdersOf(entry);
    assert.deepStrictEqual(used, defined, `${language}/${key}: every placeholder used is defined, and the reverse`);
    assert.deepStrictEqual(used, placeholdersOf(english[key]).used, `${language}/${key}: same placeholders as English`);
    for (const [name, placeholder] of Object.entries(entry.placeholders || {})) {
      assert.match(placeholder.content, /^\$[1-9]$/, `${language}/${key}/${name}: placeholder content is $1..$9`);
      assert.deepStrictEqual(
        placeholder,
        english[key].placeholders[name],
        `${language}/${key}/${name}: same placeholder content as English`
      );
    }
  }
}

// uk and ru are real translations: any message with an English word has Cyrillic in it.
const cyrillic = /[\u0400-\u04FF]/;
for (const key of englishKeys) {
  assert.ok(!cyrillic.test(english[key].message), `en/${key} has no Cyrillic`);
  assert.ok(!cyrillic.test(catalogs.de[key].message), `de/${key} has no Cyrillic`);
  if (/[a-z]{4,}/.test(english[key].message.replace(/MyPass|otpauth|Chrome|macOS|Google|Passkey/g, ''))) {
    for (const language of ['uk', 'ru']) {
      assert.ok(cyrillic.test(catalogs[language][key].message), `${language}/${key} is translated`);
    }
  }
}

// ---- references ------------------------------------------------------------------------------
function listFiles(directory, extension, skip = []) {
  return fs.readdirSync(path.join(root, directory), { withFileTypes: true })
    .filter((item) => item.isFile() && item.name.endsWith(extension) && !skip.includes(item.name))
    .map((item) => path.posix.join(directory, item.name));
}

const scripts = [
  ...listFiles('.', '.js', ['background-loader.js']),
  ...listFiles('shared', '.js', ['i18n.js']),
  ...listFiles('app', '.js'),
];
const pages = ['popup.html', 'create.html', 'generator.html', 'prompt.html', 'browser-check.html', 'app/app.html'];

const referenced = new Map(); // key -> where it is referenced
function reference(key, where) {
  if (!referenced.has(key)) referenced.set(key, where);
}

// `t('key')`, `t(cond ? 'a' : 'b')` and `labelKey: 'key'`: collect the quoted names inside the call.
for (const file of scripts) {
  const source = read(file);
  for (const match of source.matchAll(/\bt\(([^()]*)\)/g)) {
    // After a `?` only: the condition's own literals (`kind === 'create'`) are not keys.
    const first = match[1].split(',')[0];
    const choices = first.includes('?') ? first.slice(first.indexOf('?') + 1) : first;
    for (const literal of choices.matchAll(/['"]([A-Za-z][A-Za-z0-9_]*)['"]/g)) reference(literal[1], file);
  }
  for (const match of source.matchAll(/\b[A-Za-z]*Key:\s*['"]([A-Za-z0-9_]+)['"]/g)) reference(match[1], file);
}
for (const file of pages) {
  const source = read(file);
  for (const match of source.matchAll(/data-i18n(?:-[a-z-]+)?="([^"]+)"/g)) reference(match[1], file);
}
for (const match of JSON.stringify(manifest).matchAll(/__MSG_([A-Za-z0-9_]+)__/g)) reference(match[1], 'manifest.json');

for (const [key, where] of referenced) {
  assert.ok(english[key], `${where} references the missing message ${key}`);
}

// Keys picked through a variable (`const key = cond ? 'a' : 'b'; t(key)`) count as used when their
// name appears quoted anywhere in the sources.
const everything = [...scripts, ...pages, 'manifest.json'].map(read).join('\n');
for (const key of englishKeys) {
  assert.ok(
    referenced.has(key) || everything.includes(`'${key}'`) || everything.includes(`"${key}"`),
    `the message ${key} is not used anywhere`
  );
}

// ---- no English left in the UI files ---------------------------------------------------------
const LETTERS = /[A-Za-z]{2,}/;

for (const file of pages) {
  const markup = read(file).replace(/<(style|script)\b[\s\S]*?<\/\1>/gi, '');
  const texts = [...markup.matchAll(/>([^<>]*[A-Za-z]{2,}[^<>]*)</g)]
    .map((match) => match[1].trim())
    .filter((text) => text && text !== '`code`'); // the Markdown "code" button label is a literal
  assert.deepStrictEqual(texts, [], `${file}: visible text must come from the catalog (data-i18n)`);
  for (const match of markup.matchAll(/\s(placeholder|title|aria-label|alt)="([^"]*)"/g)) {
    assert.ok(!LETTERS.test(match[2]) || match[2] === 'MyPass', `${file}: ${match[1]}="${match[2]}" must use data-i18n-${match[1]}`);
  }
}

// A string literal assigned to a visible-text property is a missed translation. Only whitelisted
// non-linguistic values are allowed.
const ALLOWED_LITERALS = new Set(['MyPass', 'MyPass,', '', ' ', 'MM/YY', 'CVV', 'PIN', '2FA']);
const VISIBLE = /(?:\.(?:textContent|innerText|placeholder|title|ariaLabel)\s*=|setAttribute\(\s*['"](?:aria-label|title|placeholder)['"]\s*,)\s*(['"`])((?:\\.|(?!\1)[^\\])*)\1/g;
for (const file of scripts) {
  const lines = read(file).split('\n');
  lines.forEach((line, index) => {
    if (/^\s*\/\//.test(line)) return;
    for (const match of line.matchAll(VISIBLE)) {
      const literal = match[2];
      if (!LETTERS.test(literal) || ALLOWED_LITERALS.has(literal)) continue;
      assert.fail(`${file}:${index + 1} sets visible text from an English literal: ${literal}`);
    }
  });
}

// ---- the helper itself -----------------------------------------------------------------------
const i18n = require('../shared/i18n.js');
assert.strictEqual(i18n.t('extName'), 'MyPass');
assert.strictEqual(i18n.t('no_such_key'), 'no_such_key', 'an unknown key answers with the key');
assert.strictEqual(
  i18n.expand({ message: 'Cost $$5 for $NAME$', placeholders: { name: { content: '$1' } } }, ['x']),
  'Cost $5 for x'
);

console.log(`locales.test.js: ${englishKeys.length} messages x ${LANGUAGES.length} languages OK`);
