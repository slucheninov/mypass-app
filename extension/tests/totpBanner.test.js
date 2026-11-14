// Covers three parked follow-ups of the TOTP save banner:
//   3a — trustedClickHandler, and the save/2FA offers now being requestPrompt messages.
//   3b — per-image QR decode memoization (WeakMap cache in decodeQrFromImage).
//   3c — img.onload rescan (observeTotpCandidates' capture-phase 'load' listener), plus
//        review fix round 1: the listener must go dead with the extension context, and must
//        invalidate a refreshed image's cached decode before rescanning.

const assert = require('node:assert');

function fakeElement(tag) {
  const listeners = {};
  return {
    tag,
    style: {},
    textContent: '',
    children: [],
    removed: false,
    addEventListener(type, handler) {
      (listeners[type] = listeners[type] || []).push(handler);
    },
    dispatch(type, event) {
      (listeners[type] || []).forEach((handler) => handler(event));
    },
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    remove() {
      this.removed = true;
    },
  };
}

// --- 3a: trustedClickHandler ---
{
  const { trustedClickHandler } = require('../content.js');

  let calls = 0;
  const wrapped = trustedClickHandler((event) => {
    calls += 1;
  });

  wrapped({ isTrusted: false });
  assert.strictEqual(calls, 0, 'an untrusted click never invokes the wrapped handler');

  wrapped({ isTrusted: true });
  assert.strictEqual(calls, 1, 'a trusted click invokes the wrapped handler exactly once');

  wrapped(null);
  assert.strictEqual(calls, 1, 'a missing event is treated as untrusted, not a crash');
}
console.log('totpBanner.test.js: trustedClickHandler OK');

// --- 3a: the page no longer renders a save banner at all. A login submit / a detected 2FA secret
// becomes a requestPrompt message to the background (which owns the record, validates the sender,
// re-reads the offerSave* switches and shows the popup). There is nothing in the page's DOM for page
// script to click, so no isTrusted gate is needed on these sends; the in-page controls that remain
// (inline icon, picker, settings row, generate row) keep theirs. ---
{
  const content = require('../content.js');
  assert.strictEqual(content.showSaveBanner, undefined, 'the bottom save banner is gone');
  assert.strictEqual(content.showTotpSaveBanner, undefined, 'the bottom 2FA banner is gone');

  const sent = [];
  const send = (message) => sent.push(message);
  const state = { lastLoginPrompt: null };
  assert.strictEqual(content.requestSavePrompt('https://example.com/login', 'alice', 'hunter2', state, send, 1000), true);
  assert.deepStrictEqual(sent[0], {
    type: 'requestPrompt', kind: 'saveLogin', url: 'https://example.com/login', username: 'alice', password: 'hunter2'
  });
  assert.strictEqual(content.requestSavePrompt('https://example.com/login', 'alice', 'hunter2', state, send, 60000), false,
    'the same credentials do not re-open the popup within its 120 s lifetime');
  assert.strictEqual(sent.length, 1);
  assert.strictEqual(content.requestSavePrompt('https://example.com/login', 'alice', 'other-pass', state, send, 61000), true,
    'a different password is a new offer');
  assert.strictEqual(content.requestSavePrompt('https://example.com/login', 'alice', 'hunter2', state, send, 1000 + 120000), true,
    'and the same pair is offered again once the first prompt has expired');
  assert.ok(!JSON.stringify(Object.keys(state)).includes('banner'), 'no banner state is kept');

  const totpSent = [];
  const totpState = { totpBannerDismissed: null };
  content.requestTotpPrompt('https://example.com/2fa', 'otpauth://totp/Example:alice?secret=JBSWY3DP', totpState, 3, (m) => totpSent.push(m));
  assert.deepStrictEqual(totpSent[0], {
    type: 'requestPrompt', kind: 'saveTotp', url: 'https://example.com/2fa', otpauthOrSecret: 'otpauth://totp/Example:alice?secret=JBSWY3DP'
  });
  assert.deepStrictEqual(totpState.totpBannerDismissed, { priority: 3 },
    'offering records the candidate priority so an equal-or-weaker one is not re-offered');
  assert.strictEqual(content.shouldShowTotpBanner({ candidate: 'x', priority: 3 }, totpState), false, 'same priority is suppressed');
  assert.strictEqual(content.shouldShowTotpBanner({ candidate: 'x', priority: 1 }, totpState), true, 'a stronger candidate is offered');
}
// A submit raises a save prompt (browser UI) only for a real person: isTrusted alone is not
// enough, because form.requestSubmit() dispatches a trusted submit from page code (verified in
// Chrome: isTrusted=true, navigator.userActivation.isActive=false). The second condition is the
// browser-set, unforgeable user activation.
{
  const content = require('../content.js');
  const trusted = { isTrusted: true };
  const synthetic = { isTrusted: false };
  const active = { userActivation: { isActive: true, hasBeenActive: true } };
  const idleAfterEarlierClick = { userActivation: { isActive: false, hasBeenActive: true } };
  assert.strictEqual(content.isUserSubmit(trusted, active), true, 'a click / Enter submit inside a live activation');
  assert.strictEqual(content.isUserSubmit(trusted, idleAfterEarlierClick), false,
    'requestSubmit() long after any click: trusted but no live activation (hasBeenActive is sticky and must not count)');
  assert.strictEqual(content.isUserSubmit(trusted, {}), false, 'no userActivation API: fail closed');
  assert.strictEqual(content.isUserSubmit(trusted, null), false);
  assert.strictEqual(content.isUserSubmit(synthetic, active), false, 'a dispatched submit event is refused even inside an activation');
  assert.strictEqual(content.isUserSubmit(null, active), false);
  assert.strictEqual(content.hasLiveUserActivation({ userActivation: { isActive: 'yes' } }), false, 'only the boolean true counts');
  assert.strictEqual(content.hasLiveUserActivation(active), true);
}
console.log('totpBanner.test.js: save/2FA requests replace the bottom banners OK');

