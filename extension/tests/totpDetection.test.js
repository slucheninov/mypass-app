const assert = require('node:assert');
const {
  detectOtpauthLink, detectOtpauthText, isBase32Secret, detectManualSetupKey,
  detectTotpCandidateWithPriority, shouldShowTotpBanner, requestTotpPrompt, requestSavePrompt,
  buildTotpCandidateQueryResponse,
} = require('../content.js');

function fakeAnchor(href) {
  return { getAttribute: (name) => (name === 'href' ? href : null) };
}

function fakeDoc({ links = [], bodyText = '' }) {
  return {
    querySelectorAll: (selector) => (selector === 'a[href]' ? links : []),
    body: { textContent: bodyText },
  };
}

// detectOtpauthLink
const linkDoc = fakeDoc({ links: [fakeAnchor('https://example.com'), fakeAnchor('otpauth://totp/Example:alice?secret=JBSWY3DP')] });
assert.strictEqual(detectOtpauthLink(linkDoc), 'otpauth://totp/Example:alice?secret=JBSWY3DP');
assert.strictEqual(detectOtpauthLink(fakeDoc({ links: [fakeAnchor('https://example.com')] })), null);

// detectOtpauthText
const textDoc = fakeDoc({ bodyText: 'Scan this: otpauth://totp/Example:bob?secret=ABCDEFGH please' });
assert.strictEqual(detectOtpauthText(textDoc), 'otpauth://totp/Example:bob?secret=ABCDEFGH');
assert.strictEqual(detectOtpauthText(fakeDoc({ bodyText: 'no link here' })), null);

// isBase32Secret
assert.strictEqual(isBase32Secret('JBSWY3DPEHPK3PXP'), true);
assert.strictEqual(isBase32Secret('jbswy3dp ehpk3pxp'), true, 'lowercase and spaces normalize');
assert.strictEqual(isBase32Secret('short'), false);
assert.strictEqual(isBase32Secret('has1and9digits'), false);

// detectManualSetupKey — uppercase (existing behaviour)
const manualDoc = fakeDoc({ bodyText: 'Your manual setup key is JBSW Y3DP EHPK 3PXP — enter it in your app' });
assert.strictEqual(detectManualSetupKey(manualDoc), 'JBSWY3DPEHPK3PXP');
assert.strictEqual(detectManualSetupKey(fakeDoc({ bodyText: 'JBSWY3DPEHPK3PXP with no label nearby' })), null, 'requires a label keyword present');

// detectManualSetupKey — lowercase keys
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'Your secret is jbswy3dpehpk3pxp — enter it in your app' })),
  'JBSWY3DPEHPK3PXP',
  'single all-lowercase token accepted and normalised to uppercase'
);
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'Your manual setup key is jbsw y3dp ehpk 3pxp — enter it in your app' })),
  'JBSWY3DPEHPK3PXP',
  'space-grouped all-lowercase tokens accepted'
);
// all-letter keys (no base32 digit in key) — single token
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'Your secret is aabbccddaabbccdd — enter it in your app' })),
  'AABBCCDDAABBCCDD',
  'standalone all-letter lowercase key accepted'
);
// all-letter keys — grouped
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'Your manual setup key is aabb ccdd aabb ccdd — enter it in your app' })),
  'AABBCCDDAABBCCDD',
  'space-grouped all-letter lowercase key accepted'
);
// 32-char key in 8 groups of 4; largest-span-first returns the complete key, not a 16-char prefix
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'Your secret is aabb ccdd aabb ccdd eeff gghh iijj kkll' })),
  'AABBCCDDAABBCCDDEEFFGGHHIIJJKKLL',
  '32-char lowercase key in 8 groups of 4 returned complete'
);
// uniform-group guard: letter-only prose must not match even when a label is present —
// no single token ≥16 chars, no uniform-length multi-token span long enough
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'manual setup key information is provided here today in this form' })),
  null,
  'lowercase letter-only prose rejected (varied token lengths fail uniform-group guard)'
);
// uniform-group guard: mixed-length tokens block prose concatenation even when a digit is present
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'manual setup key information about 2factor authentication is provided here' })),
  null,
  'prose with a base32 digit token rejected because mixed-length tokens fail uniform-group guard'
);
// mixed-case tokens are not accepted (capitalized words, not TOTP keys)
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'Your secret Jbswy3dpEhpk3pxp is here' })),
  null,
  'mixed-case token rejected'
);
// lowercase key without label still returns null
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'jbswy3dpehpk3pxp with no label nearby' })),
  null,
  'lowercase key without label returns null'
);

