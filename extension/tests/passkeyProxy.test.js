const assert = require('node:assert');

// `chrome.storage.local`/`.session` are stubbed callback-style, matching both the real
// MV3 API surface background.js wraps and pairing.test.js's existing stub shape.
const localStore = { passkeyProxyEnabled: true };
let sessionStore = {};
const sentNativeMessages = [];
const alarms = {};
let nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: true, enabled: true });

function makeArea(store) {
  return {
    get: (keys, cb) => {
      const result = {};
      for (const key of (Array.isArray(keys) ? keys : [keys])) {
        if (store[key] !== undefined) result[key] = store[key];
      }
      cb(result);
    },
    set: (items, cb) => {
      Object.assign(store, items);
      if (cb) cb();
    },
    remove: (keys, cb) => {
      for (const key of (Array.isArray(keys) ? keys : [keys])) delete store[key];
      if (cb) cb();
    }
  };
}

let attachCalled = false;
let detachCalled = false;
let completedCreate = null;
let completedGet = null;
let completedIsUvpaa = null;
let popupOpened = 0;
let notified = 0;
let lastNotificationId = null;
let notificationClickListener = null;
let openPopupResult = () => Promise.resolve();
let canceledListener;
let alarmListener;

global.chrome = {
  storage: { local: makeArea(localStore), session: makeArea(sessionStore) },
  runtime: {
    lastError: null,
    onMessage: { addListener: () => {} },
    sendNativeMessage: (_host, message, cb) => {
      sentNativeMessages.push(message);
      cb(nativeResponder(message));
    }
  },
  // The watchdog runs on chrome.alarms, not setTimeout — a setTimeout does not
  // survive service-worker termination.
  alarms: {
    create: (name, info) => { alarms[name] = info; },
    clear: (name) => { delete alarms[name]; },
    onAlarm: { addListener: (listener) => { alarmListener = listener; } }
  },
  webAuthenticationProxy: {
    attach: () => { attachCalled = true; return Promise.resolve(); },
    detach: () => { detachCalled = true; return Promise.resolve(); },
    onCreateRequest: { addListener: () => {} },
    onGetRequest: { addListener: () => {} },
    onIsUvpaaRequest: { addListener: () => {} },
    onRequestCanceled: { addListener: (listener) => { canceledListener = listener; } },
    completeCreateRequest: (details) => { completedCreate = details; },
    completeGetRequest: (details) => { completedGet = details; },
    completeIsUvpaaRequest: (details) => { completedIsUvpaa = details; }
  },
  action: { openPopup: () => { popupOpened += 1; return openPopupResult(); } },
  notifications: {
    create: (id) => { notified += 1; lastNotificationId = id; },
    onClicked: { addListener: (listener) => { notificationClickListener = listener; } }
  },
  tabs: { query: (_opts, cb) => cb([{ url: 'https://example.com' }]) }
};

const {
  attachPasskeyProxyIfEligible,
  handlePasskeyRequest,
  completePasskeyRequest,
  passkeyNotAllowedError,
  originFromRequestDetails,
  readPending,
  writePending,
  clearPending,
  answerIsUvpaa,
  handleRuntimeMessage,
  knownTypes,
  PASSKEY_WATCHDOG_ALARM,
  PASSKEY_REFRESH_ALARM,
  getBrowserManagerDiagnostics
} = require('../background.js');

function resetState() {
  sessionStore = {};
  global.chrome.storage.session = makeArea(sessionStore);
  for (const key of Object.keys(alarms)) delete alarms[key];
  sentNativeMessages.length = 0;
  attachCalled = false;
  detachCalled = false;
  completedCreate = null;
  completedGet = null;
  completedIsUvpaa = null;
  popupOpened = 0;
  notified = 0;
  lastNotificationId = null;
  localStore.passkeyProxyEnabled = true;
  delete localStore.mypassNotificationSettings;
  openPopupResult = () => Promise.resolve();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: true, enabled: true });
}

assert.deepStrictEqual(passkeyNotAllowedError('x'), { name: 'NotAllowedError', message: 'x' });
console.log('passkeyProxy.test.js: passkeyNotAllowedError OK');

// The origin is authoritative from the payload, never guessed from the active tab —
// a ceremony can start in a background tab, and originUrl feeds the app's domain gate.
assert.strictEqual(
  originFromRequestDetails(JSON.stringify({
    challenge: 'AA',
    extensions: { remoteDesktopClientOverride: { origin: 'https://example.com' } }
  })),
  'https://example.com'
);
assert.strictEqual(originFromRequestDetails('{"challenge":"AA"}'), null, 'missing override must fail closed');
assert.strictEqual(originFromRequestDetails('not json'), null, 'unparseable payload must fail closed');
console.log('passkeyProxy.test.js: originFromRequestDetails OK');

assert.ok(knownTypes.includes('getPasskeyProxyStatus'));
assert.ok(knownTypes.includes('setPasskeyProxyEnabled'));
assert.ok(knownTypes.includes('passkeyCreateConfirm'));
assert.ok(knownTypes.includes('passkeyGetApprove'));
console.log('passkeyProxy.test.js: knownTypes includes the passkey relays OK');

