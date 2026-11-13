const assert = require('node:assert');

// --- copied from popup-fallback.test.js: exact global.document / global.chrome stub
// setup so `require('../popup.js')` runs its top-level DOM-wiring code without throwing. ---

const elements = {};
function makeEl(id) {
  const el = {
    id,
    hidden: false,
    textContent: '',
    innerHTML: '',
    value: '',
    onclick: null,
    className: '',
    style: {},
    children: [],
    appendChild(child) {
      this.children.push(child);
    },
    addEventListener(type, handler) {
      this._listeners = this._listeners || {};
      this._listeners[type] = handler;
    },
    setAttribute(name, value) {
      this[name] = value;
    }
  };
  elements[id] = el;
  return el;
}

[
  'locked-panel',
  'locked-title',
  'locked-helper',
  'locked-retry-btn',
  'app-unavailable-panel',
  'app-unavailable-status',
  'app-unavailable-actions',
  'retry-btn',
  'open-mypass-btn',
  'open-mypass-helper',
  'empty-panel',
  'empty-title',
  'list-panel',
  'list-header',
  'list',
  'pairing-panel',
  'pairing-title',
  'connect-btn',
  'code-entry',
  'code-input',
  'submit-code-btn',
  'pairing-error',
  'add-totp-section',
  'add-totp-btn',
  'totp-paste-input',
  'add-totp-error'
].forEach((id) => makeEl(id));

global.document = {
  getElementById: (id) => {
    if (!elements[id]) makeEl(id);
    return elements[id];
  },
  createElement: (tag) => makeEl(tag)
};

let sendMessageCall = 0;
global.chrome = {
  storage: {
    local: {
      get: (_keys, cb) => cb({}),
      set: (_items, cb) => cb && cb(),
      remove: (_keys, cb) => cb && cb()
    },
    session: null
  },
  runtime: {
    // Popup init also fires getPasskeyProxyStatus (answered generically below) and the
    // fire-and-forget refreshPasskeyProxy (no callback at all) — only getCredentials
    // calls are counted here, matching popup-fallback.test.js's stub shape.
    sendMessage: (msg, cb) => {
      if (!msg || msg.type !== 'getCredentials') {
        if (cb) cb({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: false, enabled: false });
        return;
      }
      sendMessageCall += 1;
      if (sendMessageCall === 1) {
        cb({ type: 'error', message: 'native_host_not_found' });
        return;
      }
      if (sendMessageCall === 2) {
        cb({
          type: 'credentialList',
          credentials: [{ id: '1', title: 'Example', username: 'alice@example.com' }]
        });
      }
    }
  },
  tabs: {
    query: (_opts, cb) => cb([{ id: 1, url: 'https://example.com' }])
    // sendMessage is intentionally absent at module-load time: requestPageTotpCandidate
    // guards on `typeof chrome.tabs.sendMessage !== 'function'` so popup init doesn't
    // crash on it, and every test below installs its own stub before calling it directly.
  }
};

const {
  validateTotpPasteInput,
  isBase32PasteSecret,
  renderTotpPicker,
  routeTotpCandidate,
  requestPendingPrompt,
  handleAddTotpClick,
  requestPageTotpCandidate
} = require('../popup.js');

// --- additive: the Add-2FA section (popup.html's `#add-totp-section`) must start
// hidden — it only appears once there is an actual 2FA candidate in context. ---
assert.strictEqual(elements['add-totp-section'].hidden, true, 'Add 2FA section starts hidden');
console.log('totpPicker.test.js: add-totp-section hidden by default OK');

assert.deepStrictEqual(
  validateTotpPasteInput('otpauth://totp/Example:alice?secret=JBSWY3DP'),
  { valid: true, value: 'otpauth://totp/Example:alice?secret=JBSWY3DP' }
);
assert.deepStrictEqual(validateTotpPasteInput('  JBSW Y3DP EHPK 3PXP  '), { valid: true, value: 'JBSWY3DPEHPK3PXP' });
assert.strictEqual(validateTotpPasteInput('not valid').valid, false);
assert.strictEqual(validateTotpPasteInput('').valid, false);
assert.strictEqual(isBase32PasteSecret('JBSWY3DPEHPK3PXP'), true);
assert.strictEqual(isBase32PasteSecret('short'), false);
console.log('totpPicker.test.js: paste validation OK');

