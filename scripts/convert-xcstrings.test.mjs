// Tests for scripts/convert-xcstrings.mjs. Each test writes a tiny xcstrings catalog into a temp
// directory, runs the converter as a child process and reads back what it wrote. Literal Cyrillic
// is not allowed in test files, so non-Latin expectations use \u escapes.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const script = join(dirname(fileURLToPath(import.meta.url)), 'convert-xcstrings.mjs');

const unit = (value, state = 'translated') => ({ stringUnit: { state, value } });
const loc = (en, uk, ru) => ({
  localizations: { uk: unit(uk ?? en), ru: unit(ru ?? en) },
});

function catalog(strings) {
  return { sourceLanguage: 'en', strings, version: '1.0' };
}

function run(strings, extraArgs = [], { before } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'xcs-'));
  const input = join(dir, 'Localizable.xcstrings');
  const out = join(dir, 'locales');
  mkdirSync(out);
  writeFileSync(input, JSON.stringify(catalog(strings)));
  if (before) before(out);
  const result = spawnSync(process.execPath, [script, '--in', input, '--out', out, ...extraArgs], {
    encoding: 'utf8',
  });
  const read = (lang) => {
    const file = join(out, `${lang}.json`);
    return existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : null;
  };
  return { dir, out, result, read };
}

test('positional_specifiers_become_indexed_placeholders', () => {
  const key = 'Hello %@, you have %lld items';
  const { read, result } = run({ [key]: loc(key, 'U %@ N %lld', 'R %@ N %lld') });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(read('en')[key], 'Hello {0}, you have {1} items');
  assert.equal(read('uk')[key], 'U {0} N {1}');
});

test('explicit_index_specifier_honoured', () => {
  const key = '%2$@ then %1$@';
  const { read } = run({ [key]: loc(key) });
  assert.equal(read('en')[key], '{1} then {0}');
});

test('percent_percent_is_literal_percent', () => {
  const key = '100%% sure';
  const { read } = run({ [key]: loc(key) });
  assert.equal(read('en')[key], '100% sure');
});

test('braces_are_icu_escaped', () => {
  const key = 'Use {braces}';
  const { read } = run({ [key]: loc(key) });
  assert.equal(read('en')[key], "Use '{braces}'");
});

