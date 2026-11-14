const assert = require('node:assert');

// Same DOM stub shape as popup-fallback.test.js: getElementById mints elements on
// demand, so the popup's whole top-level wiring block runs under Node.
const elements = {};
function makeEl(id) {
  const el = {
    id,
    hidden: false,
    textContent: '',
    value: '',
    checked: false,
    disabled: false,
    onclick: null,
    className: '',
    style: {},
    children: [],
    listeners: {},
    appendChild(child) { this.children.push(child); },
    addEventListener(name, fn) { (this.listeners[name] = this.listeners[name] || []).push(fn); },
    setAttribute(name, value) { this[name] = value; }
  };
  let innerHTML = '';
  // Real DOM `.innerHTML = ''` (what renderPasskeyPicker uses to clear the list before
  // re-rendering) tears down existing children too — without mirroring that, `.children`
  // from an earlier render call would linger and be picked up by a later test's row-click
  // assertions, attributing an old, unrelated click handler to a "new" row.
  Object.defineProperty(el, 'innerHTML', {
    get() { return innerHTML; },
    set(value) {
      innerHTML = value;
      if (value === '') el.children = [];
    }
  });
  elements[id] = el;
  return el;
}

global.document = {
  getElementById: (id) => elements[id] || makeEl(id),
  createElement: () => makeEl(`created-${Object.keys(elements).length}`)
};

// The pending ceremony written by background.js lives in chrome.storage.session, which
// is what getPairingStorageArea() prefers.
let sessionStore = {};
let pendingStorageListener;
const sentMessages = [];
let settingsOpenResponse = null;
let closed = 0;
global.window = { close: () => { closed += 1; } };

global.chrome = {
  storage: {
    onChanged: { addListener: (listener) => { pendingStorageListener = listener; } },
    local: {
      get: (_keys, cb) => cb({ passkeyProxyEnabled: true }),
      set: (_items, cb) => cb && cb(),
      remove: (_keys, cb) => cb && cb()
    },
    session: {
      get: (keys, cb) => {
        const result = {};
        for (const key of (Array.isArray(keys) ? keys : [keys])) {
          if (sessionStore[key] !== undefined) result[key] = sessionStore[key];
        }
        cb(result);
      },
      set: (items, cb) => { Object.assign(sessionStore, items); if (cb) cb(); },
      remove: (keys, cb) => {
        for (const key of (Array.isArray(keys) ? keys : [keys])) delete sessionStore[key];
        if (cb) cb();
      }
    }
  },
  runtime: {
    sendMessage: (msg, cb) => {
      sentMessages.push(msg);
      if (cb) cb(msg.type === 'openBrowserSettings' && settingsOpenResponse
        ? settingsOpenResponse
        : { type: msg.type === 'passkeyCreateConfirm' ? 'passkeyCreateComplete' : msg.type === 'passkeyGetApprove' ? 'passkeyGetComplete' : 'ok' });
    }
  },
  tabs: { query: (_opts, cb) => cb([{ url: 'https://example.com', id: 1 }]) }
};

const {
  choosePasskeyPanel,
  passkeyCeremonySummary,
  passkeyTargetSummary,
  renderPasskeyCeremonyPanel,
  rehydratePendingPasskeyCeremony,
  sendPasskeyCancel,
  PENDING_PASSKEY_KEY,
  initPasskeySettingsToggle,
  initPasskeyPopupUI,
  renderCredentials
} = require('../popup.js');

// --- pure routing -----------------------------------------------------------
const withPasskey = { id: 'e1', title: 'Example', username: 'alice', hasPasskey: true };
const withoutPasskey = { id: 'e2', title: 'Example', username: 'bob', hasPasskey: false };

