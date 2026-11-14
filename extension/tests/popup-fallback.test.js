const assert = require('node:assert');

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
    addEventListener() {},
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
  'pairing-error'
].forEach((id) => makeEl(id));

global.document = {
  getElementById: (id) => {
    if (!elements[id]) makeEl(id);
    return elements[id];
  },
  createElement: (tag) => makeEl(tag)
};

let sendMessageCall = 0;
let nextCredentialsResponse = { type: 'error', message: 'app_not_running' };
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
    // Counts getCredentials only: popup init also fires getPasskeyProxyStatus for the
    // passkey settings toggle, which is not a credential lookup.
    sendMessage: (msg, cb) => {
      if (!msg || msg.type !== 'getCredentials') {
        if (cb) cb({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: false });
        return;
      }
      sendMessageCall += 1;
      cb(nextCredentialsResponse);
    }
  },
  tabs: {
    query: (_opts, cb) => cb([{ url: 'https://example.com' }])
  }
};

const {
  APP_UNAVAILABLE_STATUS_TEXT,
  HOST_UNAVAILABLE_STATUS_TEXT,
  APP_UNAVAILABLE_OPEN_HELPER_TEXT,
  shouldShowAppUnavailableActions,
  LOCKED_TITLE_TEXT,
  LOCKED_HELPER_TEXT,
  EMPTY_LIST_TITLE_TEXT,
  LIST_HEADER_TEXT,
  PAIRING_PANEL_TITLE_TEXT
} = require('../popup.js');

assert.strictEqual(
  APP_UNAVAILABLE_STATUS_TEXT,
  'Cannot connect to MyPass. Make sure the app is open, then click Retry.'
);
assert.strictEqual(
  APP_UNAVAILABLE_OPEN_HELPER_TEXT,
  'Open MyPass from Applications or the Dock. Unlock your vault if needed, then tap Retry.'
);

assert.strictEqual(shouldShowAppUnavailableActions('error'), true);
assert.strictEqual(shouldShowAppUnavailableActions('locked'), false);
assert.strictEqual(shouldShowAppUnavailableActions('pairingRequired'), false);
assert.strictEqual(shouldShowAppUnavailableActions('credentialList'), false);
assert.strictEqual(shouldShowAppUnavailableActions('unknown'), false);

assert.strictEqual(LOCKED_TITLE_TEXT, 'MyPass is locked');
assert.strictEqual(LOCKED_HELPER_TEXT, 'Open MyPass and unlock the vault');
assert.strictEqual(EMPTY_LIST_TITLE_TEXT, 'No saved logins for this site');
assert.strictEqual(LIST_HEADER_TEXT, 'Select an entry to autofill:');
assert.strictEqual(PAIRING_PANEL_TITLE_TEXT, 'Connect browser extension');

const listEl = elements.list;
const retryBtn = elements['retry-btn'];

assert.strictEqual(sendMessageCall, 1, 'initial load should query credentials once');
assert.strictEqual(elements['app-unavailable-panel'].hidden, false, 'app-unavailable panel visible on error');
assert.strictEqual(elements['list-panel'].hidden, true, 'list panel hidden on error');
assert.strictEqual(elements['app-unavailable-status'].textContent, APP_UNAVAILABLE_STATUS_TEXT,
  'an unreachable app must not be described as missing or not installed');

nextCredentialsResponse = {
  type: 'credentialList', credentials: [{ id: '1', title: 'Example', username: 'alice@example.com' }]
};
retryBtn.onclick();
assert.strictEqual(sendMessageCall, 2, 'retry should re-query credentials');
assert.strictEqual(elements['list-panel'].hidden, false, 'list panel visible after credentialList');
assert.strictEqual(elements['app-unavailable-panel'].hidden, true, 'app-unavailable panel hidden after success');
assert.strictEqual(listEl.children.length, 1, 'credential rendered in list');

nextCredentialsResponse = { type: 'error', code: 'native_host_not_found', message: 'native_host_not_found' };
retryBtn.onclick();
assert.strictEqual(elements['app-unavailable-status'].textContent, HOST_UNAVAILABLE_STATUS_TEXT,
  'only an explicit missing-host code offers connector setup');
for (const response of [null, { type: 'error', message: 'unauthorized' },
  { type: 'error', message: 'timeout' }, { type: 'error', code: 'connection_unavailable' }]) {
  nextCredentialsResponse = response;
  retryBtn.onclick();
  assert.strictEqual(elements['app-unavailable-status'].textContent, APP_UNAVAILABLE_STATUS_TEXT);
}
nextCredentialsResponse = { type: 'locked' };
retryBtn.onclick();
assert.strictEqual(elements['locked-panel'].hidden, false);
assert.strictEqual(elements['app-unavailable-panel'].hidden, true);
nextCredentialsResponse = { type: 'pairingRequired' };
retryBtn.onclick();
assert.strictEqual(elements['pairing-panel'].hidden, false);
assert.strictEqual(elements['app-unavailable-panel'].hidden, true);

// Exercise the browser branch too: CommonJS imports can hide a missing helper
// on the namespace populated by separate classic script tags in popup.html.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const context = vm.createContext({ document: global.document, chrome: global.chrome,
  console, URL, setTimeout, clearTimeout, setInterval, clearInterval });
context.window = context;
nextCredentialsResponse = { type: 'error', code: 'connection_unavailable' };
const html = fs.readFileSync(path.join(__dirname, '..', 'popup.html'), 'utf8');
for (const [, relative] of html.matchAll(/<script\s+src="([^"]+)"><\/script>/g)) {
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', relative), 'utf8'), context, { filename: relative });
}
assert.strictEqual(elements['app-unavailable-status'].textContent, APP_UNAVAILABLE_STATUS_TEXT);
nextCredentialsResponse = { type: 'error', code: 'native_host_not_found' };
elements['retry-btn'].onclick();
assert.strictEqual(elements['app-unavailable-status'].textContent, HOST_UNAVAILABLE_STATUS_TEXT);

console.log('popup-fallback.test.js OK');
