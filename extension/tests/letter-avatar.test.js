const assert = require('node:assert');

const {
  letterFromTitle,
  colorForTitle,
  LETTER_AVATAR_PALETTE
} = require('../shared/entry-visuals.js');

assert.strictEqual(letterFromTitle('GitHub'), 'G');
assert.strictEqual(letterFromTitle('amazon'), 'A');
assert.strictEqual(letterFromTitle(''), '?');
assert.strictEqual(letterFromTitle('   '), '?');

assert.strictEqual(colorForTitle('GitHub'), '#A2845E');
assert.strictEqual(colorForTitle('amazon'), '#5856D6');
assert.strictEqual(colorForTitle('Example Service'), '#30B0C7');
assert.strictEqual(colorForTitle('Alpha'), '#5856D6');
assert.strictEqual(colorForTitle('Beta'), '#007AFF');
assert.strictEqual(colorForTitle('MyPass Vault Entry'), '#00C7BE');
assert.strictEqual(colorForTitle('Very Long Service Name For Testing'), '#00C7BE');
assert.notStrictEqual(colorForTitle('Alpha'), colorForTitle('Beta'));

assert.strictEqual(LETTER_AVATAR_PALETTE.length, 10);
assert.deepStrictEqual(LETTER_AVATAR_PALETTE, [
  '#007AFF',
  '#34C759',
  '#FF9500',
  '#AF52DE',
  '#FF2D55',
  '#30B0C7',
  '#5856D6',
  '#00C7BE',
  '#32ADE6',
  '#A2845E'
]);

console.log('letter-avatar.test.js OK');