const detailsFor = (origin) => JSON.stringify({
  challenge: 'AA',
  rp: { id: 'example.com', name: 'Example' },
  extensions: { remoteDesktopClientOverride: { origin } }
});

(async () => {
  await new Promise((resolve) => setImmediate(resolve));
  assert.strictEqual(notified, 0, 'a worker that starts while the vault is already unlocked stays quiet');

  // --- attach gating (app-authoritative: every status fetch resyncs the cache) --------
  resetState();
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(attachCalled, true, 'attach() called when setting enabled and status allows it');
  console.log('passkeyProxy.test.js: attachPasskeyProxyIfEligible attaches when eligible OK');

  // The app said disabled while the extension's cache still says enabled (e.g. toggled
  // off in the app, no restart yet): sync must detach and correct the cache. This is the
  // Gate 6 defect this fix closes — nothing used to ever detach on an app-side disable.
  resetState();
  localStore.passkeyProxyEnabled = true;
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: false, enabled: false });
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(detachCalled, true, 'detach() called when the app reports enabled:false');
  assert.strictEqual(attachCalled, false);
  assert.strictEqual(localStore.passkeyProxyEnabled, false, 'the local cache must be corrected to match the app');
  console.log('passkeyProxy.test.js: attachPasskeyProxyIfEligible detaches and corrects the cache when the app says disabled OK');

  // The app said enabled while the extension's cache still says disabled (toggled on in
  // the app, no restart yet): sync must attach and correct the cache.
  resetState();
  localStore.passkeyProxyEnabled = false;
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: true, enabled: true });
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(attachCalled, true, 'attach() called when the app reports enabled:true and attachAllowed:true');
  assert.strictEqual(detachCalled, false);
  assert.strictEqual(localStore.passkeyProxyEnabled, true, 'the local cache must be corrected to match the app');
  console.log('passkeyProxy.test.js: attachPasskeyProxyIfEligible attaches and corrects the cache when the app says enabled OK');

  // Enabled app-side but locked: neither attach nor detach — the lock already gates
  // ceremonies via attachAllowed on every request — but the cache still tracks `enabled`.
  resetState();
  localStore.passkeyProxyEnabled = false;
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: false, paired: true, attachAllowed: false, enabled: true });
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(attachCalled, false, 'attach() not called when status.attachAllowed is false (locked)');
  assert.strictEqual(detachCalled, false, 'detach() not called either — enabled is still true, just locked');
  assert.strictEqual(localStore.passkeyProxyEnabled, true);
  console.log('passkeyProxy.test.js: attachPasskeyProxyIfEligible skips when locked but tracks enabled OK');

  // sendNativeMessage resolves an {type:"error"} envelope rather than rejecting when
  // the host is unreachable, so the guard has to test the envelope, not just catch.
  // Without a status to trust, neither attach/detach nor the cache may be touched.
  resetState();
  localStore.passkeyProxyEnabled = false;
  nativeResponder = () => ({ type: 'error', message: 'app_not_running' });
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(attachCalled, false, 'attach() not called when MyPass is not running');
  assert.strictEqual(detachCalled, false, 'detach() not called when MyPass is not running');
  assert.strictEqual(localStore.passkeyProxyEnabled, false, 'the cache must be left untouched when the app is unreachable');
  console.log('passkeyProxy.test.js: attachPasskeyProxyIfEligible skips and leaves the cache untouched when app unavailable OK');

  // Old-shaped app (pre-`enabled` wire format): a status with attachAllowed but no
  // `enabled` field at all must not be treated as "disabled" — that would detach an
  // already legitimately-attached proxy and clobber the cache, and the popup toggle could
  // never re-enable (set -> refetch old-shaped status -> detach again).
  resetState();
  localStore.passkeyProxyEnabled = true;
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: true });
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(attachCalled, false, 'no attach without an authoritative enabled field');
  assert.strictEqual(detachCalled, false, 'no detach without an authoritative enabled field');
  assert.strictEqual(localStore.passkeyProxyEnabled, true, 'the cache must be left untouched for an old-shaped status');
  console.log('passkeyProxy.test.js: attachPasskeyProxyIfEligible ignores an old-shaped status missing enabled OK');

  // --- popup-open refresh ----------------------------------------------------------
  // popup.js sends this fire-and-forget on every open; it must re-run the same
  // app-authoritative eligibility check rather than waiting for a service-worker restart.
  resetState();
  localStore.passkeyProxyEnabled = false;
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: true, enabled: true });
  handleRuntimeMessage({ type: 'refreshPasskeyProxy' }, {}, () => {});
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.strictEqual(attachCalled, true, 'the refreshPasskeyProxy message re-evaluates and attaches');
  assert.strictEqual(localStore.passkeyProxyEnabled, true);
  console.log('passkeyProxy.test.js: refreshPasskeyProxy message triggers attachPasskeyProxyIfEligible OK');

  // --- ceremony relay ------------------------------------------------------
  resetState();
  nativeResponder = (msg) => (msg.type === 'passkeyCreate'
    ? { type: 'passkeyCreatePending', requestId: 1, rpId: 'example.com', rpName: 'Example', matches: [] }
    : null);
  await handlePasskeyRequest('create', 1, detailsFor('https://example.com'));
  assert.strictEqual(completedCreate, null, 'pending response must not auto-complete the ceremony');
  const pending = await readPending();
  assert.strictEqual(pending.requestId, 1);
  assert.strictEqual(pending.kind, 'create');
  assert.strictEqual(pending.originUrl, 'https://example.com');
  assert.ok(alarms[PASSKEY_WATCHDOG_ALARM], 'watchdog alarm must be armed while a ceremony is pending');
  assert.strictEqual(popupOpened, 1, 'the confirm popup must open');
  console.log('passkeyProxy.test.js: handlePasskeyRequest pending does not auto-complete OK');

  await completePasskeyRequest('create', { requestId: 1, responseJson: '{}' });
  assert.deepStrictEqual(completedCreate, { requestId: 1, responseJson: '{}' });
  console.log('passkeyProxy.test.js: completePasskeyRequest calls completeCreateRequest OK');

  // A second concurrent ceremony is rejected outright, and must not disturb the first's
  // bookkeeping — the pending record still belongs to requestId 1.
  resetState();
  await writePending({ kind: 'create', requestId: 1, originUrl: 'https://example.com', rpId: 'example.com', matches: [] });
  await handlePasskeyRequest('get', 2, detailsFor('https://example.com'));
  assert.strictEqual(completedGet.requestId, 2);
  assert.strictEqual(completedGet.error.name, 'AbortError');
  assert.strictEqual((await readPending()).requestId, 1, 'rejecting a second ceremony must not clear the first');
  console.log('passkeyProxy.test.js: second concurrent ceremony rejected with AbortError OK');

  // Fail closed when the payload carries no trustworthy origin.
  resetState();
  await handlePasskeyRequest('create', 3, '{"challenge":"AA"}');
  assert.strictEqual(completedCreate.error.name, 'NotAllowedError');
  assert.strictEqual(await readPending(), null);
  assert.strictEqual(sentNativeMessages.length, 0, 'nothing may be sent to the app without an origin');
  console.log('passkeyProxy.test.js: missing origin fails closed OK');

  // `locked` is the most likely real-world reply and must complete the ceremony rather
  // than fall through and hang the page until Chrome's own 180 s timeout.
  for (const [replyType, fragment] of [['locked', 'locked'], ['pairingRequired', 'not paired'], ['error', 'unavailable']]) {
    resetState();
    nativeResponder = () => ({ type: replyType, message: 'app_not_running' });
    await handlePasskeyRequest('get', 4, detailsFor('https://example.com'));
    assert.strictEqual(completedGet.requestId, 4, `${replyType} must complete the ceremony`);
    assert.strictEqual(completedGet.error.name, 'NotAllowedError');
    assert.ok(
      completedGet.error.message.includes(fragment),
      `${replyType} message should explain the cause, got "${completedGet.error.message}"`
    );
    assert.strictEqual(await readPending(), null, `${replyType} must leave no pending record`);
    assert.strictEqual(alarms[PASSKEY_WATCHDOG_ALARM], undefined, `${replyType} must not arm the watchdog`);
  }
  console.log('passkeyProxy.test.js: locked / pairingRequired / error all complete the ceremony OK');

  // An app-side error envelope on the ceremony itself is forwarded verbatim, since
  // PasskeyErrorDetails already mirrors Chrome's DOMExceptionDetails shape.
  resetState();
  nativeResponder = () => ({
    type: 'passkeyCreateComplete',
    requestId: 5,
    error: { name: 'NotSupportedError', message: 'MyPass passkeys require ES256' }
  });
  await handlePasskeyRequest('create', 5, detailsFor('https://example.com'));
  assert.deepStrictEqual(completedCreate.error, { name: 'NotSupportedError', message: 'MyPass passkeys require ES256' });
  console.log('passkeyProxy.test.js: app error details forwarded verbatim OK');

  // --- completion bookkeeping ---------------------------------------------
  resetState();
  await writePending({ kind: 'create', requestId: 1, originUrl: 'https://example.com', rpId: 'example.com', matches: [] });
  await completePasskeyRequest('create', { requestId: 99, error: passkeyNotAllowedError('other') });
  assert.strictEqual((await readPending()).requestId, 1, 'completing a different requestId must not clear the pending record');
  await completePasskeyRequest('create', { requestId: 1, responseJson: '{}' });
  assert.strictEqual(await readPending(), null);
  assert.strictEqual(alarms[PASSKEY_WATCHDOG_ALARM], undefined, 'completing must disarm the watchdog');
  console.log('passkeyProxy.test.js: completePasskeyRequest clears only its own record OK');

  // --- isUvpaa ------------------------------------------------------------
  // Every isUvpaa request MUST be answered; an early return leaves Chrome waiting.
  resetState();
  localStore.passkeyProxyEnabled = false;
  await answerIsUvpaa(11);
  assert.deepStrictEqual(completedIsUvpaa, { requestId: 11, isUvpaa: false }, 'disabled proxy must still answer');
  console.log('passkeyProxy.test.js: isUvpaa answered false when disabled OK');

  resetState();
  await answerIsUvpaa(12);
  assert.deepStrictEqual(completedIsUvpaa, { requestId: 12, isUvpaa: true });

  resetState();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: false, paired: true, attachAllowed: false });
  await answerIsUvpaa(13);
  assert.deepStrictEqual(completedIsUvpaa, { requestId: 13, isUvpaa: false }, 'attached-but-locked must answer false');

  // Unlocked but turned off app-side: attachAllowed is the authority, not `unlocked`.
  resetState();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: false });
  await answerIsUvpaa(14);
  assert.deepStrictEqual(completedIsUvpaa, { requestId: 14, isUvpaa: false });

  resetState();
  nativeResponder = () => ({ type: 'error', message: 'app_not_running' });
  await answerIsUvpaa(15);
  assert.deepStrictEqual(completedIsUvpaa, { requestId: 15, isUvpaa: false }, 'an unreachable app must still answer');
  console.log('passkeyProxy.test.js: isUvpaa reflects attach eligibility OK');

  // --- popup-driven confirm / cancel --------------------------------------
  resetState();
  await writePending({ kind: 'create', requestId: 7, originUrl: 'https://example.com', rpId: 'example.com', matches: [] });
  nativeResponder = () => ({ type: 'passkeyCreateComplete', requestId: 7, responseJson: '{"id":"abc"}' });
  await new Promise((resolve) => {
    handleRuntimeMessage({ type: 'passkeyCreateConfirm', requestId: 7, entryId: 'e1', replace: false }, {}, resolve);
  });
  assert.deepStrictEqual(completedCreate, { requestId: 7, responseJson: '{"id":"abc"}', error: undefined });
  assert.strictEqual(await readPending(), null);
  console.log('passkeyProxy.test.js: passkeyCreateConfirm completes the ceremony OK');

  // Chromium's MakeCredentialResponseFromValue requires these toJSON fields even
  // though the authenticator data and public key are also inside the CBOR object.
  // Mirror that *boundary* here, not a full WebAuthn/CBOR validator: native ceremony
  // tests independently check the byte-level fields generated by the app.
  const normalCreate = chrome.webAuthenticationProxy.completeCreateRequest;
  const completeCalls = [];
  const schemaCheckedCreate = async (details) => {
    completeCalls.push(details);
    if (details.error) return;
    const response = JSON.parse(details.responseJson).response;
    if (!response || !Number.isInteger(response.publicKeyAlgorithm)
      || typeof response.authenticatorData !== 'string' || typeof response.publicKey !== 'string') {
      throw new Error('Invalid responseJson: publicKeyAlgorithm');
    }
  };
  resetState();
  chrome.webAuthenticationProxy.completeCreateRequest = schemaCheckedCreate;
  await writePending({ kind: 'create', requestId: 70, rpId: 'example.com', matches: [] });
  alarms[PASSKEY_WATCHDOG_ALARM] = {};
  nativeResponder = () => ({ type: 'passkeyCreateComplete', requestId: 70, responseJson: JSON.stringify({
    id: 'AA', rawId: 'AA', type: 'public-key', clientExtensionResults: {},
    response: { clientDataJSON: 'AA', attestationObject: 'AA', transports: ['internal'] }
  }) });
  const rejectedCreate = await new Promise((resolve) => handleRuntimeMessage(
    { type: 'passkeyCreateConfirm', requestId: 70, replace: false }, {}, resolve));
  assert.strictEqual(completeCalls.length, 2, 'malformed native JSON must be followed by an explicit browser error');
  assert.strictEqual(completeCalls[1].error.name, 'OperationError');
  assert.strictEqual(completeCalls[1].responseJson, undefined, 'fallback must not resend malformed credential JSON');
  assert.strictEqual(rejectedCreate.error.name, 'OperationError', 'popup must not receive native success after browser rejection');
  assert.strictEqual(rejectedCreate.responseJson, undefined);
  assert.strictEqual(await readPending(), null, 'accepted error completion settles the reservation');
  assert.match(sessionStore.lastPasskeyFailure.error.message, /may already be saved in MyPass/);
  assert.strictEqual(alarms[PASSKEY_WATCHDOG_ALARM], undefined);
  assert.ok(!sessionStore.lastPasskeyFailure.error.message.includes('Invalid responseJson'), 'raw API errors stay private');

  // No early storage removal: the popup must stay on its waiting panel until the
  // browser actually acknowledges. A duplicate Save must not write another key.
  resetState();
  let acknowledgeBrowser;
  chrome.webAuthenticationProxy.completeCreateRequest = () => new Promise((resolve) => { acknowledgeBrowser = resolve; });
  await writePending({ kind: 'create', requestId: 71, rpId: 'example.com', matches: [] });
  alarms[PASSKEY_WATCHDOG_ALARM] = {};
  nativeResponder = () => ({ type: 'passkeyCreateComplete', requestId: 71, responseJson: '{}' });
  let popupResponded = false;
  const waitingCreate = new Promise((resolve) => handleRuntimeMessage(
    { type: 'passkeyCreateConfirm', requestId: 71, replace: false }, {}, (value) => { popupResponded = true; resolve(value); }));
  await new Promise((resolve) => setImmediate(resolve));
  assert.strictEqual((await readPending()).phase, 'completing');
  assert.ok(alarms[PASSKEY_WATCHDOG_ALARM]);
  assert.strictEqual(popupResponded, false);
  const duplicate = await new Promise((resolve) => handleRuntimeMessage(
    { type: 'passkeyCreateConfirm', requestId: 71, replace: false }, {}, resolve));
  assert.strictEqual(duplicate.error.name, 'NotAllowedError');
  assert.strictEqual(sentNativeMessages.filter((message) => message.type === 'passkeyCreateConfirm').length, 1);
  acknowledgeBrowser();
  assert.strictEqual((await waitingCreate).error, undefined);
  assert.strictEqual(await readPending(), null);

  // If the browser rejects BOTH the credential and the error completion, preserve
  // the watchdog rather than forgetting a possibly still-live website request.
  resetState();
  chrome.webAuthenticationProxy.completeCreateRequest = async () => { throw new Error('unavailable'); };
  await writePending({ kind: 'create', requestId: 72, phase: 'completing' });
  alarms[PASSKEY_WATCHDOG_ALARM] = {};
  assert.strictEqual((await completePasskeyRequest('create', { requestId: 72, responseJson: '{}' })).error.name, 'OperationError');
  assert.strictEqual((await readPending()).requestId, 72);
  assert.ok(alarms[PASSKEY_WATCHDOG_ALARM]);
  assert.strictEqual(sessionStore.lastPasskeyFailure.requestId, 72);
  canceledListener(72);
  await new Promise((resolve) => setImmediate(resolve));
  assert.strictEqual(await readPending(), null, 'actual Chrome cancellation can clear a failed completion');
  chrome.webAuthenticationProxy.completeCreateRequest = normalCreate;

  // The API Promise itself can reject late, after Chrome cancellation has allowed
  // another ceremony to start. Its failure must not replace the next popup panel,
  // erase the next request's bookkeeping, or disarm the shared watchdog.
  resetState();
  let rejectOldBrowserCompletion;
  chrome.webAuthenticationProxy.completeCreateRequest = (details) => details.error
    ? Promise.resolve()
    : new Promise((_resolve, reject) => { rejectOldBrowserCompletion = reject; });
  await writePending({ kind: 'create', requestId: 720, phase: 'completing' });
  const oldBrowserCompletion = completePasskeyRequest('create', { requestId: 720, responseJson: '{}' });
  await new Promise((resolve) => setImmediate(resolve));
  canceledListener(720);
  await new Promise((resolve) => setImmediate(resolve));
  await writePending({ kind: 'get', requestId: 721 });
  alarms[PASSKEY_WATCHDOG_ALARM] = { delayInMinutes: 2 };
  rejectOldBrowserCompletion(new Error('Invalid requestId'));
  assert.strictEqual((await oldBrowserCompletion).error.name, 'OperationError');
  assert.strictEqual(sessionStore.lastPasskeyFailure, undefined, 'late browser failure cannot cover a new ceremony');
  assert.strictEqual((await readPending()).requestId, 721);
  assert.deepStrictEqual(alarms[PASSKEY_WATCHDOG_ALARM], { delayInMinutes: 2 });
  chrome.webAuthenticationProxy.completeCreateRequest = normalCreate;

  // Sign-in has the same error propagation contract; the site gets a DOMException
  // and the popup receives failure rather than a false assertion success.
  resetState();
  const normalGet = chrome.webAuthenticationProxy.completeGetRequest;
  const getCalls = [];
  chrome.webAuthenticationProxy.completeGetRequest = async (details) => {
    getCalls.push(details);
    if (!details.error) throw new Error('Invalid responseJson');
  };
  await writePending({ kind: 'get', requestId: 73 });
  nativeResponder = () => ({ type: 'passkeyGetComplete', requestId: 73, responseJson: '{}' });
  const rejectedGet = await new Promise((resolve) => handleRuntimeMessage(
    { type: 'passkeyGetApprove', requestId: 73, entryId: 'e1' }, {}, resolve));
  assert.strictEqual(rejectedGet.error.name, 'OperationError');
  assert.strictEqual(getCalls.length, 2);
  assert.strictEqual(getCalls[1].error.name, 'OperationError');
  assert.strictEqual(await readPending(), null);
  chrome.webAuthenticationProxy.completeGetRequest = normalGet;
  console.log('passkeyProxy.test.js: rejected Chrome JSON settles website, preserves failure, and waits for acknowledgement OK');

  resetState();
  const nativeBeforeConfirm = chrome.runtime.sendNativeMessage;
  let lateConfirmation;
  chrome.runtime.sendNativeMessage = (_host, message, callback) => {
    if (message.type === 'passkeyCreateConfirm') lateConfirmation = callback;
    else callback({ type: 'passkeyCancelled' });
  };
  await writePending({ kind: 'create', requestId: 74 });
  const canceledConfirmation = new Promise((resolve) => handleRuntimeMessage(
    { type: 'passkeyCreateConfirm', requestId: 74, replace: false }, {}, resolve));
  await new Promise((resolve) => setImmediate(resolve));
  canceledListener(74);
  await new Promise((resolve) => setImmediate(resolve));
  await writePending({ kind: 'get', requestId: 75 });
  lateConfirmation({ type: 'passkeyCreateComplete', requestId: 74, responseJson: '{}' });
  assert.strictEqual((await canceledConfirmation).error.name, 'NotAllowedError');
  assert.strictEqual(completedCreate, null, 'a canceled native confirmation cannot be submitted to Chrome');
  assert.strictEqual((await readPending()).requestId, 75, 'a late response cannot clear the next ceremony');
  assert.strictEqual(sessionStore.lastPasskeyFailure, undefined, 'stale confirmation must not cover the next ceremony with a failure');
  chrome.runtime.sendNativeMessage = nativeBeforeConfirm;

  resetState();
  await writePending({ kind: 'create', requestId: 76 });
  nativeResponder = () => ({ type: 'passkeyCreateComplete', requestId: 999, responseJson: '{}' });
  const mismatchedConfirmation = await new Promise((resolve) => handleRuntimeMessage(
    { type: 'passkeyCreateConfirm', requestId: 76, replace: false }, {}, resolve));
  assert.strictEqual(mismatchedConfirmation.error.name, 'NotAllowedError');
  assert.strictEqual(completedCreate.requestId, 76);
  assert.strictEqual(completedCreate.responseJson, undefined, 'mismatched native response never reaches Chrome as success');
  assert.strictEqual(await readPending(), null);

  resetState();
  await writePending({ kind: 'get', requestId: 8, originUrl: 'https://example.com', rpId: 'example.com', matches: [] });
  nativeResponder = () => ({ type: 'passkeyCancelled', requestId: 8 });
  await new Promise((resolve) => {
    handleRuntimeMessage({ type: 'passkeyCancelUI', requestId: 8 }, {}, resolve);
  });
  assert.strictEqual(completedGet.error.name, 'NotAllowedError', 'cancel must complete the get ceremony, not the create one');
  assert.strictEqual(await readPending(), null);
  console.log('passkeyProxy.test.js: passkeyCancelUI cancels the pending ceremony kind OK');

  // --- settings toggle -----------------------------------------------------
  resetState();
  localStore.passkeyProxyEnabled = false;
  nativeResponder = (msg) => (msg.type === 'setPasskeyProxyEnabled'
    ? { type: 'passkeyProxyEnabled', enabled: true }
    : { type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: true, enabled: true });
  await new Promise((resolve) => {
    handleRuntimeMessage({ type: 'setPasskeyProxyEnabled', enabled: true }, {}, resolve);
  });
  assert.strictEqual(localStore.passkeyProxyEnabled, true, 'the toggle must persist locally too');
  assert.strictEqual(attachCalled, true, 'enabling attaches');

  resetState();
  nativeResponder = () => ({ type: 'passkeyProxyEnabled', enabled: false });
  await new Promise((resolve) => {
    handleRuntimeMessage({ type: 'setPasskeyProxyEnabled', enabled: false }, {}, resolve);
  });
  assert.strictEqual(localStore.passkeyProxyEnabled, false);
  assert.strictEqual(detachCalled, true, 'disabling detaches');
  console.log('passkeyProxy.test.js: setPasskeyProxyEnabled persists and attaches/detaches OK');

  // --- popup fallback ------------------------------------------------------
  resetState();
  openPopupResult = () => Promise.reject(new Error('kOpenPopupInactiveWindow'));
  nativeResponder = () => ({ type: 'passkeyGetPending', requestId: 9, rpId: 'example.com', matches: [] });
  await handlePasskeyRequest('get', 9, detailsFor('https://example.com'));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.strictEqual(notified, 1, 'a backgrounded Chrome falls back to a notification');
  assert.strictEqual((await readPending()).requestId, 9, 'the ceremony stays pending behind the notification');
  console.log('passkeyProxy.test.js: openPopup rejection falls back to a notification OK');

  // Attach failures are observable; a checked opt-in is never reported as active.
  resetState();
  const realAttach = chrome.webAuthenticationProxy.attach;
  chrome.webAuthenticationProxy.attach = () => Promise.reject(new Error('another proxy attached'));
  assert.strictEqual((await attachPasskeyProxyIfEligible()).proxyState, 'attachment-error');
  chrome.webAuthenticationProxy.attach = realAttach;
  assert.strictEqual((await attachPasskeyProxyIfEligible()).proxyState, 'active');
  nativeResponder = () => ({ type: 'pairingRequired' });
  assert.strictEqual((await attachPasskeyProxyIfEligible()).proxyState, 'unpaired');
  nativeResponder = () => ({ type: 'error' });
  assert.strictEqual((await attachPasskeyProxyIfEligible()).proxyState, 'unavailable');

  // Unlock is discovered by the periodic alarm without opening a popup or asking
  // the user to authenticate. Status probes must remain passive.
  resetState();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', enabled: true, paired: true, unlocked: false, attachAllowed: false });
  assert.strictEqual((await attachPasskeyProxyIfEligible()).proxyState, 'locked');
  nativeResponder = () => ({ type: 'passkeyProxyStatus', enabled: true, paired: true, unlocked: true, attachAllowed: true });
  alarmListener({ name: PASSKEY_REFRESH_ALARM });
  await new Promise((resolve) => setImmediate(resolve));
  assert.strictEqual(attachCalled, true);
  assert.strictEqual(popupOpened, 0, 'an unlock probe does not open the popup');
  assert.strictEqual(notified, 1, 'a lock then unlock posts one notification');
  assert.strictEqual(lastNotificationId, 'mypass-unlocked');
  assert.ok(sentNativeMessages.every((message) => ['getPasskeyProxyStatus', 'reportBrowserDiagnostics'].includes(message.type)));

  resetState();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', enabled: true, paired: true, unlocked: false, attachAllowed: false });
  await attachPasskeyProxyIfEligible();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', enabled: true, paired: true, unlocked: true, attachAllowed: true });
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(notified, 1);
  assert.strictEqual(lastNotificationId, 'mypass-unlocked');
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(notified, 1, 'staying unlocked does not notify again');
  nativeResponder = () => ({ type: 'error' });
  await attachPasskeyProxyIfEligible();
  nativeResponder = () => ({ type: 'pairingRequired' });
  await attachPasskeyProxyIfEligible();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', enabled: true, paired: true, unlocked: true, attachAllowed: true });
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(notified, 1, 'an error between two unlock readings is not a lock');
  localStore.mypassNotificationSettings = { passkeyConfirmation: true, vaultUnlocked: false };
  nativeResponder = () => ({ type: 'passkeyProxyStatus', enabled: true, paired: true, unlocked: false, attachAllowed: false });
  await attachPasskeyProxyIfEligible();
  nativeResponder = () => ({ type: 'passkeyProxyStatus', enabled: true, paired: true, unlocked: true, attachAllowed: true });
  notified = 0;
  await attachPasskeyProxyIfEligible();
  assert.strictEqual(notified, 0, 'the unlock switch can turn the notification off');
  assert.strictEqual(popupOpened, 0);
  delete localStore.mypassNotificationSettings;

  resetState();
  localStore.mypassNotificationSettings = { passkeyConfirmation: false, vaultUnlocked: true };
  openPopupResult = () => Promise.reject(new Error('kOpenPopupInactiveWindow'));
  nativeResponder = () => ({ type: 'passkeyGetPending', requestId: 11, rpId: 'example.com', matches: [] });
  await handlePasskeyRequest('get', 11, detailsFor('https://example.com'));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.strictEqual(popupOpened, 1, 'the passkey switch does not skip the popup');
  assert.strictEqual(notified, 0, 'the passkey switch can turn the fallback notification off');
  delete localStore.mypassNotificationSettings;

  resetState();
  openPopupResult = () => Promise.reject(new Error('kOpenPopupInactiveWindow'));
  const openedBeforeClick = popupOpened;
  notificationClickListener('mypass-unlocked');
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.strictEqual(popupOpened, openedBeforeClick + 1);
  assert.strictEqual(notified, 0, 'clicking the unlock notification does not post another one');

  // A refused setting mutation cannot detach the proxy or rewrite the mirror.
  resetState();
  nativeResponder = () => ({ type: 'locked' });
  await new Promise((resolve) => handleRuntimeMessage({ type: 'setPasskeyProxyEnabled', enabled: false }, {}, resolve));
  assert.strictEqual(detachCalled, false);
  assert.strictEqual(localStore.passkeyProxyEnabled, true);

  // Two truly concurrent requests reserve exactly one slot before native delivery.
  resetState();
  nativeResponder = (message) => ({ type: message.type === 'passkeyCreate' ? 'passkeyCreatePending' : 'passkeyGetPending', rpId: 'example.com', matches: [] });
  await Promise.all([handlePasskeyRequest('create', 20, detailsFor('https://example.com')), handlePasskeyRequest('get', 21, detailsFor('https://example.com'))]);
  assert.strictEqual((await readPending()).requestId, 20);
  assert.strictEqual(completedGet.error.name, 'AbortError');
  assert.strictEqual(sentNativeMessages.filter((message) => message.type === 'passkeyGet').length, 0);

  // Cancel while the native app is still preparing a ceremony: its late response
  // cannot reopen the popup or replace another request's pending record.
  resetState();
  const normalNative = chrome.runtime.sendNativeMessage;
  let finishNative;
  chrome.runtime.sendNativeMessage = (_host, message, callback) => {
    if (message.type === 'passkeyCreate') finishNative = callback;
    else callback({ type: 'passkeyCancelled' });
  };
  const inFlight = handlePasskeyRequest('create', 22, detailsFor('https://example.com'));
  await new Promise((resolve) => setImmediate(resolve));
  assert.strictEqual((await readPending()).phase, 'checking');
  canceledListener(22);
  await new Promise((resolve) => setImmediate(resolve));
  await writePending({ kind: 'get', requestId: 23 });
  finishNative({ type: 'passkeyCreatePending', rpId: 'example.com', matches: [] });
  await inFlight;
  assert.strictEqual((await readPending()).requestId, 23);
  assert.strictEqual(popupOpened, 0);
  chrome.runtime.sendNativeMessage = normalNative;

  // Diagnostics are opt-in and do not classify arbitrary installed extensions.
  resetState();
  assert.strictEqual((await getBrowserManagerDiagnostics()).checked, false);
  chrome.permissions = { contains: async () => true };
  chrome.runtime.id = 'mypass';
  chrome.privacy = { services: { passwordSavingEnabled: { get: async () => ({ value: true }) } } };
  chrome.management = { getAll: async () => [
    { id: 'mypass', enabled: true, name: 'MyPass', permissions: ['webAuthenticationProxy'] },
    { id: 'unrelated', enabled: true, name: 'Reader', permissions: ['storage'] },
    { id: 'proxy', enabled: true, name: 'Other passkey provider', permissions: ['webAuthenticationProxy'] },
    { id: 'nngceckbapebfimnlniiiahkandclblb', enabled: true, name: 'Bitwarden', permissions: [] },
    { id: 'disabled', enabled: false, name: 'Disabled provider', permissions: ['webAuthenticationProxy'] }
  ] };
  const diagnostics = await getBrowserManagerDiagnostics();
  assert.strictEqual(diagnostics.passwordSavingEnabled, true);
  assert.deepStrictEqual(diagnostics.extensions.map((entry) => entry.name), ['Other passkey provider', 'Bitwarden']);
  // Stable iCloud and 1Password extensions need no proxy permission to be detected.
  // Browser password saving is the observable Google Password Manager setting;
  // neither value establishes whether its passkeys or autofill are enabled.
  for (const passwordSavingEnabled of [true, false]) {
    chrome.privacy.services.passwordSavingEnabled.get = async () => ({ value: passwordSavingEnabled });
    for (const enabled of [true, false]) {
      chrome.management.getAll = async () => [
        { id: 'pejdijmoenmkgeppbflobdenhhabjlaj', enabled, name: 'iCloud Passwords', permissions: [] },
        { id: 'aeblfdkhhhdcdjpifhhbdiojplfjncoa', enabled, name: '1Password', permissions: [] },
        { id: 'mypass', enabled: true, name: 'MyPass', permissions: ['webAuthenticationProxy'] }
      ];
      const observed = await getBrowserManagerDiagnostics();
      assert.strictEqual(observed.checked, true);
      assert.strictEqual(observed.passwordSavingEnabled, passwordSavingEnabled);
      assert.deepStrictEqual(observed.extensions.map((entry) => entry.name), enabled ? ['iCloud Passwords', '1Password'] : []);
    }
  }
  chrome.privacy.services.passwordSavingEnabled.get = async () => { throw new Error('setting unavailable'); };
  assert.strictEqual((await getBrowserManagerDiagnostics()).passwordSavingEnabled, null);
  delete chrome.privacy;
  assert.strictEqual((await getBrowserManagerDiagnostics()).passwordSavingEnabled, null);
  chrome.management.getAll = async () => { throw new Error('inspection unavailable'); };
  assert.strictEqual((await getBrowserManagerDiagnostics()).checked, false);
  delete chrome.management;
  assert.strictEqual((await getBrowserManagerDiagnostics()).checked, false);
  chrome.permissions.contains = async () => false;
  assert.strictEqual((await getBrowserManagerDiagnostics()).checked, false);
  chrome.permissions.contains = async () => { throw new Error('permission unavailable'); };
  assert.strictEqual((await getBrowserManagerDiagnostics()).checked, false);
  delete chrome.permissions;
  delete chrome.management;
  delete chrome.privacy;

  // Content scripts cannot approve user-presence ceremonies or change the app toggle.
  for (const type of ['passkeyCreateConfirm', 'passkeyGetApprove', 'passkeyCancelUI', 'setPasskeyProxyEnabled']) {
    assert.strictEqual(handleRuntimeMessage({ type }, { tab: { id: 1 } }, () => {}), false);
  }
  console.log('passkeyProxy.test.js: status, passive recovery, concurrency, cancellation and diagnostics OK');
  console.log('passkeyProxy.test.js OK');
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
