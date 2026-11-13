// The popup, the content script, and the service worker must call the same
// function, not three copies that can drift. Re-exports keep the old require()
// paths working; this file checks those re-exports are the shared function.

const assert = require('node:assert');

const urlMatch = require('../shared/url-match.js');
const bridgeStatus = require('../shared/bridge-status.js');
const base32 = require('../shared/base32.js');
const formDetection = require('../shared/form-detection.js');
const totpDetection = require('../shared/totp-detection.js');
const content = require('../content.js');
const popup = require('../popup.js');

global.chrome = {
  runtime: { onMessage: { addListener() {} } },
};
const background = require('../background.js');

assert.strictEqual(content.hostnameMatches, urlMatch.hostnameMatches);
assert.strictEqual(background.hostnameMatches, urlMatch.hostnameMatches);
assert.strictEqual(content.classifyGetCredentialsResponse, bridgeStatus.classifyGetCredentialsResponse);
assert.strictEqual(popup.classifyGetCredentialsResponse, bridgeStatus.classifyGetCredentialsResponse);
assert.strictEqual(content.isBase32Secret, base32.isBase32Secret);
assert.strictEqual(totpDetection.isBase32Secret, base32.isBase32Secret);
assert.strictEqual(content.findFillableFields, formDetection.findFillableFields);
assert.strictEqual(content.findLoginForm, formDetection.findLoginForm);
assert.strictEqual(content.scoreUsernameLikeness, formDetection.scoreUsernameLikeness);
assert.strictEqual(content.detectManualSetupKey, totpDetection.detectManualSetupKey);
assert.strictEqual(content.decodeQrFromImage, totpDetection.decodeQrFromImage);
assert.strictEqual(content.detectTotpCandidateWithPriority, totpDetection.detectTotpCandidateWithPriority);
assert.strictEqual(content.shouldShowTotpBanner, totpDetection.shouldShowTotpBanner);

assert.strictEqual(popup.isBase32PasteSecret('jbsw y3dp ehpk 3pxp'), true);
assert.strictEqual(base32.isBase32Secret('jbsw y3dp ehpk 3pxp'), true);
assert.strictEqual(popup.isBase32PasteSecret('short'), false);
assert.strictEqual(urlMatch.hostnameMatches('https://example.com/login', 'https://example.com/account'), true);
assert.strictEqual(urlMatch.hostnameMatches('https://example.com/', ''), false);

const contentSource = require('node:fs').readFileSync(require('node:path').join(__dirname, '..', 'content.js'), 'utf8');
assert.ok(contentSource.includes('formDetection: mypassFormDetection'));
assert.ok(
  !/globalThis\s*\./.test(contentSource),
  'content.js must not read shared helpers off the page global: findLoginForm is not a function there'
);

console.log('sharedModules.test.js: one function per shared helper OK');