// detectManualSetupKey — complete long keys. The extracted value must be the whole
// secret, never a valid-looking prefix of it.
function groupsOf(key, width) {
  return key.match(new RegExp(`.{1,${width}}`, 'g')).join(' ');
}
const KEY32 = 'JBSWY3DPEHPK3PXPQRSTUVWX23456723';
const KEY64 = KEY32 + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.slice(0, 32);
const KEY128 = KEY64 + KEY64;
const detectIn = (text) => detectManualSetupKey(fakeDoc({ bodyText: text }));

// uppercase 32 chars: grouped 8x4 and standalone
assert.strictEqual(detectIn(`Your manual setup key is ${groupsOf(KEY32, 4)} — enter it in your app`), KEY32, 'uppercase 32-char key in 8 groups of 4 returned complete');
assert.strictEqual(detectIn(`Your secret is ${KEY32} — enter it in your app`), KEY32, 'standalone uppercase 32-char key');
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY32, 8)}`), KEY32, 'uppercase 32-char key in 4 groups of 8');
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY32, 16)}`), KEY32, 'uppercase 32-char key in 2 groups of 16 is not cut to its first token');
// 16-char behaviour preserved
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY32.slice(0, 16), 4)}`), KEY32.slice(0, 16), 'uppercase 16-char grouped');
assert.strictEqual(detectIn(`Your secret is ${KEY32.slice(0, 16)}`), KEY32.slice(0, 16), 'uppercase 16-char standalone');
// all-letter uppercase keys
assert.strictEqual(
  detectIn('Your manual setup key is AABB CCDD AABB CCDD EEFF GGHH IIJJ KKLL'),
  'AABBCCDDAABBCCDDEEFFGGHHIIJJKKLL',
  'all-letter uppercase 32-char grouped key'
);
assert.strictEqual(
  detectIn('Your secret is AABBCCDDAABBCCDDEEFFGGHHIIJJKKLL'),
  'AABBCCDDAABBCCDDEEFFGGHHIIJJKKLL',
  'all-letter uppercase 32-char standalone key'
);
// lowercase 32 stays complete
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY32, 4).toLowerCase()}`), KEY32, 'lowercase 32-char key in 8 groups of 4');
// no label, no key
assert.strictEqual(detectIn(groupsOf(KEY32, 4)), null, 'uppercase 32-char key without a label returns null');

// more than 8 groups: no silent truncation at an 8-token window
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY64, 4)}`), KEY64, 'uppercase 64-char key in 16 groups of 4');
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY64, 4).toLowerCase()}`), KEY64, 'lowercase 64-char key in 16 groups of 4');
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY128, 4)}`), KEY128, 'a key at the 128-char base32 cap is accepted');
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY128 + 'ABCD', 4)}`), null, 'a sequence past the 128-char cap fails closed instead of returning a prefix');
assert.strictEqual(detectIn(`Your secret is ${groupsOf(KEY128 + 'ABCD', 4).toLowerCase()}`), null, 'lowercase sequence past the cap fails closed too');

