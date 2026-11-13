const assert = require('node:assert');
const watch = require('../shared/security-watch.js');

const now = 1_000_000;
const filled = watch.rememberFilledCredential([], {
  type: 'credential',
  username: 'alice',
  password: 's3cret-password',
  urls: ['https://bank.example', 'https://pay.bank.example']
}, 'https://www.bank.example/login', now);

assert.deepStrictEqual(filled[0].hosts.sort(), ['bank.example', 'pay.bank.example', 'www.bank.example'].sort());

const pasteOnPhish = watch.evaluateSecurityUse(null, filled, 'paste', 's3cret-password', 'https://evil.example/login', now);
assert.strictEqual(pasteOnPhish.warn, true);

const pasteOnSaved = watch.evaluateSecurityUse(null, filled, 'paste', 's3cret-password', 'https://login.bank.example/signin', now);
assert.strictEqual(pasteOnSaved.warn, false, 'a subdomain of a saved website is still that item');

const copyLogin = watch.evaluateSecurityUse(null, filled, 'copy', 'alice', 'https://evil.example', now);
assert.strictEqual(copyLogin.warn, true);

const copyOnSaved = watch.evaluateSecurityUse(null, filled, 'copy', 'alice', 'https://bank.example/account', now);
assert.strictEqual(copyOnSaved.warn, false);

const usernamePaste = watch.evaluateSecurityUse(null, filled, 'paste', 'alice', 'https://evil.example', now);
assert.strictEqual(usernamePaste.warn, false, 'paste warning is for the password');

const pasteOff = watch.evaluateSecurityUse({ warnPaste: false, warnCopy: true }, filled, 'paste', 's3cret-password', 'https://evil.example', now);
assert.strictEqual(pasteOff.warn, false);
const copyOff = watch.evaluateSecurityUse({ warnPaste: true, warnCopy: false }, filled, 'copy', 's3cret-password', 'https://evil.example', now);
assert.strictEqual(copyOff.warn, false);

const expired = watch.evaluateSecurityUse(null, filled, 'paste', 's3cret-password', 'https://evil.example', now + watch.SECURITY_WATCH_TTL_MS + 1);
assert.strictEqual(expired.warn, false);
assert.deepStrictEqual(expired.watches, []);

const generated = watch.rememberFilledCredential([], { type: 'generatedPassword', password: 's3cret-password' }, 'https://bank.example', now);
assert.deepStrictEqual(generated, [], 'a generated password is not an item yet');

assert.strictEqual(watch.textFromCopyTarget('s3cret-password', 0, 15, ''), 's3cret-password');
assert.strictEqual(watch.textFromCopyTarget('ada', 1, 1, 'selected'), 'selected');

console.log('securityWatch.test.js: OK');