(() => {
  let sentMessage = null;
  global.chrome.runtime.sendMessage = (msg, cb) => { sentMessage = msg; if (cb) cb({ type: 'saveProposalAccepted' }); };

  routeTotpCandidate('https://example.com', 'JBSWY3DP', []);
  assert.strictEqual(sentMessage.type, 'proposeSaveTotp');
  assert.strictEqual(sentMessage.entryId, undefined, 'zero matches sends no entryId');

  routeTotpCandidate('https://example.com', 'JBSWY3DP', [{ id: 'entry-1', title: 'Example', hasTotp: false }]);
  assert.strictEqual(sentMessage.entryId, 'entry-1', 'single match includes entryId');

  console.log('totpPicker.test.js: routeTotpCandidate routing OK');
})();

// --- additive: renderTotpPicker builds one row per credential and only tags
// hasTotp credentials with the 2FA badge. ---
(() => {
  elements.list.children = []; // this fake DOM's innerHTML='' does not clear .children

  renderTotpPicker(
    [
      { id: 'a', title: 'Alpha', hasTotp: true },
      { id: 'b', title: 'Bravo', hasTotp: false }
    ],
    'https://example.com',
    'JBSWY3DP'
  );

  assert.strictEqual(elements.list.children.length, 2, 'renders one row per credential');
  const alphaBadges = elements.list.children[0].children.filter((c) => c.className === 'totp-badge');
  const bravoBadges = elements.list.children[1].children.filter((c) => c.className === 'totp-badge');
  assert.strictEqual(alphaBadges.length, 1, 'hasTotp credential gets a 2FA badge');
  assert.strictEqual(bravoBadges.length, 0, 'credential without hasTotp has no badge');

  console.log('totpPicker.test.js: renderTotpPicker badge rendering OK');
})();

// --- additive: routeTotpCandidate falls through to renderTotpPicker (not a direct
// sendMessage) once there is more than one match. ---
(() => {
  let sentMessage = null;
  global.chrome.runtime.sendMessage = (msg, cb) => { sentMessage = msg; if (cb) cb({}); };
  elements.list.children = [];

  routeTotpCandidate('https://example.com', 'JBSWY3DP', [
    { id: 'entry-1', title: 'Example', hasTotp: false },
    { id: 'entry-2', title: 'Other', hasTotp: false }
  ]);

  assert.strictEqual(sentMessage, null, 'multi-match does not eagerly propose a save');
  assert.strictEqual(elements.list.children.length, 2, 'multi-match renders a picker row per credential');

  console.log('totpPicker.test.js: routeTotpCandidate multi-match delegates to renderTotpPicker OK');
})();

// --- additive: a picker row click reports the actual proposeSaveTotp outcome instead of
// always claiming success (Finding 3). ---
(() => {
  global.chrome.runtime.sendMessage = (msg, cb) => { if (cb) cb({ type: 'locked' }); };
  elements.list.children = [];
  elements['add-totp-error'].hidden = true;
  elements['add-totp-error'].textContent = '';

  renderTotpPicker([{ id: 'a', title: 'Alpha', hasTotp: false }], 'https://example.com', 'JBSWY3DP');
  elements.list.children[0]._listeners.click();

  assert.strictEqual(elements['add-totp-error'].hidden, false, 'error shown on locked outcome');
  assert.strictEqual(elements['add-totp-error'].textContent, 'Unlock MyPass first');

  console.log('totpPicker.test.js: picker row click reports locked outcome OK');
})();

(() => {
  global.chrome.runtime.sendMessage = (msg, cb) => { if (cb) cb({ type: 'saveProposalAccepted' }); };
  elements.list.children = [];
  elements['add-totp-error'].hidden = true;
  elements['add-totp-error'].textContent = '';

  renderTotpPicker([{ id: 'a', title: 'Alpha', hasTotp: false }], 'https://example.com', 'JBSWY3DP');
  elements.list.children[0]._listeners.click();

  assert.strictEqual(elements['add-totp-error'].hidden, false);
  assert.strictEqual(elements['add-totp-error'].textContent, 'Opened in MyPass');

  console.log('totpPicker.test.js: picker row click reports success outcome OK');
})();

