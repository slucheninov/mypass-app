const assert = require('node:assert');
const path = require('node:path');

const elements = {};
function makeEl(id) {
  const el = {
    id,
    hidden: id !== '',
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


const sent = [], pending = [];
let closed = 0;
const address = { id: 'address-1', title: 'Home', username: 'Example Person', entryType: 'Address' };
const forms = [
  { id: '0', label: 'Shipping form 1', fields: 5, focused: false },
  { id: '1', label: 'Billing form 2', fields: 4, focused: false }
];
let scanResponse = { type: 'addressForms', token: 'target', url: 'https://checkout.example.test/address', forms };
let fillResponse = { type: 'addressFilled', filled: 4 };
global.document = {
  getElementById: (id) => elements[id] || makeEl(id),
  createElement: () => makeEl('created-' + Object.keys(elements).length)
};
global.window = { close() { closed += 1; } };
const storage = { get: (_key, cb) => cb({}), set: (_items, cb) => cb && cb(), remove: (_keys, cb) => cb && cb() };
global.chrome = {
  storage: { local: storage, session: storage },
  runtime: {
    lastError: null,
    sendMessage(message, cb) {
      sent.push(message);
      if (!cb) return;
      if (message.type === 'listEntries' || message.type === 'getCredentials') cb({ type: 'credentialList', credentials: [address] });
      else if (message.type === 'prepareAddressFill') cb(scanResponse);
      else if (message.type === 'fillAddress') cb(fillResponse);
      else cb({ type: 'ok' });
    }
  },
  tabs: { query: (_opts, cb) => cb([{ id: 1, url: 'https://checkout.example.test/address' }]), sendMessage() {},
    create() { throw Error('Address must never open URL or arm credential fill'); } }
};
require('../popup.js');
const selectAddresses = () => elements['category-menu'].children.find((child) => child.textContent.replace('✓ ', '') === 'Addresses').listeners.click[0]();
const click = (element, trusted = true) => element.listeners.click[0]({ isTrusted: trusted });
const row = () => elements.list.children[0];
const chooser = () => elements.list.children[0].children;
const buttons = () => chooser().filter((item) => item.type === 'button');
selectAddresses();
assert.equal(sent.at(-1).category, 'addresses');
assert.equal(row().role, 'button');
assert.equal(row().tabIndex, 0);
click(row(), false);
assert.equal(sent.at(-1).type, 'listEntries', 'synthetic address click cannot start fill');
click(row());
assert.equal(sent.at(-1).type, 'prepareAddressFill', 'choosing address only inspects form');
assert.ok(chooser()[0].textContent.includes('checkout.example.test'), 'destination is visible before confirm');
assert.equal(buttons().length, 3, 'ambiguous forms each need explicit choice plus back');
click(buttons()[0], false);
assert.equal(sent.at(-1).type, 'prepareAddressFill', 'synthetic confirm cannot request address');
click(buttons()[1]);
assert.deepEqual(sent.at(-1), { type: 'fillAddress', token: 'target', groupId: '1', id: 'address-1' });
assert.ok(chooser()[0].textContent.includes('Filled 4 fields'));
assert.ok(buttons().slice(0, 2).every((button) => button.disabled));
assert.equal(closed, 0, 'show acknowledgement and keep popup open');

selectAddresses();
scanResponse = { ...scanResponse, forms: forms.map((form, index) => ({ ...form, focused: index === 1 })) };
row().listeners.keydown[0]({ isTrusted: true, key: 'Enter', preventDefault() {} });
assert.equal(buttons().length, 2, 'unique focused group selected for confirmation');
assert.ok(buttons()[0].textContent.includes('billing'));
fillResponse = { type: 'error', code: 'address_page_changed' };
click(buttons()[0]);
assert.ok(chooser()[0].textContent.includes('page or selected tab changed'));
assert.equal(closed, 0, 'delivery failure never closes popup silently');

selectAddresses();
scanResponse = { type: 'error', code: 'address_no_form' };
click(row());
assert.ok(chooser()[0].textContent.includes('No supported address form'));
assert.equal(buttons().length, 1, 'no form does not offer a dead fill button');
assert.ok(!sent.some((message) => message.type === 'fillCredential' || message.type === 'armOpenAndFill'));

const originalGet = chrome.storage.local.get;
chrome.storage.local.get = (keys, cb) => {
  const stored = { mypassAutofillSettings: { showAddresses: false } };
  const result = {};
  if (Array.isArray(keys)) keys.forEach((name) => { if (stored[name] !== undefined) result[name] = stored[name]; });
  cb(result);
};
const sentBefore = sent.length;
selectAddresses();
click(row());
assert.equal(sent.length, sentBefore + 1, 'a hidden address suggestion does not inspect the form');
assert.equal(sent.at(-1).type, 'listEntries');
assert.ok(chooser()[0].textContent.includes('Autofill and save'));
chrome.storage.local.get = originalGet;
console.log('addressPopup.test.js: explicit destination/form confirmation, trusted actions, focus and errors OK');
