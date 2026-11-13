const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const html = fs.readFileSync(path.join(__dirname, '..', 'popup.html'), 'utf8');
const source = fs.readFileSync(path.join(__dirname, '..', 'popup.js'), 'utf8');

// --- popup.html: the locked panel is only the key, title, subtitle and Unlock. ---
const start = html.indexOf('<section id="locked-panel"');
assert.ok(start >= 0, 'locked panel exists');
const block = html.slice(start, html.indexOf('</section>', start));
assert.ok(block.includes('id="locked-unlock-btn"'), 'unlock button present');
assert.ok(!block.includes('locked-retry-btn'), 'no retry button');
assert.ok(!/<input/i.test(block), 'no input in locked panel');
assert.ok(!/<svg/i.test(block), 'no svg (gear) in locked panel');
assert.ok(!/settings/i.test(block), 'no settings text in locked panel');
assert.ok(!/id="locked-unlock-btn"[^>]*\bhidden\b/.test(block), 'unlock button always visible');

assert.ok(!source.includes('lockedRetryBtn'), 'popup.js no longer binds a retry button');

// --- Fake DOM harness (same style as popup-fallback.test.js). ---
const elements = {};
function makeEl(id) {
  const el = {
    id, hidden: false, textContent: '', innerHTML: '', value: '', onclick: null,
    className: '', style: {}, children: [], disabled: false,
    appendChild(child) { this.children.push(child); },
    addEventListener() {},
    setAttribute(name, value) { this[name] = value; }
  };
  elements[id] = el;
  return el;
}
global.document = {
  getElementById: (id) => elements[id] || makeEl(id),
  createElement: (tag) => makeEl(tag)
};
const windowListeners = {};
global.addEventListener = (name, fn) => { windowListeners[name] = fn; };
global.window = global;

// Fake timers: record registrations so leaks are observable.
const intervals = [];
const cleared = [];
global.setInterval = (fn, delay) => { const h = { fn, delay }; intervals.push(h); return h; };
global.clearInterval = (h) => { cleared.push(h); };

let nextResponse = { type: 'locked' };
let nextUnlockReply = null;
const sent = [];
global.chrome = {
  storage: {
    local: { get: (_k, cb) => cb({}), set: (_i, cb) => cb && cb(), remove: (_k, cb) => cb && cb() },
    session: null
  },
  runtime: {
    sendMessage: (msg, cb) => {
      sent.push(msg);
      if (msg && msg.type === 'getCredentials') { cb(nextResponse); return; }
      if (msg && msg.type === 'requestUnlock') { cb(nextUnlockReply); return; }
      if (cb) cb({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: false });
    }
  },
  tabs: { query: (_o, cb) => cb([{ url: 'https://example.com' }]) }
};

const popup = require('../popup.js');
const {
  LOCKED_TITLE_TEXT, LOCKED_HELPER_TEXT, UNLOCK_BUTTON_LABEL,
  LOCKED_POLL_INTERVAL_MS, startLockedPolling, stopLockedPolling
} = popup;

assert.strictEqual(LOCKED_TITLE_TEXT, 'MyPass is locked');
assert.strictEqual(LOCKED_HELPER_TEXT, 'Open MyPass and unlock the vault');
assert.strictEqual(UNLOCK_BUTTON_LABEL, 'Unlock MyPass');
assert.strictEqual(LOCKED_POLL_INTERVAL_MS, 2000);

// The popup init already presented the locked panel (getCredentials -> locked).
assert.strictEqual(elements['locked-panel'].hidden, false, 'locked panel shown');
assert.strictEqual(elements['add-totp-section'].hidden, true, 'add-totp hidden when locked');
assert.strictEqual(elements['browser-chrome'].hidden, true, 'locked view shows no menu button at all');
const afterInit = intervals.length;
assert.strictEqual(afterInit, 1, 'showing the locked panel registers one poll');
assert.strictEqual(intervals[0].delay, 2000);

// Repeated starts do not stack timers.
let calls = 0;
startLockedPolling(() => { calls += 1; });
startLockedPolling(() => { calls += 1; });
assert.strictEqual(intervals.length, afterInit, 'start is a no-op while running');

