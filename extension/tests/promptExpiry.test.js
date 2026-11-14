const assert = require('node:assert');

// Secret lifecycle of pending prompts: an expired or answered record must be DELETED from
// chrome.storage.session (not merely rejected when read), the "!" badge goes with the last
// record, an alarm (not a setTimeout) wakes a suspended worker to do the deleting, and the
// content script's dedupe state never holds the raw password.

let session = {};
const local = {};
let badgeText = '';
const alarms = {};
let alarmListener = null;
const alarmClears = [];
// Popup and window both refuse to open, so the "!" badge is the surface that says a prompt waits.
const behavior = { surfacesFail: true };

function area(store) {
  return {
    get: (keys, cb) => {
      const out = {};
      for (const k of (Array.isArray(keys) ? keys : [keys])) if (store[k] !== undefined) out[k] = store[k];
      cb(out);
    },
    set: (items, cb) => { Object.assign(store, items); if (cb) cb(); },
    remove: (keys, cb) => { for (const k of (Array.isArray(keys) ? keys : [keys])) delete store[k]; if (cb) cb(); }
  };
}

global.chrome = {
  storage: { local: area(local), session: { get: (k, cb) => area(session).get(k, cb), set: (i, cb) => area(session).set(i, cb), remove: (k, cb) => area(session).remove(k, cb) } },
  runtime: {
    id: 'ext',
    lastError: null,
    getURL: (p) => `chrome-extension://ext/${p}`,
    onMessage: { addListener: () => {} },
    sendNativeMessage: (_host, message, cb) => cb({ type: 'saveProposalAccepted' })
  },
  action: {
    setBadgeText: ({ text }) => { badgeText = text; return Promise.resolve(); },
    getBadgeText: () => Promise.resolve(badgeText),
    openPopup: () => (behavior.surfacesFail ? Promise.reject(new Error('No active browser window')) : Promise.resolve())
  },
  tabs: {
    get: async (id) => ({ id, active: true, windowId: 3 }),
    sendMessage: (_tabId, _msg, _opts, cb) => cb({ type: 'ok', url: 'https://login.example.com/signin' }),
    onRemoved: { addListener: () => {} }
  },
  windows: { get: async () => ({ focused: true }), create: async () => { if (behavior.surfacesFail) throw new Error('no windows'); return { id: 101 }; }, update: async () => {}, onRemoved: { addListener: () => {} } },
  notifications: { create: () => Promise.resolve() },
  alarms: {
    create: (name, info) => { alarms[name] = info; },
    clear: (name) => { alarmClears.push(name); delete alarms[name]; },
    onAlarm: { addListener: (fn) => { alarmListener = fn; } }
  }
};

const bg = require('../background.js');
const content = require('../content.js');
const records = require('../shared/prompt-record.js');

const PASSWORD = 'hunter2-SECRET';
const SEED = 'JBSWY3DPEHPK3PXP';
const KEY = records.PROMPT_STORAGE_KEY;
const loginRequest = { type: 'requestPrompt', kind: 'saveLogin', url: 'https://login.example.com/signin', username: 'alice', password: PASSWORD };
const totpRequest = { type: 'requestPrompt', kind: 'saveTotp', url: 'https://login.example.com/signin', otpauthOrSecret: SEED };
const contentSender = (tabId = 7, documentId = 'DOC-1') => ({
  id: 'ext', url: 'https://login.example.com/signin', frameId: 0, documentId, tab: { id: tabId, windowId: 3 }
});
const uiSender = { id: 'ext', url: 'chrome-extension://ext/popup.html' };

function dispatch(message, sender) {
  return new Promise((resolve) => {
    const kept = bg.handleRuntimeMessage(message, sender, resolve);
    if (!kept) resolve(undefined);
  });
}
const settle = () => new Promise((resolve) => setTimeout(resolve, 20));
function reset() {
  session = {};
  badgeText = '';
  for (const k of Object.keys(alarms)) delete alarms[k];
  alarmClears.length = 0;
}
const realNow = Date.now;
const at = (ms) => { Date.now = () => ms; };