// --- 3b: per-image QR decode memoization ---
{
  let jsQrCalls = 0;
  global.__memoQrResult = { data: 'otpauth://totp/Example:memo?secret=JBSWY3DP' };
  global.jsQR = () => {
    jsQrCalls += 1;
    return global.__memoQrResult;
  };
  global.document = {
    createElement: (tag) => (tag === 'canvas' ? {
      width: 0,
      height: 0,
      getContext: () => ({
        drawImage: () => {},
        getImageData: () => ({ data: [], width: 1, height: 1 }),
      }),
    } : fakeElement(tag)),
    body: { appendChild: () => {} },
  };

  const { decodeQrFromImage } = require('../content.js');

  const img = { complete: true, naturalWidth: 200, naturalHeight: 200, src: 'https://example.com/memo.png' };

  const first = decodeQrFromImage(img);
  const second = decodeQrFromImage(img);
  assert.strictEqual(first, 'otpauth://totp/Example:memo?secret=JBSWY3DP');
  assert.strictEqual(second, first, 'second decode of the same unchanged image returns the same cached result');
  assert.strictEqual(jsQrCalls, 1, 'jsQR is invoked only once for two decodes of the same unchanged image');

  // currentSrc takes precedence over src when both are present, per the memo key spec.
  img.currentSrc = 'https://example.com/memo.png';
  decodeQrFromImage(img);
  assert.strictEqual(jsQrCalls, 1, 'currentSrc equal to src is still a cache hit');

  // Changing src (an SPA reusing the same <img> node for a different picture) must
  // invalidate the cache and trigger a fresh decode.
  img.src = 'https://example.com/memo-2.png';
  delete img.currentSrc;
  global.__memoQrResult = { data: 'otpauth://totp/Example:memo2?secret=ABCDEFGH' };
  const third = decodeQrFromImage(img);
  assert.strictEqual(jsQrCalls, 2, 'changing src decodes again instead of returning the stale cached result');
  assert.strictEqual(third, 'otpauth://totp/Example:memo2?secret=ABCDEFGH');

  // Incomplete images are never cached, and never even reach jsQR.
  const incomplete = { complete: false, naturalWidth: 0, naturalHeight: 0, src: 'https://example.com/incomplete.png' };
  assert.strictEqual(decodeQrFromImage(incomplete), null);
  assert.strictEqual(jsQrCalls, 2, 'an incomplete image is skipped before jsQR runs, and never cached');
}
console.log('totpBanner.test.js: decodeQrFromImage memoization OK');

