// Completeness checks over the committed catalogs: src/lib/i18n/locales/{en,uk,ru}.json and
// src-tauri/resources/macos/*.lproj/InfoPlist.strings. Literal Cyrillic is not allowed in test
// files, so nothing here spells a translation out; expectations are read from the catalogs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const load = (lang) =>
  JSON.parse(readFileSync(join(root, 'src/lib/i18n/locales', `${lang}.json`), 'utf8'));
const en = load('en');
const uk = load('uk');
const ru = load('ru');
const CYRILLIC = /[\u0400-\u052F\u1C80-\u1C8F\u2DE0-\u2DFF\uA640-\uA69F]/;

const list = (keys) => keys.slice(0, 20).map((k) => JSON.stringify(k)).join(', ');

// Placeholder names of an ICU message: {0}, {1}, {count}, {n}; quoted spans ('{x}') are literal.
function placeholders(message) {
  const unquoted = message.replace(/'[{}][^']*'/g, '').replace(/'[{}]/g, '');
  const names = new Set();
  for (const match of unquoted.matchAll(/\{\s*(\w+)\s*[,}]/g)) {
    if (/^(\d+|count|n)$/.test(match[1])) names.add(match[1]);
  }
  return [...names].sort().join(',');
}

test('every_key_has_uk_and_ru', () => {
  for (const [name, catalog] of [['uk', uk], ['ru', ru]]) {
    const missing = Object.keys(en).filter((k) => !catalog[k]);
    assert.deepEqual(missing, [], `${name} is missing ${missing.length} keys, e.g. ${list(missing)}`);
  }
});

test('no_extra_keys_in_translations', () => {
  for (const [name, catalog] of [['uk', uk], ['ru', ru]]) {
    const extra = Object.keys(catalog).filter((k) => !(k in en));
    assert.deepEqual(extra, [], `${name} has ${extra.length} keys that en lacks, e.g. ${list(extra)}`);
  }
});

test('placeholder_sets_match_across_languages', () => {
  const bad = Object.keys(en).filter(
    (k) => placeholders(en[k]) !== placeholders(uk[k] ?? '') || placeholders(en[k]) !== placeholders(ru[k] ?? ''),
  );
  assert.deepEqual(bad, [], `placeholder mismatch in ${bad.length} keys, e.g. ${list(bad)}`);
});

test('plural_other_branch_always_present', () => {
  for (const [name, catalog] of [['en', en], ['uk', uk], ['ru', ru]]) {
    const bad = Object.keys(catalog).filter(
      (k) => /\{\w+,\s*plural,/.test(catalog[k]) && !/\bother\s*\{/.test(catalog[k]),
    );
    assert.deepEqual(bad, [], `${name}: plural without an other branch, e.g. ${list(bad)}`);
  }
});

test('catalog_has_expected_scale', () => {
  assert.ok(Object.keys(en).length >= 450, `en has only ${Object.keys(en).length} keys`);
});

test('no_cyrillic_in_english_catalog', () => {
  const bad = Object.entries(en).filter(([k, v]) => CYRILLIC.test(k) || CYRILLIC.test(v));
  assert.deepEqual(bad.map(([k]) => k), []);
});

test('translations_are_not_identical_to_english_for_ui_words', () => {
  const words = [
    'Lock', 'Settings', 'Delete', 'Cancel', 'Search', 'Favorites', 'Notes', 'Files', 'Passkeys',
    'Cards', 'Addresses', 'Codes', 'Security', 'Deleted', 'Appearance', 'General', 'Vault',
    'Browser', 'About', 'Backups',
  ];
  const same = [];
  for (const word of words) {
    assert.ok(word in en, `spot-list key ${word} is missing from en`);
    for (const [name, catalog] of [['uk', uk], ['ru', ru]]) {
      if (catalog[word] === en[word]) same.push(`${name}:${word}`);
    }
  }
  assert.deepEqual(same, []);
});

test('tray_and_dialog_keys_exist_in_all_languages', () => {
  const keys = ['Unlock', 'Lock', 'Open MyPass', 'Settings…', 'Quit', 'Quit Anyway', 'Cancel'];
  for (const [name, catalog] of [['en', en], ['uk', uk], ['ru', ru]]) {
    const missing = keys.filter((k) => !catalog[k]);
    assert.deepEqual(missing, [], `${name} lacks tray keys`);
  }
});

function parseStrings(lang) {
  const text = readFileSync(
    join(root, 'src-tauri/resources/macos', `${lang}.lproj`, 'InfoPlist.strings'),
    'utf8',
  );
  const entries = {};
  for (const match of text.matchAll(/^"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)";$/gm)) {
    entries[match[1]] = match[2];
  }
  return entries;
}

test('infoplist_strings_exist_for_en_uk_ru_and_include_face_id_description', () => {
  const sets = ['en', 'uk', 'ru'].map(parseStrings);
  for (const entries of sets) {
    assert.ok(entries.NSFaceIDUsageDescription?.length > 0, 'NSFaceIDUsageDescription missing');
  }
  const keySets = sets.map((s) => Object.keys(s).sort().join('|'));
  assert.equal(new Set(keySets).size, 1, 'key sets differ across languages');
});