// --- the 2FA save prompt no longer travels through storage the popup reads: the old
// `pendingTotpCandidate` key (written by the content script) is gone, and a pending prompt is
// asked of the background by tab id (requestPendingPrompt). ---
(() => {
  const storageReads = [];
  const realLocalGet = global.chrome.storage.local.get;
  global.chrome.storage.local.get = (keys, cb) => { storageReads.push(keys); realLocalGet(keys, cb); };
  const asked = [];
  global.window = { close() { global.__closed = (global.__closed || 0) + 1; } };
  const promptView = {
    id: 'p-1', kind: 'saveTotp', origin: 'https://example.com', label: 'Example:alice',
    secretDetected: true, expiresAt: Date.now() + 60000, state: 'ready',
    accounts: [{ id: 'x', title: 'Xray', username: 'u', hasTotp: false }, { id: 'y', title: 'Yankee', username: 'v', hasTotp: true }]
  };
  let reply = { type: 'prompt', prompt: promptView };
  global.chrome.runtime.sendMessage = (msg, cb) => {
    asked.push(msg);
    if (cb) cb(msg.type === 'getPrompt' ? reply : { type: 'ok' });
  };

  requestPendingPrompt(false);
  assert.deepStrictEqual(asked[0], { type: 'getPrompt', tabId: 1 }, 'asks the background for the popup tab only');
  assert.strictEqual(elements['prompt-panel'].hidden, false, 'a pending prompt takes over the popup');
  assert.ok(elements['prompt-card-root'].children.length > 0, 'the card is rendered into the prompt panel');
  assert.ok(storageReads.every((keys) => !JSON.stringify(keys).includes('pendingTotpCandidate')),
    'the popup no longer reads pendingTotpCandidate');

  reply = { type: 'noPrompt' };
  requestPendingPrompt(false);
  assert.strictEqual(elements['prompt-panel'].hidden, true, 'prompt released when nothing is pending');

  global.__closed = 0;
  requestPendingPrompt(true);
  assert.strictEqual(global.__closed, 1, 'after an answer, an empty queue closes the popup');
  global.chrome.storage.local.get = realLocalGet;

  console.log('totpPicker.test.js: requestPendingPrompt takes over, releases and closes OK');
})();

// --- additive: handleAddTotpClick must never fire a blind proposeSaveTotp when the
// vault is locked or unpaired — same trust model as the password-save flow (Global
// Constraints: pairing secret required, vault must be unlocked). ---
(() => {
  elements['totp-paste-input'].value = 'JBSWY3DPEHPK3PXP';
  elements['add-totp-error'].hidden = true;
  elements['add-totp-error'].textContent = '';
  let sentMessage = null;
  global.chrome.runtime.sendMessage = (msg, cb) => {
    if (msg.type === 'proposeSaveTotp') sentMessage = msg;
    if (msg.type === 'getCredentials') cb({ type: 'locked' });
  };

  handleAddTotpClick('https://example.com');

  assert.strictEqual(sentMessage, null, 'locked vault must not trigger proposeSaveTotp');
  assert.strictEqual(elements['add-totp-error'].hidden, false, 'error shown when locked');
  assert.strictEqual(elements['add-totp-error'].textContent, 'Unlock MyPass first');

  console.log('totpPicker.test.js: handleAddTotpClick locked gate OK');
})();

(() => {
  elements['totp-paste-input'].value = 'JBSWY3DPEHPK3PXP';
  elements['add-totp-error'].hidden = true;
  elements['add-totp-error'].textContent = '';
  let sentMessage = null;
  global.chrome.runtime.sendMessage = (msg, cb) => {
    if (msg.type === 'proposeSaveTotp') sentMessage = msg;
    if (msg.type === 'getCredentials') cb({ type: 'pairingRequired' });
  };

  handleAddTotpClick('https://example.com');

  assert.strictEqual(sentMessage, null, 'unpaired browser must not trigger proposeSaveTotp');
  assert.strictEqual(elements['add-totp-error'].hidden, false, 'error shown when pairing required');
  assert.strictEqual(elements['add-totp-error'].textContent, 'Connect to MyPass first');

  console.log('totpPicker.test.js: handleAddTotpClick pairingRequired gate OK');
})();