assert.strictEqual(choosePasskeyPanel({ kind: 'create', matches: [] }), 'passkey-save');
assert.strictEqual(choosePasskeyPanel({ kind: 'create', matches: [withoutPasskey] }), 'passkey-save');
assert.strictEqual(choosePasskeyPanel({ kind: 'create', matches: [withPasskey] }), 'passkey-replace');
assert.strictEqual(choosePasskeyPanel({ kind: 'create', matches: [withPasskey, withoutPasskey] }), 'passkey-picker');
assert.strictEqual(choosePasskeyPanel({ kind: 'get', matches: [withPasskey] }), 'passkey-use');
assert.strictEqual(choosePasskeyPanel({ kind: 'get', matches: [withPasskey, withoutPasskey] }), 'passkey-picker');
assert.strictEqual(choosePasskeyPanel({ kind: 'get', matches: [] }), null);
assert.strictEqual(choosePasskeyPanel(null), null);
assert.strictEqual(choosePasskeyPanel({ kind: 'nonsense', matches: [] }), null);
console.log('passkeyPopup.test.js: choosePasskeyPanel routing OK');

// A single get match must still land on the Approve panel — never an auto-sign path.
assert.notStrictEqual(choosePasskeyPanel({ kind: 'get', matches: [withPasskey] }), null);

// --- summary carries both RP id and origin (phishing resistance) -------------
assert.strictEqual(
  passkeyCeremonySummary({ rpId: 'example.com', originUrl: 'https://login.example.com' }),
  'example.com — https://login.example.com'
);
assert.ok(passkeyCeremonySummary({ rpId: 'example.com' }).includes('unknown origin'));
assert.strictEqual(passkeyCeremonySummary(null), '');
console.log('passkeyPopup.test.js: summary shows rpId and origin OK');

// --- passkeyTargetSummary: which entry is the save/replace target -----------
const basePending = { rpId: 'example.com', originUrl: 'https://example.com' };

// Gate 6 regression: zero matches must read as "a new entry will be created", not the
// bare rpId/origin pair that looked identical to the 1-match case.
assert.deepStrictEqual(passkeyTargetSummary({ ...basePending, matches: [] }), {
  headline: 'New entry for example.com',
  detail: 'example.com — https://example.com'
});

// One match with a username: headline names both the entry and the account on it.
assert.deepStrictEqual(passkeyTargetSummary({ ...basePending, matches: [withoutPasskey] }), {
  headline: 'Save passkey to "Example" (bob)?',
  detail: 'example.com — https://example.com'
});

// One match without a username: the "()" segment is omitted rather than left empty.
const noUsername = { id: 'e3', title: 'No Username Entry', username: '', hasPasskey: false };
assert.deepStrictEqual(passkeyTargetSummary({ ...basePending, matches: [noUsername] }), {
  headline: 'Save passkey to "No Username Entry"?',
  detail: 'example.com — https://example.com'
});

// One match that already has a passkey: Replace wording, not Save.
assert.deepStrictEqual(passkeyTargetSummary({ ...basePending, matches: [withPasskey] }), {
  headline: 'Replace passkey in "Example" (alice)?',
  detail: 'example.com — https://example.com'
});

// The picker path passes an explicit override (the row the user clicked) rather than
// relying on matches[0], since the picker's own matches list always has more than one.
assert.deepStrictEqual(
  passkeyTargetSummary({ ...basePending, matches: [withPasskey, withoutPasskey] }, withoutPasskey),
  { headline: 'Save passkey to "Example" (bob)?', detail: 'example.com — https://example.com' }
);

// The get flow was left behind when the create flow got a named headline: a single get
// match must also name the entry ("Sign in as ..."), not just repeat the bare rpId/origin.
assert.deepStrictEqual(passkeyTargetSummary({ ...basePending, kind: 'get', matches: [withPasskey] }), {
  headline: 'Sign in as "Example" (alice)?',
  detail: 'example.com — https://example.com'
});
const noUsernameWithPasskey = { id: 'e4', title: 'No Username Passkey', username: '', hasPasskey: true };
assert.deepStrictEqual(passkeyTargetSummary({ ...basePending, kind: 'get', matches: [noUsernameWithPasskey] }), {
  headline: 'Sign in as "No Username Passkey"?',
  detail: 'example.com — https://example.com'
});
console.log('passkeyPopup.test.js: passkeyTargetSummary headline/detail OK');

