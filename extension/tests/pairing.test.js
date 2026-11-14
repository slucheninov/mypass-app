const assert = require('node:assert');

const storage = {};
const sentNativeMessages = [];
global.chrome = {
  storage: {
    local: {
      get: (keys, cb) => {
        const result = {};
        for (const key of (Array.isArray(keys) ? keys : [keys])) {
          if (storage[key] !== undefined) result[key] = storage[key];
        }
        cb(result);
      },
      set: (items, cb) => {
        Object.assign(storage, items);
        if (cb) cb();
      },
      remove: (keys, cb) => {
        for (const key of (Array.isArray(keys) ? keys : [keys])) delete storage[key];
        if (cb) cb();
      }
    }
  },
  runtime: {
    lastError: null,
    onMessage: { addListener: () => {} },
    sendNativeMessage: (_host, message, cb) => {
      sentNativeMessages.push(message);
      cb({ type: 'paired', secret: 'newSecret' });
    }
  }
};

const {
  PAIRING_STORAGE_KEY,
  attachPairingField,
  persistPairingResponse,
  clearStaleSecretOnPairingRequired,
  detectBrowserName,
  sendPairingCompletion
} = require('../background.js');

const { classifyGetCredentialsResponse, isPairingChallengeFresh, normalizePairingCode, PAIRING_CHALLENGE_TTL_MS } = require('../popup.js');

assert.strictEqual(classifyGetCredentialsResponse(null), 'error');
assert.strictEqual(classifyGetCredentialsResponse({ type: 'error', message: 'x' }), 'error');
assert.strictEqual(classifyGetCredentialsResponse({ type: 'locked' }), 'locked');
assert.strictEqual(classifyGetCredentialsResponse({ type: 'pairingRequired' }), 'pairingRequired');
assert.strictEqual(classifyGetCredentialsResponse({ type: 'credentialList', credentials: [] }), 'credentialList');
assert.strictEqual(classifyGetCredentialsResponse({ type: 'unexpected' }), 'unknown');

const now = 1_000_000;
assert.strictEqual(isPairingChallengeFresh(now - 60_000, now), true);
assert.strictEqual(isPairingChallengeFresh(now - PAIRING_CHALLENGE_TTL_MS, now), false);
assert.strictEqual(isPairingChallengeFresh(undefined, now), false);
assert.strictEqual(isPairingChallengeFresh(null, now), false);

assert.strictEqual(normalizePairingCode('123 456'), '123456');
assert.strictEqual(normalizePairingCode('1234567'), '123456');
assert.strictEqual(normalizePairingCode('12a3b4c'), '1234');

assert.strictEqual(attachPairingField({ type: 'getCredentials', url: 'https://example.com' }, 'sec').pairing, 'sec');
assert.strictEqual(attachPairingField({ type: 'getCredentials', url: 'https://example.com' }, null).pairing, undefined);

delete global.navigator;
global.navigator = {
  userAgentData: {
    brands: [
      { brand: 'Google Chrome' },
      { brand: 'Chromium' },
      { brand: 'Not.A/Brand' }
    ]
  }
};
assert.strictEqual(detectBrowserName(), 'Google Chrome');

delete global.navigator.userAgentData;
assert.strictEqual(detectBrowserName(), 'Chromium browser');

(async () => {
  await persistPairingResponse({ type: 'paired', secret: 'YWJj' });
  assert.strictEqual(storage[PAIRING_STORAGE_KEY], 'YWJj');
  storage[PAIRING_STORAGE_KEY] = 'stale';
  await clearStaleSecretOnPairingRequired({ type: 'pairingRequired' }, 'stale');
  assert.strictEqual(storage[PAIRING_STORAGE_KEY], undefined);

  storage[PAIRING_STORAGE_KEY] = 'stale';
  sentNativeMessages.length = 0;
  await sendPairingCompletion({ type: 'completePairing', code: '123456' });
  assert.strictEqual(sentNativeMessages.length, 1);
  assert.strictEqual(sentNativeMessages[0].type, 'completePairing');
  assert.strictEqual(sentNativeMessages[0].code, '123456');
  assert.strictEqual(sentNativeMessages[0].pairing, undefined);
  assert.strictEqual(storage[PAIRING_STORAGE_KEY], 'newSecret');

  // Transport errors describe connectivity, not whether an already-open app exists.
  // Preserve only known diagnostic categories; never expose raw host/path details.
  const originalNativeSend = chrome.runtime.sendNativeMessage;
  const { handleRuntimeMessage } = require('../background.js');
  for (const [rawError, expectedCode] of [
    ['Specified native messaging host not found.', 'native_host_not_found'],
    ['Native host has exited.', 'connection_unavailable'],
    ['Access to the specified native messaging host is forbidden.', 'connection_unavailable'],
    ['/private/test-only/path: native messaging failed', 'connection_unavailable'],
    [null, 'connection_unavailable']
  ]) {
    let calls = 0;
    chrome.runtime.sendNativeMessage = (_host, _message, callback) => {
      calls++;
      chrome.runtime.lastError = rawError ? { message: rawError } : null;
      callback(undefined);
      chrome.runtime.lastError = null;
    };
    const response = await new Promise((resolve) => handleRuntimeMessage(
      { type: 'getCredentials', url: 'https://example.com' }, {}, resolve));
    assert.deepStrictEqual(response, { type: 'error', code: expectedCode, message: expectedCode });
    assert.strictEqual(calls, 1, 'transport errors must not automatically replay requests');
    assert.strictEqual(storage[PAIRING_STORAGE_KEY], 'newSecret', 'transport failure must not revoke pairing');
  }
  chrome.runtime.sendNativeMessage = originalNativeSend;

  {
    const { knownTypes } = require('../background.js');
    assert.ok(knownTypes.includes('getTotp'), 'getTotp must be a known routed message type');
    console.log('pairing.test.js: getTotp knownTypes OK');
  }

  console.log('pairing.test.js OK');
})();

{
  const { handleRuntimeMessage } = require('../background.js');

  // The old content-script "open the popup for the picker" message is gone: prompts go through
  // requestPrompt, which the background binds to the authenticated sender.
  let openPopupCalled = false;
  global.chrome.action = { openPopup: () => { openPopupCalled = true; return Promise.resolve(); } };
  const legacy = handleRuntimeMessage({ type: 'openPopupForTotpPicker' }, {}, () => {});
  assert.strictEqual(openPopupCalled, false, 'openPopupForTotpPicker no longer opens the popup');
  assert.strictEqual(legacy, false, 'and is dropped as an unknown type');

  const keepChannelOpen = handleRuntimeMessage(
    { type: 'proposeSaveTotp', url: 'https://example.com', otpauthOrSecret: 'JBSWY3DP' }, {}, () => {}
  );
  assert.strictEqual(keepChannelOpen, true, 'proposeSaveTotp is a known type routed through native messaging');

  console.log('pairing.test.js: proposeSaveTotp routing OK');
}