(async () => {
  assert.strictEqual(bg.PROMPT_EXPIRY_ALARM, 'mypass-prompt-expiry');
  assert.strictEqual(typeof alarmListener, 'function', 'the worker listens for the expiry alarm at load');

  // --- creating a record stores the secret and arms the expiry alarm for its expiresAt ---
  const T0 = realNow();
  at(T0);
  assert.strictEqual((await dispatch(loginRequest, contentSender())).created, true);
  let stored = session[KEY];
  assert.strictEqual(stored.length, 1);
  assert.strictEqual(stored[0].payload.password, PASSWORD, 'the record is the one place that holds the secret');
  assert.strictEqual(alarms[bg.PROMPT_EXPIRY_ALARM].when, stored[0].expiresAt, 'alarm armed for the soonest expiry');
  assert.strictEqual(badgeText, '!', 'badge says a prompt is waiting');

  // --- reading after expiry DELETES the record from storage, clears the badge and the alarm ---
  at(T0 + 120001);
  assert.deepStrictEqual(await bg.readPrompts(), [], 'expired records are not returned');
  assert.strictEqual(session[KEY], undefined, 'and the expired password is gone from storage, not just filtered');
  assert.ok(!JSON.stringify(session).includes(PASSWORD));
  await settle();
  assert.strictEqual(badgeText, '', 'the last record leaving clears the "!" badge');
  assert.strictEqual(alarms[bg.PROMPT_EXPIRY_ALARM], undefined, 'no record left, no alarm');

  // --- the alarm path: a worker woken by the alarm deletes the expired record with no other call ---
  reset();
  at(T0);
  await dispatch(loginRequest, contentSender());
  await dispatch(totpRequest, contentSender());
  assert.strictEqual(session[KEY].length, 2);
  at(T0 + 120001);
  alarmListener({ name: 'some-other-alarm' });
  await settle();
  assert.strictEqual(session[KEY].length, 2, 'an unrelated alarm sweeps nothing');
  alarmListener({ name: bg.PROMPT_EXPIRY_ALARM });
  await settle();
  assert.strictEqual(session[KEY], undefined, 'the expiry alarm removes every expired record from storage');
  assert.ok(!JSON.stringify(session).includes(PASSWORD) && !JSON.stringify(session).includes(SEED), 'no stale secrets remain');
  assert.strictEqual(badgeText, '', 'and the badge goes with the last one');

  // --- a partly expired list keeps the live record and re-arms the alarm for it ---
  reset();
  at(T0);
  await dispatch(loginRequest, contentSender(7));
  at(T0 + 60000);
  await dispatch(loginRequest, contentSender(8, 'DOC-2'));
  assert.strictEqual(session[KEY].length, 2);
  at(T0 + 120001);
  alarmListener({ name: bg.PROMPT_EXPIRY_ALARM });
  await settle();
  assert.strictEqual(session[KEY].length, 1, 'only the expired record was deleted');
  assert.strictEqual(session[KEY][0].tabId, 8);
  assert.strictEqual(alarms[bg.PROMPT_EXPIRY_ALARM].when, T0 + 60000 + 120000, 're-armed for the surviving record');
  assert.strictEqual(badgeText, '!', 'the badge stays while a prompt is still waiting');
  at(T0 + 60000 + 120001);
  await bg.sweepExpiredPrompts();
  assert.strictEqual(session[KEY], undefined, 'the worker-start / resume sweep deletes the last one too');
  await settle();
  assert.strictEqual(badgeText, '');

  // --- a corrupt value is removed, not kept ---
  session[KEY] = { not: 'a list' };
  at(T0);
  assert.deepStrictEqual(await bg.readPrompts(), []);
  assert.strictEqual(session[KEY], undefined);

  // --- save and dismiss delete the record and leave no secret behind ---
  for (const action of ['dismiss', 'save']) {
    reset();
    at(T0);
    await dispatch(loginRequest, contentSender());
    const id = session[KEY][0].id;
    await dispatch({ type: 'consumePrompt', promptId: id, action }, uiSender);
    assert.strictEqual(session[KEY], undefined, action + ' removes the record from storage');
    assert.ok(!JSON.stringify(session).includes(PASSWORD), action + ' leaves no password in session storage');
    await settle();
    assert.strictEqual(badgeText, '', action + ' clears the badge');
    assert.strictEqual(alarms[bg.PROMPT_EXPIRY_ALARM], undefined, action + ' clears the alarm');
  }

  // --- a pending retrieval after expiry returns nothing ---
  reset();
  at(T0);
  await dispatch(loginRequest, contentSender());
  at(T0 + 120001);
  const pending = await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender);
  assert.ok(!JSON.stringify(pending || null).includes(PASSWORD), 'no stale secret in a late retrieval');
  assert.strictEqual(session[KEY], undefined, 'and the late retrieval itself persists the cleanup');

  Date.now = realNow;

  // --- content script: dedupe state never holds the raw password ---
  const state = { lastLoginPrompt: null };
  const sent = [];
  const send = (m) => sent.push(m);
  const realSetTimeout = global.setTimeout;
  const timers = [];
  global.setTimeout = (fn, ms) => { timers.push({ fn, ms }); return { unref() {} }; };
  assert.strictEqual(content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, state, send, 1000), true);
  const firstKey = state.lastLoginPrompt.key;
  assert.ok(!JSON.stringify(state).includes(PASSWORD), 'no plaintext password in the dedupe state');
  assert.ok(!JSON.stringify(state).includes('hunter2'), 'not even a fragment of it');
  assert.strictEqual(content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, state, send, 2000), false, 'same credentials still deduped');
  assert.strictEqual(content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD + '!', state, send, 3000), true, 'a changed password is still a new offer');
  assert.ok(!JSON.stringify(state).includes(PASSWORD), 'still none after a changed password');
  assert.strictEqual(sent.length, 2);
  assert.strictEqual(timers.length, 2);
  assert.strictEqual(timers[0].ms, 120000, 'the entry lives as long as the prompt does');
  timers[0].fn();
  assert.notStrictEqual(state.lastLoginPrompt, null, 'an older submit\'s timer does not drop the newer entry');
  timers[1].fn();
  assert.strictEqual(state.lastLoginPrompt, null, 'the entry is dropped when its lifetime is over');
  const other = { lastLoginPrompt: null };
  content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, other, send, 1000);
  assert.notStrictEqual(other.lastLoginPrompt.key, firstKey, 'separate documents have separate salted fingerprints');

  // Rejected requests must not suppress a later retry, and old replies must not clear a newer entry.
  for (const response of [undefined, { type: 'error' }, { type: 'ok', created: false }]) {
    const rejected = { lastLoginPrompt: null };
    const reject = (_message, callback) => callback(response);
    assert.strictEqual(content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, rejected, reject, 1000), true);
    assert.strictEqual(rejected.lastLoginPrompt, null, 'failed creation clears its dedupe entry');
    assert.strictEqual(content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, rejected, reject, 2000), true, 'retry is allowed');
  }
  // A reply that never came sets runtime.lastError; the callback must read it (so Chrome does not
  // log "Unchecked runtime.lastError") and still treat the prompt as not created, so a retry works.
  const hadChrome = Object.prototype.hasOwnProperty.call(global, 'chrome');
  const previousChrome = global.chrome;
  let lastErrorReads = 0;
  global.chrome = { runtime: { get lastError() { lastErrorReads += 1; return { message: 'The message port closed before a response was received.' }; } } };
  try {
    const lost = { lastLoginPrompt: null };
    const noReply = (_message, callback) => callback(undefined);
    assert.strictEqual(content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, lost, noReply, 1000), true);
    assert.strictEqual(lastErrorReads, 1, 'the missing-response callback consumes runtime.lastError');
    assert.strictEqual(lost.lastLoginPrompt, null, 'a missing response still clears the dedupe entry');
    assert.strictEqual(content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, lost, noReply, 2000), true, 'and the retry is still allowed');
  } finally {
    if (hadChrome) global.chrome = previousChrome; else delete global.chrome;
  }
  const pendingState = { lastLoginPrompt: null };
  const replies = [];
  const pendingSend = (_message, callback) => replies.push(callback);
  content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD, pendingState, pendingSend, 1000);
  content.requestSavePrompt('https://login.example.com/', 'alice', PASSWORD + '!', pendingState, pendingSend, 2000);
  const latest = pendingState.lastLoginPrompt;
  replies[0]({ type: 'error' });
  assert.strictEqual(pendingState.lastLoginPrompt, latest, 'a late failure cannot clear a newer submission');
  replies[1]({ type: 'ok', created: true });
  assert.strictEqual(pendingState.lastLoginPrompt, latest, 'successful creation retains deduplication');
  global.setTimeout = realSetTimeout;

  console.log('promptExpiry.test.js: expired/answered secrets are deleted, alarm sweep, dedupe holds no password OK');
})().catch((error) => { Date.now = realNow; console.error(error); process.exit(1); });