// --- rendering --------------------------------------------------------------
function resetPanels() {
  for (const el of Object.values(elements)) el.hidden = false;
  sentMessages.length = 0;
  closed = 0;
}

resetPanels();
assert.strictEqual(
  renderPasskeyCeremonyPanel({ kind: 'create', requestId: 1, rpId: 'example.com', originUrl: 'https://example.com', matches: [] }),
  true
);
assert.strictEqual(elements['passkey-save-panel'].hidden, false);
assert.strictEqual(elements['passkey-replace-panel'].hidden, true);
assert.strictEqual(elements['list-panel'].hidden, true, 'a ceremony hides the ordinary credential list');
assert.strictEqual(elements['add-totp-section'].hidden, true, 'a ceremony hides the Add-2FA section');
assert.strictEqual(elements['popup-menu'].hidden, true, 'a ceremony closes the extension menu');
assert.strictEqual(elements['browser-chrome'].hidden, true, 'a ceremony hides the menu button');
assert.strictEqual(elements['passkey-save-summary'].textContent, 'A new entry will be created.');
assert.deepStrictEqual(elements['passkey-save-detail'].children.slice(-4).map((el) => el.textContent), ['Website', 'example.com', 'Origin', 'https://example.com']);
console.log('passkeyPopup.test.js: create panel renders and owns the popup OK');

resetPanels();
renderPasskeyCeremonyPanel({ kind: 'create', requestId: 1.5, rpId: 'example.com', originUrl: 'https://example.com', matches: [withoutPasskey] });
assert.strictEqual(elements['passkey-save-panel'].hidden, false);
assert.deepStrictEqual(elements['passkey-save-detail'].children.slice(0, 4).map((el) => el.textContent), ['Entry', 'Example', 'Account', 'bob']);
assert.deepStrictEqual(elements['passkey-save-detail'].children.slice(-4).map((el) => el.textContent), ['Website', 'example.com', 'Origin', 'https://example.com']);
console.log('passkeyPopup.test.js: single-match save panel names the target entry OK');

resetPanels();
renderPasskeyCeremonyPanel({ kind: 'create', requestId: 2, rpId: 'example.com', originUrl: 'https://example.com', matches: [withPasskey] });
assert.strictEqual(elements['passkey-replace-panel'].hidden, false);
assert.deepStrictEqual(elements['passkey-replace-detail'].children.slice(0, 4).map((el) => el.textContent), ['Entry', 'Example', 'Account', 'alice']);
assert.deepStrictEqual(elements['passkey-replace-detail'].children.slice(-4).map((el) => el.textContent), ['Website', 'example.com', 'Origin', 'https://example.com']);
elements['passkey-replace-btn'].onclick();
assert.deepStrictEqual(sentMessages[0], { type: 'passkeyCreateConfirm', requestId: 2, entryId: 'e1', replace: true });
assert.strictEqual(closed, 1, 'confirming closes the popup');
assert.strictEqual(sessionStore[PENDING_PASSKEY_KEY], undefined, 'the UI leaves background-owned pending state untouched');
console.log('passkeyPopup.test.js: replace panel sends replace:true and names the target entry OK');

