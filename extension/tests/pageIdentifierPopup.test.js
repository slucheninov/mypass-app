const assert = require('node:assert');
const path = require('node:path');

const elements = {};
function makeEl(id) {
  const el = {
    id,
    hidden: false,
    textContent: '',
    value: '',
    disabled: false,
    className: '',
    style: {},
    children: [],
    listeners: {},
    appendChild(child) { this.children.push(child); },
    addEventListener(name, fn) { (this.listeners[name] = this.listeners[name] || []).push(fn); },
    setAttribute(name, value) { this[name] = value; }
  };
  let innerHTML = '';
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

const sentMessages = [];
const createdWindows = [];
let pageIdentifierResponse = { value: '+38 066 183 74 99', kind: 'phone' };
let siteCredentials = [{ id: 'yasno', title: 'Yasno', username: 'ada@example.com', hasTotp: false, hasPasskey: false, identifiers: [] }];
let addResult = { type: 'ok' };

global.document = {
  getElementById: (id) => elements[id] || makeEl(id),
  createElement: () => makeEl('created-' + Object.keys(elements).length)
};
global.window = {
  close() {},
  screen: { availWidth: 1440, availHeight: 900, availLeft: 0, availTop: 0 }
};
global.chrome = {
  storage: {
    local: { get: (_keys, cb) => cb({}), set: (_items, cb) => cb && cb(), remove: (_keys, cb) => cb && cb() },
    session: { get: (_keys, cb) => cb({}), set: (_items, cb) => cb && cb(), remove: (_keys, cb) => cb && cb() }
  },
  runtime: {
    lastError: null,
    getURL(fragment) { return 'chrome-extension://extension-id/' + fragment; },
    sendMessage(message, cb) {
      sentMessages.push(message);
      if (!cb) return;
      if (message.type === 'getCredentials') {
        cb({ type: 'credentialList', credentials: siteCredentials });
        return;
      }
      if (message.type === 'addEntryIdentifier') { cb(addResult); return; }
      cb({ type: 'ok' });
    }
  },
  tabs: {
    query(_opts, cb) { cb([{ id: 3, url: 'https://login.yasno.ua/oauth' }]); },
    sendMessage(_tabId, message, cb) {
      if (message.type === 'queryPageIdentifier') cb(pageIdentifierResponse);
      else if (cb) cb({});
    }
  },
  windows: {
    create(props, cb) {
      createdWindows.push(props);
      if (cb) cb({ id: 1 });
    }
  }
};

const resolved = require.resolve(path.join(__dirname, '..', 'popup.js'));
delete require.cache[resolved];
require(resolved);

const offer = elements['identifier-offer'];
assert.equal(offer.hidden, false, 'a filled phone missing from the saved item is offered');
assert.match(elements['identifier-offer-text'].textContent, /\+38 066 183 74 99/);
assert.match(elements['identifier-offer-text'].textContent, /existing item/);
assert.match(elements['identifier-offer-text'].textContent, /new one/);
const actions = elements['identifier-offer-actions'].children;
assert.equal(actions.length, 2);
assert.equal(actions[0].textContent, 'Add to Yasno');
assert.equal(actions[1].textContent, 'New item');
assert.match(elements['list'].children[0].className, /credential-row-unsaved/);

actions[1].listeners.click[0]({ isTrusted: true });
const created = new URL(createdWindows[0].url);
assert.equal(created.searchParams.get('type'), 'Login');
assert.equal(created.searchParams.get('username'), '+38 066 183 74 99');
assert.equal(created.searchParams.get('url'), 'https://login.yasno.ua/oauth');

actions[0].listeners.click[0]({ isTrusted: true });
const added = sentMessages.filter((message) => message.type === 'addEntryIdentifier').pop();
assert.equal(added.id, 'yasno');
assert.equal(added.value, '+38 066 183 74 99');
assert.equal(added.fieldKey, 'Phone');
assert.equal(added.url, 'https://login.yasno.ua/oauth');
assert.equal(offer.hidden, true, 'adding the phone clears the offer');
assert.doesNotMatch(elements['list'].children[0].className, /unsaved/);

console.log('pageIdentifierPopup.test.js: highlight, add, and new item OK');