test('plural_variations_become_icu_plural', () => {
  const key = '%lld files';
  const plural = (forms) => ({
    variations: {
      plural: Object.fromEntries(Object.entries(forms).map(([k, v]) => [k, unit(v)])),
    },
  });
  const { read, result } = run({
    [key]: {
      localizations: {
        en: plural({ one: '%lld file', other: '%lld files' }),
        uk: plural({ one: '%lld a', few: '%lld b', many: '%lld c', other: '%lld d' }),
        ru: plural({ one: '%lld a', few: '%lld b', many: '%lld c', other: '%lld d' }),
      },
    },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(read('en')[key], '{count, plural, one {# file} other {# files}}');
  assert.equal(read('uk')[key], '{count, plural, one {# a} few {# b} many {# c} other {# d}}');
});

test('plural_keeps_other_specifiers_positional', () => {
  const key = '%@ has %lld files';
  const plural = (forms) => ({
    variations: {
      plural: Object.fromEntries(Object.entries(forms).map(([k, v]) => [k, unit(v)])),
    },
  });
  const forms = { one: '%@ has %lld file', other: '%@ has %lld files' };
  const { read } = run({
    [key]: { localizations: { en: plural(forms), uk: plural(forms), ru: plural(forms) } },
  });
  assert.equal(
    read('en')[key],
    '{count, plural, one {{0} has # file} other {{0} has # files}}',
  );
});

test('device_variation_uses_mac_branch', () => {
  const key = 'Tap here';
  const { read } = run({
    [key]: {
      localizations: {
        en: {
          variations: {
            device: { iphone: unit('Tap'), ipad: unit('Touch'), mac: unit('Click here') },
          },
        },
        uk: unit('Ukr'),
        ru: unit('Rus'),
      },
    },
  });
  assert.equal(read('en')[key], 'Click here');
});

test('untranslated_state_counts_as_missing_and_strict_fails', () => {
  const key = 'Needs work';
  const strings = {
    [key]: {
      localizations: { uk: unit('Ukr'), ru: unit('Rus', 'new') },
    },
  };
  const { result, read } = run(strings, ['--strict']);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Needs work/);
  assert.match(result.stderr, /ru/);
  const lenient = run(strings);
  assert.equal(lenient.result.status, 0, lenient.result.stderr);
  assert.equal(lenient.read('ru')[key], undefined);
  assert.equal(lenient.read('uk')[key], 'Ukr');
  assert.equal(read('uk'), null, 'strict failure writes nothing');
});

test('should_translate_false_keys_are_copied_verbatim', () => {
  const { read, result } = run({ MyPass: { shouldTranslate: false }, '': { shouldTranslate: false } }, [
    '--strict',
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(read('uk').MyPass, 'MyPass');
  assert.equal(read('ru').MyPass, 'MyPass');
  assert.equal(Object.hasOwn(read('en'), ''), false);
});

test('sftp_keys_dropped_and_listed', () => {
  const { result, read } = run({
    'Connect to SFTP server': loc('Connect to SFTP server'),
    'Choose Private Key…': loc('Choose Private Key…'),
    'Trust this host key?': loc('Trust this host key?'),
    Lock: loc('Lock'),
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /dropped:/);
  assert.match(result.stdout, /Connect to SFTP server/);
  assert.match(result.stdout, /Choose Private Key/);
  assert.deepEqual(Object.keys(read('en')), ['Lock']);
});

test('merge_keeps_existing_hand_edited_values', () => {
  const { read, result } = run({ Lock: loc('Lock', 'Ukr', 'Rus'), Open: loc('Open', 'Uo', 'Ro') }, ['--merge'], {
    before: (out) => writeFileSync(join(out, 'uk.json'), JSON.stringify({ Lock: 'Hand edited' })),
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(read('uk').Lock, 'Hand edited');
  assert.equal(read('uk').Open, 'Uo');
  assert.equal(read('ru').Lock, 'Rus');
});

test('output_is_sorted_lf_two_space_json', () => {
  const { out } = run({ Zeta: loc('Zeta'), Alpha: loc('Alpha'), Mid: loc('Mid') });
  const text = readFileSync(join(out, 'en.json'), 'utf8');
  assert.equal(text, '{\n  "Alpha": "Alpha",\n  "Mid": "Mid",\n  "Zeta": "Zeta"\n}\n');
  assert.ok(!text.includes('\r'));
});

test('infoplist_strings_written_per_language', () => {
  const dir = mkdtempSync(join(tmpdir(), 'xcs-ip-'));
  const input = join(dir, 'Localizable.xcstrings');
  const plist = join(dir, 'InfoPlist.xcstrings');
  const out = join(dir, 'locales');
  const strOut = join(dir, 'macos');
  mkdirSync(out);
  writeFileSync(input, JSON.stringify(catalog({ Lock: loc('Lock') })));
  writeFileSync(
    plist,
    JSON.stringify(
      catalog({
        NSFaceIDUsageDescription: {
          localizations: {
            en: unit('Use "Touch ID" to unlock.'),
            uk: unit('\u0422\u0435\u0441\u0442'),
            ru: unit('\u0422\u0435\u0441\u0442 2'),
          },
        },
      }),
    ),
  );
  const result = spawnSync(
    process.execPath,
    [script, '--in', input, '--out', out, '--infoplist-in', plist, '--infoplist-out', strOut],
    { encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stderr);
  const en = readFileSync(join(strOut, 'en.lproj', 'InfoPlist.strings'), 'utf8');
  assert.equal(en, '"NSFaceIDUsageDescription" = "Use \\"Touch ID\\" to unlock.";\n');
  const uk = readFileSync(join(strOut, 'uk.lproj', 'InfoPlist.strings'), 'utf8');
  assert.equal(uk, '"NSFaceIDUsageDescription" = "\u0422\u0435\u0441\u0442";\n');
  assert.ok(existsSync(join(strOut, 'ru.lproj', 'InfoPlist.strings')));
});

// Reference un-escaper for the ICU apostrophe mode the converter targets (DOUBLE_OPTIONAL, the
// default of ICU4J and intl-messageformat). Returns tokens: literal text and unquoted placeholders.
function unescapeIcu(text, { inPlural = false } = {}) {
  const tokens = [];
  let literal = '';
  const flush = () => {
    if (literal) tokens.push({ lit: literal });
    literal = '';
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    const next = text[i + 1];
    if (c === "'") {
      if (next === "'") {
        literal += "'";
        i += 2;
      } else if (next === '{' || next === '}' || (inPlural && next === '#')) {
        i++;
        while (i < text.length) {
          if (text[i] === "'") {
            if (text[i + 1] === "'") {
              literal += "'";
              i += 2;
              continue;
            }
            i++;
            break;
          }
          literal += text[i++];
        }
      } else {
        literal += c;
        i++;
      }
    } else if (c === '{') {
      let depth = 1;
      let j = i + 1;
      while (j < text.length && depth > 0) {
        if (text[j] === '{') depth++;
        else if (text[j] === '}') depth--;
        j++;
      }
      flush();
      tokens.push({ ph: text.slice(i, j) });
      i = j;
    } else if (c === '#' && inPlural) {
      flush();
      tokens.push({ ph: '#' });
      i++;
    } else {
      literal += c;
      i++;
    }
  }
  flush();
  return tokens;
}

function convertEn(key) {
  const { read, result } = run({ [key]: loc(key) });
  assert.equal(result.status, 0, result.stderr);
  return read('en')[key];
}

test('percent_followed_by_space_is_literal', () => {
  assert.equal(convertEn('100% sure'), '100% sure');
  assert.equal(convertEn('A 50% off'), 'A 50% off');
  assert.equal(convertEn('Rate 5%'), 'Rate 5%');
  assert.equal(convertEn('Pi is %.2f'), 'Pi is {0}');
  assert.equal(convertEn('%1$lld of %2$lld'), '{0} of {1}');
});

test('apostrophe_alone_stays_literal', () => {
  const out = convertEn("Don't");
  assert.equal(out, "Don't");
  assert.deepEqual(unescapeIcu(out), [{ lit: "Don't" }]);
});

test('apostrophe_after_braces_round_trips', () => {
  const out = convertEn("{a}'s");
  assert.equal(out, "'{a}'''s");
  assert.deepEqual(unescapeIcu(out), [{ lit: "{a}'s" }]);
});

test('apostrophe_before_literal_braces', () => {
  const out = convertEn("it's {x}");
  assert.equal(out, "it's '{x}'");
  assert.deepEqual(unescapeIcu(out), [{ lit: "it's {x}" }]);
});

test('quoted_word_round_trips', () => {
  const out = convertEn("'quoted'");
  assert.equal(out, "'quoted'");
  assert.deepEqual(unescapeIcu(out), [{ lit: "'quoted'" }]);
});

test('apostrophe_before_placeholder_is_doubled', () => {
  const out = convertEn("Bob'%@");
  assert.equal(out, "Bob''{0}");
  assert.deepEqual(unescapeIcu(out), [{ lit: "Bob'" }, { ph: '{0}' }]);
  const spaced = convertEn("Bob' %@");
  assert.equal(spaced, "Bob' {0}");
  assert.deepEqual(unescapeIcu(spaced), [{ lit: "Bob' " }, { ph: '{0}' }]);
});

test('literal_hash_in_plural_branch_round_trips', () => {
  const key = '%lld items';
  const plural = (forms) => ({
    variations: {
      plural: Object.fromEntries(Object.entries(forms).map(([k, v]) => [k, unit(v)])),
    },
  });
  const forms = { one: "%lld item #1's", other: "%lld items #'2" };
  const { read, result } = run({
    [key]: { localizations: { en: plural(forms), uk: plural(forms), ru: plural(forms) } },
  });
  assert.equal(result.status, 0, result.stderr);
  const out = read('en')[key];
  assert.equal(out, "{count, plural, one {# item '#'1's} other {# items '#'''2}}");
  const match = /one \{(.*)\} other \{(.*)\}\}$/.exec(out);
  assert.deepEqual(unescapeIcu(match[1], { inPlural: true }), [
    { ph: '#' },
    { lit: " item #1's" },
  ]);
  assert.deepEqual(unescapeIcu(match[2], { inPlural: true }), [
    { ph: '#' },
    { lit: " items #'2" },
  ]);
});

test('mixed_implicit_and_explicit_indexes', () => {
  assert.equal(convertEn('Mixed %@ and %2$@'), 'Mixed {0} and {1}');
  assert.equal(convertEn('%2$@ and %@'), '{1} and {0}');
});