resetPanels();
renderPasskeyCeremonyPanel({ kind: 'get', requestId: 3, rpId: 'example.com', originUrl: 'https://example.com', matches: [withPasskey] });
assert.strictEqual(elements['passkey-use-panel'].hidden, false);
// Gate: the "Use passkey" panel used to render only the bare rpId/origin summary, never
// naming which entry it was about to sign in as.
assert.deepStrictEqual(elements['passkey-use-detail'].children.slice(0, 4).map((el) => el.textContent), ['Entry', 'Example', 'Account', 'alice']);
assert.deepStrictEqual(elements['passkey-use-detail'].children.slice(-4).map((el) => el.textContent), ['Website', 'example.com', 'Origin', 'https://example.com']);
elements['passkey-approve-btn'].onclick();
assert.deepStrictEqual(sentMessages[0], { type: 'passkeyGetApprove', requestId: 3, entryId: 'e1' });
console.log('passkeyPopup.test.js: use panel names the target entry and approves with the matched entry OK');

resetPanels();
renderPasskeyCeremonyPanel({ kind: 'get', requestId: 4, rpId: 'example.com', originUrl: 'https://example.com', matches: [withPasskey, withoutPasskey] });
assert.strictEqual(elements['passkey-picker-panel'].hidden, false);
assert.strictEqual(elements['passkey-picker-list'].children.length, 2, 'one row per match');
elements['passkey-picker-cancel-btn'].onclick();
assert.deepStrictEqual(sentMessages[0], { type: 'passkeyCancelUI', requestId: 4 });
console.log('passkeyPopup.test.js: picker renders every match and can be cancelled OK');

// A real get ceremony's matches always have hasPasskey === true (that's what made them
// match), so both rows must carry the green badge — it used to be gated on
// `pending.kind === 'create'` and never showed for the get flow at all.
const withPasskeyAlice = { id: 'e5', title: 'Alice Site', username: 'alice', hasPasskey: true };
const withPasskeyCarol = { id: 'e6', title: 'Carol Site', username: 'carol', hasPasskey: true };
resetPanels();
renderPasskeyCeremonyPanel({ kind: 'get', requestId: 4.5, rpId: 'example.com', originUrl: 'https://example.com', matches: [withPasskeyAlice, withPasskeyCarol] });
const [getRowWithBadge, getRowAlsoWithBadge] = elements['passkey-picker-list'].children;
assert.ok(
  getRowWithBadge.children.some((c) => c.className === 'passkey-badge'),
  'a get-flow picker row for a passkey-bearing entry must show the passkey badge'
);
assert.ok(
  getRowAlsoWithBadge.children.some((c) => c.className === 'passkey-badge'),
  'every get-flow picker row (all of which have a passkey) must show the badge'
);
console.log('passkeyPopup.test.js: get-flow picker rows show the passkey badge OK');

// Gate 6 regression: picking a row that already has a passkey must land on the Replace
// confirm, not silently overwrite it via a direct passkeyCreateConfirm call.
resetPanels();
renderPasskeyCeremonyPanel({ kind: 'create', requestId: 12, rpId: 'example.com', originUrl: 'https://example.com', matches: [withPasskey, withoutPasskey] });
const [passkeyRow, plainRowNoBadge] = elements['passkey-picker-list'].children;
assert.ok(
  passkeyRow.children.some((c) => c.className === 'passkey-badge'),
  'a create-flow row for an entry that already has a passkey must show the badge'
);
assert.ok(
  !plainRowNoBadge.children.some((c) => c.className === 'passkey-badge'),
  'a create-flow row with no existing passkey must not show the badge'
);
passkeyRow.listeners.click[0]();
assert.strictEqual(sentMessages.length, 0, 'selecting an entry that already has a passkey must not confirm directly');
assert.strictEqual(elements['passkey-replace-panel'].hidden, false, 'it must route to the Replace confirm panel instead');
assert.deepStrictEqual(elements['passkey-replace-detail'].children.slice(0, 4).map((el) => el.textContent), ['Entry', 'Example', 'Account', 'alice']);
elements['passkey-replace-btn'].onclick();
assert.deepStrictEqual(sentMessages[0], { type: 'passkeyCreateConfirm', requestId: 12, entryId: 'e1', replace: true });
console.log('passkeyPopup.test.js: picker routes an existing-passkey row to Replace confirm OK');

