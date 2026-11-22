const assert = require('node:assert');
const {
  detectOtpauthLink, detectOtpauthText, isBase32Secret, detectManualSetupKey,
  detectTotpCandidateWithPriority, shouldShowTotpBanner, requestTotpPrompt, requestSavePrompt,
  buildTotpCandidateQueryResponse,
} = require('../content.js');

const { parseHtml } = require('./miniDom.js');

// A page with these links and this visible paragraph. The scan reads the DOM's visible text,
// so the fixtures are real (mini) documents, not a bare textContent string.
function fakeAnchor(href) {
  return href;
}

function fakeDoc({ links = [], bodyText = '' }) {
  const anchors = links.map((href) => `<a href="${href}">link</a>`).join('');
  return parseHtml(`${anchors}<p>${bodyText}</p>`);
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

// detectManualSetupKey: a key is a distinct token (the whole text of its own element, after a
// short label at most), next to 2FA wording. `keyIn` puts the token in its own paragraph
// between a setup-key heading and an instruction, the way setup pages lay it out.
const keyIn = (token) => detectManualSetupKey(parseHtml(
  `<div><p>Your manual setup key:</p><p>${token}</p><p>Enter it in your authenticator app</p></div>`
));

assert.strictEqual(keyIn('JBSW Y3DP EHPK 3PXP'), 'JBSWY3DPEHPK3PXP', 'uppercase grouped key');
assert.strictEqual(detectManualSetupKey(fakeDoc({ bodyText: 'JBSWY3DPEHPK3PXP' })), null, 'requires 2FA wording nearby');
assert.strictEqual(keyIn('jbswy3dpehpk3pxp'), 'JBSWY3DPEHPK3PXP', 'single lowercase token normalised to uppercase');
assert.strictEqual(keyIn('jbsw y3dp ehpk 3pxp'), 'JBSWY3DPEHPK3PXP', 'space-grouped lowercase key');
assert.strictEqual(keyIn('aabbccddaabbccdd'), null, 'a single all-letter token reads as a word');
assert.strictEqual(keyIn('aabb ccdd aabb ccdd'), 'AABBCCDDAABBCCDD', 'grouped all-letter lowercase key');
assert.strictEqual(keyIn('aabb ccdd aabb ccdd eeff gghh iijj kkll'), 'AABBCCDDAABBCCDDEEFFGGHHIIJJKKLL', '8 groups of 4');
assert.strictEqual(keyIn('information is provided here today in this form'), null, 'prose in its own element');
assert.strictEqual(keyIn('Jbswy3dpEhpk3pxp'), null, 'mixed-case token rejected');
assert.strictEqual(keyIn('JBSW y3dp EHPK 3PXP'), null, 'mixed-case groups rejected');
assert.strictEqual(
  detectManualSetupKey(fakeDoc({ bodyText: 'Your secret key is jbswy3dpehpk3pxp enter it in your app' })),
  null,
  'a key inside a sentence is not a distinct token'
);

// Long keys: the whole secret or nothing, never a valid-looking prefix of it.
function groupsOf(key, width) {
  return key.match(new RegExp(`.{1,${width}}`, 'g')).join(' ');
}
const KEY32 = 'JBSWY3DPEHPK3PXPQRSTUVWX23456723';
const KEY64 = KEY32 + 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.slice(0, 32);
const KEY128 = KEY64 + KEY64;
const KEY16 = KEY32.slice(0, 16);

assert.strictEqual(keyIn(groupsOf(KEY32, 4)), KEY32, 'uppercase 32-char key in 8 groups of 4');
assert.strictEqual(keyIn(groupsOf(KEY32, 4).toLowerCase()), KEY32, 'lowercase 32-char key in 8 groups of 4');
assert.strictEqual(keyIn(KEY32), KEY32, 'standalone uppercase 32-char key');
assert.strictEqual(keyIn(groupsOf(KEY16, 4)), KEY16, 'uppercase 16-char grouped');
assert.strictEqual(keyIn(KEY16), KEY16, 'uppercase 16-char standalone');
assert.strictEqual(keyIn(groupsOf(KEY32, 8)), null, 'groups of 8 are not the grouped form');
assert.strictEqual(keyIn(groupsOf(KEY32, 16)), null, 'two groups of 16 are not the grouped form');
assert.strictEqual(keyIn('AABB CCDD AABB CCDD EEFF GGHH IIJJ KKLL'), 'AABBCCDDAABBCCDDEEFFGGHHIIJJKKLL', 'all-letter uppercase grouped key');
assert.strictEqual(keyIn('AABBCCDDAABBCCDDEEFFGGHHIIJJKKLL'), null, 'an all-letter uppercase single token reads as a word');
assert.strictEqual(keyIn(groupsOf(KEY64, 4)), null, 'more than 8 groups fails closed instead of returning a part');
assert.strictEqual(keyIn(KEY64), KEY64, 'standalone 64-char key');
assert.strictEqual(keyIn(KEY128), KEY128, 'a key at the 128-char base32 cap is accepted');
assert.strictEqual(keyIn(KEY128 + 'ABCD'), null, 'a token past the 128-char cap fails closed');

// A word after the key inside the same element is no distinct token; the key in its own
// element followed by prose is.
for (const word of ['OK', 'IN', 'TO', 'A']) {
  assert.strictEqual(keyIn(`${groupsOf(KEY16, 4)} ${word}`), null, `key plus "${word}" in one element`);
  assert.strictEqual(
    detectManualSetupKey(parseHtml(`<p>Secret key: <code>${groupsOf(KEY16, 4)}</code> ${word} your app</p>`)),
    KEY16,
    `key in <code> followed by "${word}" returns the exact key`
  );
}

// Digit-only groups are valid in both cases: leading, middle and trailing numeric groups keep
// the full key whichever case the letter groups use.
assert.strictEqual(keyIn('2345 6723 jbsw y3dp ehpk 3pxp qrst uvwx'), '23456723JBSWY3DPEHPK3PXPQRSTUVWX', 'leading numeric groups, lowercase');
assert.strictEqual(keyIn('2345 6723 JBSW Y3DP EHPK 3PXP QRST UVWX'), '23456723JBSWY3DPEHPK3PXPQRSTUVWX', 'leading numeric groups, uppercase');
assert.strictEqual(keyIn('jbsw y3dp 2345 6723 ehpk 3pxp qrst uvwx'), 'JBSWY3DP23456723EHPK3PXPQRSTUVWX', 'middle numeric groups');
assert.strictEqual(keyIn('JBSW Y3DP EHPK 3PXP QRST UVWX 2345 6723'), 'JBSWY3DPEHPK3PXPQRSTUVWX23456723', 'trailing numeric groups');
assert.strictEqual(keyIn('2345 6723 2345 6723 jbsw y3dp ehpk 3pxp'), '2345672323456723JBSWY3DPEHPK3PXP', 'a numeric prefix is not cut off as its own key');

// Label words in their own nodes are never absorbed into the key.
assert.strictEqual(
  detectManualSetupKey(parseHtml(`<div><p>AUTHENTICATOR SETUP KEY</p><p>${groupsOf(KEY32, 4)}</p><p>AFTERWARD CONFIRM</p></div>`)),
  KEY32,
  'uppercase label words around the key stay out of it'
);
assert.strictEqual(keyIn('MANUAL SETUP KEY INFORMATION IS PROVIDED HERE TODAY'), null, 'uppercase prose rejected');
// the detected value is a usable secret
assert.strictEqual(isBase32Secret(keyIn(groupsOf(KEY32, 4))), true);

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
