const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const generator = require('../shared/password-generator.js');
const create = require('../create.js');

assert.strictEqual(generator.defaultGeneratorSettings().type, 'smart');
assert.strictEqual(generator.defaultGeneratorSettings().useAsDefault, false);
assert.strictEqual(generator.generateFromSettings(generator.defaultGeneratorSettings()), '');

const pin = generator.normalizeGeneratorSettings({ type: 'pin', useAsDefault: 1, pin: { length: 99 } });
assert.strictEqual(pin.type, 'pin');
assert.strictEqual(pin.useAsDefault, false);
assert.strictEqual(pin.pin.length, 12);
assert.match(generator.generateFromSettings({ type: 'pin', useAsDefault: true, pin: { length: 4 } }), /^\d{4}$/);

const random = generator.generateRandomPassword({ length: 20, digits: true, symbols: true });
assert.strictEqual(random.length, 20);
assert.match(random, /[a-z]/);
assert.match(random, /[A-Z]/);
assert.match(random, /\d/);
assert.match(random, /[^a-zA-Z0-9]/);
assert.strictEqual(generator.generateRandomPassword({ length: 1, digits: false, symbols: false }).length, 8);
assert.strictEqual(generator.generateRandomPassword({ length: 100 }).length, 64);
const letters = generator.generateRandomPassword({ length: 24, digits: false, symbols: false });
assert.match(letters, /^[a-zA-Z]+$/);

const memorable = generator.generateMemorablePassword({
  words: 4, separator: 'hyphen', capitalize: false, fullWords: true
});
assert.strictEqual(memorable.split('-').length, 4);
assert.strictEqual(memorable, memorable.toLowerCase());
const capitalized = generator.generateMemorablePassword({
  words: 3, separator: 'space', capitalize: true, fullWords: true
});
assert.strictEqual(capitalized.split(' ').length, 3);
assert.ok(capitalized.split(' ').every((word) => word[0] === word[0].toUpperCase()));
const shortWords = generator.generateMemorablePassword({
  words: 4, separator: 'underscore', capitalize: false, fullWords: false
});
assert.ok(shortWords.split('_').every((word) => word.length >= 3 && word.length <= 4));

assert.strictEqual(generator.passwordStrength(''), 0);
assert.ok(generator.passwordStrength('1234') < 0.4);
assert.strictEqual(generator.passwordStrength(random), 1);
assert.strictEqual(generator.strengthColor(0), 'rgb(229, 72, 77)');
assert.strictEqual(generator.strengthColor(1), 'rgb(48, 164, 108)');

assert.strictEqual(generator.historyHost('https://bank.example/signup'), 'bank.example');
assert.strictEqual(generator.historyHost('chrome://extensions/'), '');
const createdAt = 1700000000000;
const entries = [
  { id: 'a', password: 'Secret', url: 'https://gitlab.com/login', createdAt: createdAt },
  { id: 'b', password: 'Other', url: 'https://example.com', createdAt: createdAt + 86400000 * 40 }
];
const groups = generator.groupPasswordHistory(entries);
assert.strictEqual(groups.length, 2);
assert.strictEqual(groups[0].label, generator.formatHistoryDay(createdAt));
assert.strictEqual(generator.filterPasswordHistory(entries, 'gitlab').length, 1);
assert.strictEqual(generator.filterPasswordHistory(entries, 'secret').length, 1);
assert.strictEqual(generator.filterPasswordHistory(entries, 'nope').length, 0);

assert.strictEqual(create.formatCardNumber('4242424242424242'), '4242 4242 4242 4242');
assert.strictEqual(create.digitsOnly('4242 4242'), '42424242');
assert.strictEqual(create.formatExpiry('1228'), '12/28');
assert.strictEqual(create.formatExpiry('12'), '12');
const wrapped = create.wrapMarkdown('hello world', 6, 11, '**', '**');
assert.strictEqual(wrapped.value, 'hello **world**');
assert.strictEqual(create.describeCreateError({ type: 'locked' }), 'MyPass is locked.');
assert.strictEqual(create.describeCreateError({ message: 'title required' }), 'Title is required.');
assert.ok(create.MATCH_HELP['registrable-domain'].includes('example.com'));
assert.strictEqual(create.INSERT_PASSWORD_KEY, 'insertGeneratedPassword');

const extensionDir = path.join(__dirname, '..');
const generatorHtml = fs.readFileSync(path.join(extensionDir, 'generator.html'), 'utf8');
const generatorJs = fs.readFileSync(path.join(extensionDir, 'generator.js'), 'utf8');
const createHtml = fs.readFileSync(path.join(extensionDir, 'create.html'), 'utf8');
[
  'Copy', 'Autofill', 'Smart password', 'Random password', 'Memorable password', 'PIN',
  'Characters', 'Digits', 'Symbols', 'Words', 'Separator', 'Hyphens', 'Spaces', 'Periods',
  'Commas', 'Underscores', 'Capitalize', 'Full words', 'Use by default for suggestions',
  'Password generator history', 'Generator history', 'Search history'
].forEach((label) => {
  assert.ok(generatorHtml.includes(label), 'generator window is missing ' + label);
});
assert.ok(generatorJs.includes("textContent = 'Show'") || generatorJs.includes('Show'));
assert.ok(generatorJs.includes("textContent = 'Copy'"));
assert.ok(generatorJs.includes("textContent = 'Delete'"));
assert.ok(generatorJs.includes('insertGeneratedPassword'));
assert.ok(generatorJs.includes('••••••••'));
assert.ok(generatorJs.includes('show-history'), 'history opens by sliding over the generator');
assert.ok(!generatorJs.includes('generatorView.hidden'), 'the generator stays under the history screen');
assert.ok(generatorHtml.includes('show-history'));
assert.ok(generatorHtml.includes('display: none !important'));
assert.ok(generatorHtml.includes('translateX(100%)'));
assert.ok(createHtml.includes('display: none !important'));
assert.ok(createHtml.includes('#notes-editor:not([hidden])'));
assert.ok(createHtml.includes('appearance: none'));
assert.ok(/#notes-title\s*\{[^}]*flex:\s*0/.test(createHtml), 'the note title stays one line');
assert.ok(createHtml.includes('class="notes-card"'));
['New Entry', 'Title', 'Username', 'Password', 'Generate', 'URL', 'Match', 'Notes',
  '+ Add one-time password', 'Cancel', 'Save', 'New Card', 'Card Number', 'Cardholder Name',
  'Expiry Date (MM/YY)', 'CVV', 'New Note'
].forEach((label) => {
  assert.ok(createHtml.includes(label) || fs.readFileSync(path.join(extensionDir, 'create.js'), 'utf8').includes(label), label);
});

console.log('passwordGenerator.test.js: OK');