// A plain row (no existing passkey) still goes through an explicit Save confirm, naming
// that specific entry — not an immediate write either.
resetPanels();
renderPasskeyCeremonyPanel({ kind: 'create', requestId: 13, rpId: 'example.com', originUrl: 'https://example.com', matches: [withPasskey, withoutPasskey] });
const [, plainRowAgain] = elements['passkey-picker-list'].children;
plainRowAgain.listeners.click[0]();
assert.strictEqual(sentMessages.length, 0, 'selecting a plain entry must not confirm directly either');
assert.strictEqual(elements['passkey-save-panel'].hidden, false);
assert.deepStrictEqual(elements['passkey-save-detail'].children.slice(0, 4).map((el) => el.textContent), ['Entry', 'Example', 'Account', 'bob']);
elements['passkey-save-btn'].onclick();
assert.deepStrictEqual(sentMessages[0], { type: 'passkeyCreateConfirm', requestId: 13, entryId: 'e2', replace: false });
console.log('passkeyPopup.test.js: picker routes a plain row to a named Save confirm OK');

resetPanels();
sendPasskeyCancel(9);
assert.deepStrictEqual(sentMessages[0], { type: 'passkeyCancelUI', requestId: 9 });
assert.strictEqual(closed, 1);
console.log('passkeyPopup.test.js: cancel sends passkeyCancelUI and closes OK');

// --- rehydration takeover ----------------------------------------------------
sessionStore = {};
rehydratePendingPasskeyCeremony((tookOver) => {
  assert.strictEqual(tookOver, false, 'no pending ceremony means the ordinary flow proceeds');
});

sessionStore[PENDING_PASSKEY_KEY] = {
  kind: 'create', requestId: 5, rpId: 'example.com', originUrl: 'https://example.com', matches: []
};
rehydratePendingPasskeyCeremony((tookOver) => {
  assert.strictEqual(tookOver, true, 'a pending ceremony takes over the popup');
});

// A malformed record must not strand the popup on a blank panel.
sessionStore[PENDING_PASSKEY_KEY] = { kind: 'get', requestId: 6, rpId: 'example.com', matches: [] };
rehydratePendingPasskeyCeremony((tookOver) => {
  assert.strictEqual(tookOver, false, 'an unroutable record falls through to the ordinary flow');
});
console.log('passkeyPopup.test.js: rehydration takeover OK');

// --- refreshPasskeyProxy resync trigger --------------------------------------
// Regression coverage for the bug where the only resync send lived inside
// initPasskeySettingsToggle, which is skipped whenever a ceremony takes over the
// popup — meaning the proxy almost never resynced/detached during real passkey use.
//
// These drive the *production* call site directly via the exported `initPasskeyPopupUI`
// (the exact function popup.js itself invokes on load), rather than re-composing
// rehydrate/refresh/tookOver by hand in the test — a hand-rolled copy would not notice a
// regression such as `refreshPasskeyProxy()` moving back below `if (tookOver) return;`.
resetPanels();
sessionStore[PENDING_PASSKEY_KEY] = {
  kind: 'create', requestId: 30, rpId: 'example.com', originUrl: 'https://example.com', matches: []
};
initPasskeyPopupUI();
assert.strictEqual(elements['passkey-save-panel'].hidden, false, 'the ceremony must still take over the popup');
assert.strictEqual(
  sentMessages.filter((m) => m.type === 'refreshPasskeyProxy').length, 1,
  'refreshPasskeyProxy must fire even while a ceremony panel is being shown'
);
console.log('passkeyPopup.test.js: refreshPasskeyProxy fires during an active ceremony OK');

resetPanels();
sessionStore = {};
elements['popup-menu'].hidden = true;
initPasskeyPopupUI();
assert.strictEqual(
  sentMessages.filter((m) => m.type === 'refreshPasskeyProxy').length, 1,
  'refreshPasskeyProxy must still fire when there is no pending ceremony'
);
assert.strictEqual(elements['browser-chrome'].hidden, false, 'the menu button stays available when idle');
assert.strictEqual(elements['popup-menu'].hidden, true, 'the menu stays closed until the menu button is clicked');
console.log('passkeyPopup.test.js: refreshPasskeyProxy fires and the idle popup still initializes OK');