// A short prose word after a uniform-width key is never absorbed into the secret. Keys
// whose final group is narrower than the rest are unsupported (indistinguishable from
// this), so nothing here asserts a partial-group key.
const KEY16 = KEY32.slice(0, 16);
for (const word of ['OK', 'IN', 'TO', 'OR', 'A', 'AM']) {
  assert.strictEqual(
    detectIn(`Secret: ${groupsOf(KEY16, 4)} ${word} your app`), KEY16,
    `16-char grouped key followed by "${word}" returns the exact key`
  );
  assert.strictEqual(
    detectIn(`Secret: ${groupsOf(KEY32, 4)} ${word}`), KEY32,
    `32-char 8x4 key followed by "${word}" returns the exact key`
  );
}
assert.strictEqual(detectIn('Secret: JBSW Y3DP EHPK 3PXP OK'), KEY16, 'uppercase key plus OK');
assert.strictEqual(detectIn('Secret: JBSW Y3DP EHPK 3PXP IN your app'), KEY16, 'uppercase key plus IN');
assert.strictEqual(
  detectIn(`Your secret is ${groupsOf(KEY16, 4).toLowerCase()} to continue`),
  KEY16,
  'lowercase keeps strict equal-width groups so a trailing prose word is never absorbed'
);

// Digit-only groups are valid in both cases: leading, middle and trailing numeric groups
// keep the full key whichever case the letter groups use.
const NUM_KEY = '23456723JBSWY3DPEHPK3PXPQRSTUVWX';
assert.strictEqual(
  detectIn('Secret: 2345 6723 jbsw y3dp ehpk 3pxp qrst uvwx'), NUM_KEY,
  'lowercase key with leading numeric groups'
);
assert.strictEqual(
  detectIn('Secret: 2345 6723 JBSW Y3DP EHPK 3PXP QRST UVWX'), NUM_KEY,
  'uppercase key with leading numeric groups'
);
assert.strictEqual(
  detectIn('Secret: jbsw y3dp 2345 6723 ehpk 3pxp qrst uvwx'), 'JBSWY3DP23456723EHPK3PXPQRSTUVWX',
  'lowercase key with middle numeric groups'
);
assert.strictEqual(
  detectIn('Secret: JBSW Y3DP 2345 6723 EHPK 3PXP QRST UVWX'), 'JBSWY3DP23456723EHPK3PXPQRSTUVWX',
  'uppercase key with middle numeric groups'
);
assert.strictEqual(
  detectIn('Secret: jbsw y3dp ehpk 3pxp qrst uvwx 2345 6723'), 'JBSWY3DPEHPK3PXPQRSTUVWX23456723',
  'lowercase key with trailing numeric groups'
);
assert.strictEqual(
  detectIn('Secret: JBSW Y3DP EHPK 3PXP QRST UVWX 2345 6723'), 'JBSWY3DPEHPK3PXPQRSTUVWX23456723',
  'uppercase key with trailing numeric groups'
);
// A numeric prefix of 16+ chars is a complete valid key on its own, and it starts at the same
// token as the longer lowercase run. The longer (complete) run must win the tie.
assert.strictEqual(
  detectIn('Manual setup key: 2345 6723 2345 6723 jbsw y3dp ehpk 3pxp'),
  '2345672323456723JBSWY3DPEHPK3PXP',
  'lowercase key behind a 16-char numeric prefix returns the complete 32-char key'
);

// surrounding prose of other widths is not absorbed into the key
assert.strictEqual(
  detectIn(`AUTHENTICATOR SETUP KEY ${groupsOf(KEY32.slice(0, 16), 4)} AFTERWARD CONFIRM`),
  KEY32.slice(0, 16),
  'uppercase label words of other widths are not absorbed into a grouped 16-char key'
);
assert.strictEqual(
  detectIn(`SCAN OR ENTER THIS SETUP KEY ${groupsOf(KEY32, 4)} AFTERWARD CONFIRM`),
  KEY32,
  'uppercase label words of other widths are not absorbed into a grouped 32-char key'
);
assert.strictEqual(
  detectIn(`SECRET CODE 0189 ${groupsOf(KEY32, 4)}`),
  KEY32,
  'a token outside the base32 alphabet ends the run instead of poisoning it'
);
// uppercase prose alone, even with a label, is not a key
assert.strictEqual(
  detectIn('MANUAL SETUP KEY INFORMATION IS PROVIDED HERE TODAY'),
  null,
  'uppercase prose of mixed widths rejected'
);
// the detected value is a usable secret
assert.strictEqual(isBase32Secret(detectIn(`Your secret is ${groupsOf(KEY32, 4)}`)), true);