// --- additive: any other non-credentialList response (error / undefined / unreachable
// native host) must also not fire a blind proposeSaveTotp. ---
(() => {
  elements['totp-paste-input'].value = 'JBSWY3DPEHPK3PXP';
  elements['add-totp-error'].hidden = true;
  elements['add-totp-error'].textContent = '';
  let sentMessage = null;
  global.chrome.runtime.sendMessage = (msg, cb) => {
    if (msg.type === 'proposeSaveTotp') sentMessage = msg;
    if (msg.type === 'getCredentials') cb({ type: 'error', message: 'native_host_not_found' });
  };

  handleAddTotpClick('https://example.com');

  assert.strictEqual(sentMessage, null, 'unreachable MyPass must not trigger proposeSaveTotp');
  assert.strictEqual(elements['add-totp-error'].hidden, false, 'error shown when unreachable');
  assert.strictEqual(elements['add-totp-error'].textContent, "Couldn't reach MyPass");

  console.log('totpPicker.test.js: handleAddTotpClick unreachable-host gate OK');
})();

// --- additive: a real credentialList response still proceeds to routeTotpCandidate. ---
(() => {
  elements['totp-paste-input'].value = 'JBSWY3DPEHPK3PXP';
  elements['add-totp-error'].hidden = true;
  elements['add-totp-error'].textContent = '';
  let sentMessage = null;
  global.chrome.runtime.sendMessage = (msg, cb) => {
    if (msg.type === 'proposeSaveTotp') sentMessage = msg;
    if (msg.type === 'getCredentials') {
      cb({ type: 'credentialList', credentials: [{ id: 'entry-1', title: 'Example', hasTotp: false }] });
    }
  };

  handleAddTotpClick('https://example.com');

  assert.strictEqual(sentMessage && sentMessage.type, 'proposeSaveTotp', 'unlocked+paired single match still proposes a save');
  assert.strictEqual(sentMessage.entryId, 'entry-1');

  console.log('totpPicker.test.js: handleAddTotpClick credentialList proceeds OK');
})();

// --- additive: requestPageTotpCandidate gates the Add-2FA section's visibility on
// whether the active tab's content script actually reports a 2FA candidate. ---

// A page candidate response unhides the section and prefills the paste input.
(() => {
  elements['add-totp-section'].hidden = true;
  elements['totp-paste-input'].value = '';
  global.chrome.runtime.lastError = undefined;
  global.chrome.tabs.sendMessage = (tabId, msg, cb) => {
    assert.strictEqual(tabId, 7);
    assert.strictEqual(msg.type, 'queryTotpCandidate');
    cb({ candidate: 'otpauth://totp/Example:alice?secret=JBSWY3DP', priority: 1 });
  };

  requestPageTotpCandidate(7);

  assert.strictEqual(elements['add-totp-section'].hidden, false, 'candidate found unhides the Add 2FA section');
  assert.strictEqual(
    elements['totp-paste-input'].value,
    'otpauth://totp/Example:alice?secret=JBSWY3DP',
    'candidate prefills the paste input'
  );

  console.log('totpPicker.test.js: requestPageTotpCandidate candidate found OK');
})();

// chrome.runtime.lastError (no content script — e.g. a chrome:// page) must not un-hide
// the section, and must not throw/log despite the callback firing with no response.
(() => {
  elements['add-totp-section'].hidden = true;
  elements['totp-paste-input'].value = '';
  global.chrome.tabs.sendMessage = (_tabId, _msg, cb) => {
    global.chrome.runtime.lastError = { message: 'Could not establish connection. Receiving end does not exist.' };
    cb(undefined);
  };

  requestPageTotpCandidate(7);

  assert.strictEqual(elements['add-totp-section'].hidden, true, 'lastError keeps the section hidden');
  global.chrome.runtime.lastError = undefined;

  console.log('totpPicker.test.js: requestPageTotpCandidate lastError stays hidden OK');
})();

// A null/no-candidate response (content script present, nothing detected on the page)
// also leaves the section hidden.
(() => {
  elements['add-totp-section'].hidden = true;
  global.chrome.tabs.sendMessage = (_tabId, _msg, cb) => { cb(null); };

  requestPageTotpCandidate(7);

  assert.strictEqual(elements['add-totp-section'].hidden, true, 'no candidate keeps the section hidden');

  console.log('totpPicker.test.js: requestPageTotpCandidate no-candidate stays hidden OK');
})();