// --- ordinary credential list: passkey badge --------------------------------
// Gate: the plain #list-panel rows never surfaced credential.hasPasskey at all, even
// though the summary the extension already receives carries it.
resetPanels();
renderCredentials(
  [
    { id: 'e1', title: 'Example', username: 'alice', hasPasskey: true },
    { id: 'e2', title: 'Other', username: 'bob', hasPasskey: false }
  ],
  'https://example.com'
);
const [listRowWithPasskey, listRowWithoutPasskey] = elements['list'].children;
assert.ok(
  listRowWithPasskey.children.some((c) => c.className === 'passkey-badge'),
  'a list row for an entry with a passkey must show the badge'
);
assert.ok(
  !listRowWithoutPasskey.children.some((c) => c.className === 'passkey-badge'),
  'a list row for an entry without a passkey must not show the badge'
);
console.log('passkeyPopup.test.js: ordinary credential list shows the passkey badge OK');

console.log('passkeyPopup.test.js OK');

// Runtime status must never confuse a stored preference with working interception.
const { passkeyProxyStatusText, browserManagerDiagnosticsText } = require('../popup.js');
assert.match(passkeyProxyStatusText({ proxyState: 'attachment-error' }), /MyPass settings/);
assert.strictEqual(passkeyProxyStatusText({ proxyState: 'active' }), '');
assert.strictEqual(passkeyProxyStatusText({ proxyState: 'locked' }), '');
assert.strictEqual(browserManagerDiagnosticsText({ checked: false }), '');
const managersOnText = browserManagerDiagnosticsText({
  checked: true, passwordSavingEnabled: true, extensions: [{ name: 'iCloud Passwords' }, { name: '1Password' }]
});
assert.match(managersOnText, /Browser password saving/);
assert.match(managersOnText, /iCloud Passwords, 1Password/);
assert.match(managersOnText, /MyPass settings/);
assert.strictEqual(browserManagerDiagnosticsText({ checked: true, passwordSavingEnabled: false, extensions: [] }), '');
assert.strictEqual(choosePasskeyPanel({ kind: 'create', phase: 'checking', matches: [] }), null);
console.log('passkeyPopup.test.js: actionable runtime and manager diagnostics OK');

// Opening the popup during the native round trip must show a loading surface and
// transition in place when storage receives the eventual reply (no popup reload).
sessionStore[PENDING_PASSKEY_KEY] = { kind: 'create', requestId: 80, phase: 'checking' };
initPasskeyPopupUI();
assert.strictEqual(elements['passkey-checking-panel'].hidden, false);
assert.strictEqual(elements['passkey-save-panel'].hidden, true);
const ready = { kind: 'create', requestId: 80, rpId: 'example.com', matches: [] };
sessionStore[PENDING_PASSKEY_KEY] = ready;
pendingStorageListener({ [PENDING_PASSKEY_KEY]: { newValue: ready } }, 'local');
assert.strictEqual(elements['passkey-checking-panel'].hidden, false, 'local storage must not drive session ceremonies');
pendingStorageListener({ [PENDING_PASSKEY_KEY]: { newValue: ready } }, 'session');
assert.strictEqual(elements['passkey-checking-panel'].hidden, true);
assert.strictEqual(elements['passkey-save-panel'].hidden, false, 'late native reply renders confirmation in the already-open popup');
delete sessionStore[PENDING_PASSKEY_KEY];
pendingStorageListener({ [PENDING_PASSKEY_KEY]: { oldValue: ready } }, 'session');
assert.strictEqual(elements['passkey-save-panel'].hidden, true, 'cancellation restores ordinary UI');