// --- 3c: img.onload triggers a rescan, respects the extension-context gate, and invalidates
// the 3b memo cache for the refreshed image ---
{
  const savedDocument = global.document;
  const savedObserver = global.MutationObserver;
  const savedSetTimeout = global.setTimeout;
  const savedClearTimeout = global.clearTimeout;
  const savedChrome = global.chrome;

  // The sweep itself is debounced via setTimeout(sweep, 400) inside observeTotpCandidates.
  // Stubbed to run synchronously so this test can assert the load listener actually reaches
  // the sweep (via onAttachIcons) rather than only asserting it doesn't throw.
  global.setTimeout = (fn) => { fn(); return 0; };
  global.clearTimeout = () => {};

  // The extension context is alive for the "normal" assertions below; flipped to dead
  // (chrome deleted) for the dedicated dead-context assertion further down.
  global.chrome = { runtime: { id: 'test-extension-id' } };

  global.MutationObserver = class {
    constructor(callback) { this.callback = callback; }
    observe() {}
    disconnect() {}
  };

  const loadListeners = [];
  const body = { nodeType: 1 };
  let jsQrCalls = 0;
  global.__rescanQrResult = { data: 'otpauth://totp/Example:rescan?secret=JBSWY3DP' };
  global.jsQR = () => {
    jsQrCalls += 1;
    return global.__rescanQrResult;
  };
  global.document = {
    body,
    querySelectorAll: () => [],
    querySelector: () => null,
    addEventListener: (type, handler, capture) => {
      if (type === 'load' && capture === true) loadListeners.push(handler);
    },
    createElement: (tag) => (tag === 'canvas' ? {
      width: 0,
      height: 0,
      getContext: () => ({
        drawImage: () => {},
        getImageData: () => ({ data: [], width: 1, height: 1 }),
      }),
    } : fakeElement(tag)),
  };

  const { observeTotpCandidates, decodeQrFromImage } = require('../content.js');

  let mutationCalls = 0;
  const state = { totpBannerDismissed: null, vaultLocked: false };
  observeTotpCandidates(state, () => { mutationCalls += 1; }, true);

  assert.strictEqual(loadListeners.length, 1, 'observeTotpCandidates attaches exactly one capture-phase load listener when QR scanning is enabled for this frame');

  // Non-<img> target: no rescan.
  loadListeners[0]({ isTrusted: true, target: { tagName: 'IFRAME' } });
  assert.strictEqual(mutationCalls, 0, 'a load event on a non-img target does not schedule a rescan');

  // <img> target: schedules the same debounced sweep as the MutationObserver (setTimeout is
  // stubbed synchronous above, so onAttachIcons — mutationCalls — fires immediately).
  loadListeners[0]({ isTrusted: true, target: { tagName: 'IMG' } });
  assert.strictEqual(mutationCalls, 1, 'a load event on an <img> target schedules the same sweep the MutationObserver uses');

  // A page-synthesized (untrusted) load event must not evict the cache or schedule a sweep:
  // it must not be able to force a rescan or evict the QR cache on demand.
  mutationCalls = 0;
  const untrustedImg = { complete: true, naturalWidth: 200, naturalHeight: 200, src: 'https://example.com/untrusted.png', tagName: 'IMG' };
  const untrustedCachedFirst = decodeQrFromImage(untrustedImg);
  loadListeners[0]({ isTrusted: false, target: untrustedImg });
  assert.strictEqual(mutationCalls, 0, 'an untrusted load event does not schedule a rescan');
  const untrustedCachedSecond = decodeQrFromImage(untrustedImg);
  assert.strictEqual(untrustedCachedSecond, untrustedCachedFirst, 'an untrusted load event does not evict the cache entry for its image');
  jsQrCalls = 0;

  // Review fix #2: a same-URL, same-size QR refresh (re-fetched endpoint, animated image)
  // still fires 'load'. The listener must drop that image's cached decode before rescanning,
  // or decodeQrFromImage would keep handing back the pre-refresh verdict forever.
  const refreshingImg = { complete: true, naturalWidth: 200, naturalHeight: 200, src: 'https://example.com/rescan.png', tagName: 'IMG' };
  const cachedFirst = decodeQrFromImage(refreshingImg);
  const cachedSecond = decodeQrFromImage(refreshingImg);
  assert.strictEqual(cachedSecond, cachedFirst, 'sanity: unrelated to the load event, repeat decodes of an unchanged image are cached');
  assert.strictEqual(jsQrCalls, 1, 'sanity: the cache is doing its job before any load event fires');

  loadListeners[0]({ isTrusted: true, target: refreshingImg });
  global.__rescanQrResult = { data: 'otpauth://totp/Example:rescan2?secret=ABCDEFGH' };
  const afterLoad = decodeQrFromImage(refreshingImg);
  assert.strictEqual(jsQrCalls, 2, 'a load event on a cached image invalidates its cache entry, so the next decode calls jsQR again');
  assert.strictEqual(afterLoad, 'otpauth://totp/Example:rescan2?secret=ABCDEFGH');

  // Review fix #1: a dead extension context must not re-run the sweep, independent of
  // whether teardownExtensionContext has already removed this listener (that machinery lives
  // in content.js's non-exported browser wiring and isn't reachable from this require).
  delete global.chrome;
  mutationCalls = 0;
  loadListeners[0]({ isTrusted: true, target: { tagName: 'IMG' } });
  assert.strictEqual(mutationCalls, 0, 'a load event is ignored while the extension context is dead');
  global.chrome = { runtime: { id: 'test-extension-id' } };

  // Calling observeTotpCandidates again (e.g. from a second content-script injection) must
  // not attach a second document-level load listener.
  observeTotpCandidates(state, () => {}, true);
  assert.strictEqual(loadListeners.length, 1, 'a second observeTotpCandidates call does not attach a duplicate load listener');

  global.document = savedDocument;
  global.MutationObserver = savedObserver;
  global.setTimeout = savedSetTimeout;
  global.clearTimeout = savedClearTimeout;
  global.chrome = savedChrome;
}
console.log('totpBanner.test.js: observeTotpCandidates load-listener rescan, context-death, and cache invalidation OK');