// priority ordering
const combinedDoc = fakeDoc({
  links: [fakeAnchor('otpauth://totp/Example:alice?secret=JBSWY3DP')],
  bodyText: 'manual setup key JBSWY3DPEHPK3PXP',
});
assert.strictEqual(detectTotpCandidateWithPriority(combinedDoc).priority, 1, 'otpauth link wins over manual key');

// shouldShowTotpBanner
assert.strictEqual(shouldShowTotpBanner(null, {}), false);
assert.strictEqual(shouldShowTotpBanner({ candidate: 'x', priority: 1 }, { vaultLocked: true }), false);
assert.strictEqual(shouldShowTotpBanner({ candidate: 'x', priority: 1 }, { vaultLocked: false, totpBannerDismissed: null }), true);
assert.strictEqual(shouldShowTotpBanner({ candidate: 'x', priority: 4 }, { vaultLocked: false, totpBannerDismissed: { priority: 1 } }), false, 'weaker candidate stays suppressed');
assert.strictEqual(shouldShowTotpBanner({ candidate: 'x', priority: 1 }, { vaultLocked: false, totpBannerDismissed: { priority: 4 } }), true, 'stronger candidate re-shows');

// dismissal stores the ACTUAL priority of the dismissed candidate (not a hardcoded weakest
// priority) — a priority-1 dismissal must suppress a subsequent priority-1 candidate too,
// per the plan's "session-dismiss suppresses only equal-or-weaker-priority candidates" rule.
assert.strictEqual(shouldShowTotpBanner({ candidate: 'x', priority: 1 }, { vaultLocked: false, totpBannerDismissed: { priority: 1 } }), false, 'dismissed priority-1 candidate stays suppressed for an equal-priority re-detection');

console.log('totpDetection.test.js: OK');

// --- requestTotpPrompt / requestSavePrompt: the offers are requests to the background now; the
// background (not this script) checks lock/pairing state, matches accounts and asks the user.
// What stays here is the dismissal memory: an offered priority suppresses equal-or-weaker ones
// (the MutationObserver would otherwise re-detect the same candidate ~400 ms later). ---

(() => {
  const state = { totpBannerDismissed: null };
  const sent = [];
  requestTotpPrompt('https://example.com', 'JBSWY3DP', state, 2, (message) => sent.push(message));
  assert.deepStrictEqual(sent, [{ type: 'requestPrompt', kind: 'saveTotp', url: 'https://example.com', otpauthOrSecret: 'JBSWY3DP' }]);
  assert.deepStrictEqual(state.totpBannerDismissed, { priority: 2 }, 'the offered priority is remembered');
  assert.strictEqual(shouldShowTotpBanner({ candidate: 'JBSWY3DP', priority: 2 }, state), false, 'not re-offered');
  assert.strictEqual(shouldShowTotpBanner({ candidate: 'x', priority: 1 }, state), true, 'a stronger candidate still is');
})();

(() => {
  const sent = [];
  const state = { lastLoginPrompt: null };
  assert.strictEqual(requestSavePrompt('https://example.com', 'a', 'b', state, (m) => sent.push(m), 5), true);
  assert.strictEqual(sent[0].kind, 'saveLogin');
  assert.strictEqual(requestSavePrompt('https://example.com', 'a', 'b', state, (m) => sent.push(m), 6), false);
  assert.strictEqual(sent.length, 1);
})();