// A delayed approval/cancel callback belongs to the old popup, not the next request.
const ordinarySendMessage = chrome.runtime.sendMessage;
const popupExports = require('../popup.js');
for (const act of [() => popupExports.sendPasskeyCreateConfirm(81, 'entry', false), () => popupExports.sendPasskeyGetApprove(81, 'entry'), () => sendPasskeyCancel(81)]) {
  let finishOldRequest;
  chrome.runtime.sendMessage = (_message, callback) => { finishOldRequest = callback; };
  act();
  const next = { kind: 'get', requestId: 82, phase: 'checking' };
  sessionStore[PENDING_PASSKEY_KEY] = next;
  finishOldRequest({ type: 'ok' });
  assert.strictEqual(sessionStore[PENDING_PASSKEY_KEY], next, 'old UI callback must never remove the next ceremony');
}
chrome.runtime.sendMessage = ordinarySendMessage;
console.log('passkeyPopup.test.js: live checking/ready/cancel hydration and exclusive background storage ownership OK');

// Browser completion errors remain visible even if the worker removes pending first.
chrome.runtime.sendMessage = (_message, callback) => callback({ type: 'passkeyCreateComplete', error: { name: 'OperationError' } });
popupExports.sendPasskeyCreateConfirm(90, 'entry', false);
assert.strictEqual(elements['passkey-failure-panel'].hidden, false);
assert.match(elements['passkey-failure-message'].textContent, /local passkey may already exist/);
pendingStorageListener({ [PENDING_PASSKEY_KEY]: { oldValue: { requestId: 90 } } }, 'session');
assert.strictEqual(elements['passkey-failure-panel'].hidden, false);
chrome.runtime.sendMessage = ordinarySendMessage;
sessionStore = { lastPasskeyFailure: { requestId: 90, kind: 'create' } };
rehydratePendingPasskeyCeremony((owns) => assert.strictEqual(owns, true));
assert.strictEqual(elements['passkey-failure-panel'].hidden, false, 'failure survives reopening');
sessionStore = { [PENDING_PASSKEY_KEY]: { requestId: 91, kind: 'create', phase: 'completing' } };
rehydratePendingPasskeyCeremony((owns) => assert.strictEqual(owns, true));
assert.strictEqual(elements['passkey-checking-panel'].hidden, false);
assert.strictEqual(elements['passkey-save-panel'].hidden, true, 'cannot approve twice while completing');
console.log('passkeyPopup.test.js: durable completion failure and completing state OK');

const nextCeremony = { kind: 'create', requestId: 92, rpId: 'example.com', originUrl: 'https://example.com', matches: [] };
pendingStorageListener({ [PENDING_PASSKEY_KEY]: { newValue: nextCeremony } }, 'session');
assert.strictEqual(elements['passkey-save-panel'].hidden, false);
pendingStorageListener({ lastPasskeyFailure: { newValue: { requestId: 91, kind: 'create' } } }, 'session');
assert.strictEqual(elements['passkey-save-panel'].hidden, false, 'old completion failure cannot cover the new ceremony');
assert.strictEqual(elements['passkey-failure-panel'].hidden, true);
let finishPrevious;
chrome.runtime.sendMessage = (_message, callback) => { finishPrevious = callback; };
popupExports.sendPasskeyCreateConfirm(92, 'entry', false);
pendingStorageListener({ [PENDING_PASSKEY_KEY]: { newValue: { ...nextCeremony, requestId: 93 } } }, 'session');
finishPrevious({ type: 'passkeyCreateComplete', error: { name: 'OperationError' } });
assert.strictEqual(elements['passkey-save-panel'].hidden, false, 'old callback cannot cover the new ceremony');
assert.strictEqual(elements['passkey-failure-panel'].hidden, true);
chrome.runtime.sendMessage = ordinarySendMessage;
console.log('passkeyPopup.test.js: stale failure event and callback do not cover the next approval OK');