stopLockedPolling();
assert.strictEqual(cleared.length, 1, 'stop clears the single timer');
stopLockedPolling();
assert.strictEqual(cleared.length, 1, 'stop when idle does not clear again');

startLockedPolling(() => { calls += 1; });
assert.strictEqual(intervals.length, afterInit + 1, 'start after stop registers a new timer');
intervals[intervals.length - 1].fn();
assert.strictEqual(calls, 1, 'tick invokes reload');
stopLockedPolling();

// --- A poll tick must not run while the Unlock wait is polling (sendBrowse would cancel it). ---
{
  const browseCalls = () => sent.filter((m) => m.type === 'getCredentials').length;
  const timers = [];
  const realSetTimeout = global.setTimeout;
  global.setTimeout = (fn, d) => { timers.push({ fn, d }); return timers.length; };
  const tick = intervals[0].fn;
  nextUnlockReply = { type: 'unlockPresented' };
  elements['locked-unlock-btn'].onclick();
  assert.strictEqual(elements['locked-unlock-btn'].disabled, true, 'unlock wait is running');
  const waiting = elements['locked-helper'] ? elements['locked-helper'].textContent : null;
  nextResponse = { type: 'locked' };
  const before = browseCalls();
  tick();
  assert.strictEqual(browseCalls(), before, 'a locked poll tick does not reload while the unlock wait runs');
  assert.strictEqual(elements['locked-unlock-btn'].disabled, true, 'the wait is not cancelled by the tick');
  if (waiting !== null) assert.strictEqual(elements['locked-helper'].textContent, waiting, 'waiting text kept');
  // End the wait (the unlock "succeeds") so later sections start clean.
  nextResponse = { type: 'credentialList', credentials: [{ id: '1', title: 'Example', username: 'a' }] };
  timers[timers.length - 1].fn();
  global.setTimeout = realSetTimeout;
  nextResponse = { type: 'locked' };
  tick();
  assert.strictEqual(elements['locked-panel'].hidden, false, 'locked again after the wait ended');
}

// --- Failed unlock request shows a short error; the default helper returns on next present. ---
{
  const tick = intervals[0].fn;
  elements['locked-unlock-btn'].disabled = false; // the previous block's wait is abandoned
  nextUnlockReply = null;
  elements['locked-unlock-btn'].onclick();
  assert.strictEqual(elements['locked-helper'].textContent, 'Could not reach MyPass. Open the app and try again.');
  assert.strictEqual(elements['locked-unlock-btn'].disabled, false, 'button stays usable after a failure');
  nextResponse = { type: 'credentialList', credentials: [{ id: '1', title: 'Example', username: 'a' }] };
  tick();
  nextResponse = { type: 'locked' };
  tick();
  assert.strictEqual(elements['locked-helper'].textContent, LOCKED_HELPER_TEXT, 'default helper restored on next present');
}

// --- Leaving the locked state stops the poll (real popup tick, not a stub). ---
const popupTick = intervals[0].fn;
function lockAgain() {
  nextResponse = { type: 'locked' };
  popupTick(); // reload -> locked -> showPanel('locked') starts polling again
}
lockAgain();
let before = cleared.length;
nextResponse = { type: 'credentialList', credentials: [{ id: '1', title: 'Example', username: 'a' }] };
popupTick();
assert.strictEqual(elements['list-panel'].hidden, false, 'credential list shown after unlock');
assert.strictEqual(cleared.length, before + 1, 'credential list stops the poll');

lockAgain();
before = cleared.length;
nextResponse = { type: 'credentialList', credentials: [] };
popupTick();
assert.strictEqual(elements['empty-panel'].hidden, false, 'empty panel shown');
assert.strictEqual(cleared.length, before + 1, 'empty panel stops the poll');

lockAgain();
const registered = intervals.length;
lockAgain(); // a tick while locked must not stack timers
assert.strictEqual(intervals.length, registered, 'repeated locked panels keep one timer');

before = cleared.length;
assert.strictEqual(typeof windowListeners.pagehide, 'function', 'pagehide listener registered');
windowListeners.pagehide();
assert.strictEqual(cleared.length, before + 1, 'pagehide stops the poll');

console.log('lockedPopup.test.js passed');