console.log('totpDetection.test.js: requestTotpPrompt / requestSavePrompt OK');

// --- buildTotpCandidateQueryResponse: what the content script's onMessage listener
// answers a popup's `{ type: 'queryTotpCandidate' }` request with. The listener itself
// lives inside content.js's browser-only wiring guard (chrome/document both defined) and
// isn't reachable from this require (chrome/document are undefined at this point in the
// file), so this exercises the extracted response-building logic directly — same pattern
// as requestTotpPrompt above. ---

assert.deepStrictEqual(
  buildTotpCandidateQueryResponse(combinedDoc),
  detectTotpCandidateWithPriority(combinedDoc),
  'queryTotpCandidate response mirrors detectTotpCandidateWithPriority for a page with a candidate'
);
assert.strictEqual(
  buildTotpCandidateQueryResponse(fakeDoc({})),
  null,
  'queryTotpCandidate response is null for a page with no TOTP candidate'
);

console.log('totpDetection.test.js: buildTotpCandidateQueryResponse OK');

// --- QR detection ---

function fakeCanvasContext(imageDataResult) {
  return { drawImage: () => {}, getImageData: () => imageDataResult };
}

function fakeCanvas(imageDataResult, shouldThrow) {
  return {
    width: 0,
    height: 0,
    getContext: () => {
      if (shouldThrow) throw new Error('SecurityError: tainted canvas');
      return fakeCanvasContext(imageDataResult);
    },
  };
}

global.document = global.document || {};
global.document.createElement = (tag) => (tag === 'canvas' ? fakeCanvas({ data: [], width: 1, height: 1 }, global.__fakeCanvasThrows) : {});
global.jsQR = () => global.__fakeQrResult || null;

const { decodeQrFromImage, detectQrCandidate } = require('../content.js');

// Each assertion below uses its own <img> (distinct `src`) rather than reusing one object:
// decodeQrFromImage now memoizes per image (see the dedicated memoization test further down),
// so re-using the same element/src pair here would return the FIRST call's cached result
// instead of exercising the fresh-decode path each assertion is actually testing.
global.__fakeQrResult = { data: 'otpauth://totp/Example:alice?secret=JBSWY3DP' };
const decodableImg = { complete: true, naturalWidth: 200, naturalHeight: 200, src: 'https://example.com/qr-alice.png' };
assert.strictEqual(decodeQrFromImage(decodableImg), 'otpauth://totp/Example:alice?secret=JBSWY3DP');

global.__fakeQrResult = null;
const noPayloadImg = { complete: true, naturalWidth: 200, naturalHeight: 200, src: 'https://example.com/qr-no-payload.png' };
assert.strictEqual(decodeQrFromImage(noPayloadImg), null, 'no QR payload decodes to null');

global.__fakeCanvasThrows = true;
const taintedImg = { complete: true, naturalWidth: 200, naturalHeight: 200, src: 'https://example.com/qr-tainted.png' };
assert.strictEqual(decodeQrFromImage(taintedImg), null, 'CORS-tainted canvas is caught and skipped, not thrown');
global.__fakeCanvasThrows = false;

const incompleteImg = { complete: false, naturalWidth: 0, naturalHeight: 0 };
assert.strictEqual(decodeQrFromImage(incompleteImg), null, 'unloaded image is skipped');

global.__fakeQrResult = { data: 'otpauth://totp/Example:carol?secret=ABCDEFGH' };
const carolImg = { complete: true, naturalWidth: 200, naturalHeight: 200, src: 'https://example.com/qr-carol.png' };
const qrDoc = { querySelectorAll: (sel) => (sel === 'img' ? [carolImg] : []) };
assert.strictEqual(detectQrCandidate(qrDoc), 'otpauth://totp/Example:carol?secret=ABCDEFGH');

console.log('totpDetection.test.js: QR detection OK');
