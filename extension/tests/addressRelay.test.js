const assert = require('node:assert/strict');
if (!global.crypto) global.crypto = require('node:crypto').webcrypto;
let handler, sent = [], native = [], active, documentId, mutateDuringNative, nativeAddressId, fillAck;
let showAddresses;
const extensionId = 'test-extension';
const popup = { id: extensionId, url: 'chrome-extension://' + extensionId + '/popup.html' };
const contentSender = () => ({ id: extensionId, tab: { id: 7 }, frameId: 0, documentId, url: 'https://shop.example.test/checkout' });
const forms = [{ id: '0', label: 'Shipping form 1', fields: 4, focused: false }];
global.chrome = {
  runtime: { id: extensionId, lastError: null, getURL: (path) => 'chrome-extension://' + extensionId + '/' + path,
    onMessage: { addListener: (fn) => { handler = fn; } },
    sendNativeMessage(_host, message, callback) {
      native.push(message);
      if (mutateDuringNative) mutateDuringNative();
      callback({ type: 'address', address: { id: nativeAddressId, addressLine1: '12 Example Street' } });
    }
  },
  storage: { local: { get: (_keys, cb) => cb({ pairingSecret: 'paired', mypassAutofillSettings: { showAddresses } }) } },
  tabs: {
    query: (_options, cb) => cb(active ? [active] : []),
    sendMessage(tabId, message, options, callback) {
      sent.push({ tabId, message, options });
      if (message.type === 'queryAddressForm') {
        handler({ type: 'addressFormReport', challenge: message.challenge, forms }, contentSender(), () => {});
        callback({ type: 'ok' });
      } else if (options.documentId !== documentId) {
        chrome.runtime.lastError = { message: 'No document' };
        callback(); chrome.runtime.lastError = null;
      } else callback(fillAck);
    }
  }
};
require('../background.js');
function reset() {
  sent = []; native = []; active = { id: 7, url: 'https://shop.example.test/checkout' };
  documentId = 'original-document'; mutateDuringNative = null; nativeAddressId = 'abc';
  showAddresses = true;
  fillAck = { type: 'addressFilled', filled: 3 };
}
function request(message, sender = popup) {
  return new Promise((resolve) => {
    const handled = handler(message, sender, resolve);
    if (!handled) resolve(undefined);
  });
}
async function prepare() {
  const result = await request({ type: 'prepareAddressFill' });
  assert.equal(result.type, 'addressForms');
  return result.token;
}
async function run() {
  reset();
  const token = await prepare();
  const result = await request({ type: 'fillAddress', token, id: 'abc', groupId: '0', url: 'https://attacker.test/' });
  assert.deepEqual(result, { type: 'addressFilled', filled: 3 });
  assert.deepEqual(native, [{ type: 'fillAddress', id: 'abc', url: active.url, pairing: 'paired' }]);
  assert.deepEqual(sent[1].options, { frameId: 0, documentId: 'original-document' });
  assert.equal(sent[1].message.url, active.url, 'caller URL is ignored');
  assert.equal(result.address, undefined, 'popup never receives address plaintext');
  assert.equal((await request({ type: 'fillAddress', token, id: 'abc', groupId: '0' })).code, 'address_page_changed');
  assert.equal(native.length, 1, 'confirmation cannot replay');

  for (const sender of [contentSender(), { ...popup, url: 'https://shop.example.test/' }, {},
    { ...popup, id: 'other-extension' }]) {
    reset();
    assert.equal((await request({ type: 'prepareAddressFill' }, sender)).code, 'address_unauthorized');
    assert.equal((await request({ type: 'fillAddress', id: 'abc' }, sender)).code, 'address_unauthorized');
    assert.equal(native.length, 0); assert.equal(sent.length, 0);
  }
  for (const type of ['listEntries', 'searchEntries', 'getFavorites']) {
    assert.equal(await request({ type, category: 'addresses', query: 'Home' }, contentSender()), undefined);
    assert.equal(native.length, 0, 'vault browsing stays in the popup');
  }
  reset();
  showAddresses = false;
  assert.equal((await request({ type: 'prepareAddressFill' })).code, 'address_disabled');
  assert.equal(sent.length, 0);
  reset();
  let disabledToken = await prepare();
  showAddresses = false;
  assert.equal((await request({ type: 'fillAddress', token: disabledToken, id: 'abc', groupId: '0' })).code, 'address_disabled');
  assert.equal(native.length, 0);
  reset();
  disabledToken = await prepare();
  mutateDuringNative = () => { showAddresses = false; };
  assert.equal((await request({ type: 'fillAddress', token: disabledToken, id: 'abc', groupId: '0' })).code, 'address_disabled');
  assert.equal(sent.length, 1, 'turning off suggestions in flight stops delivery');

  reset();
  let prepared = await prepare(); active = { id: 8, url: active.url };
  assert.equal((await request({ type: 'fillAddress', token: prepared, id: 'abc', groupId: '0' })).code, 'address_page_changed');
  assert.equal(native.length, 0, 'tab switch before retrieval aborts');

  for (const mutate of [() => { active.url = 'https://evil.example.test/'; },
    () => { active.url = 'http://shop.example.test/checkout'; },
    () => { active.url = 'https://shop.example.test/other'; },
    () => { active.id = 8; }]) {
    reset(); prepared = await prepare(); mutateDuringNative = mutate;
    assert.equal((await request({ type: 'fillAddress', token: prepared, id: 'abc', groupId: '0' })).code, 'address_page_changed');
    assert.equal(sent.length, 1, 'navigation/tab switch during native response never sends PII');
  }
  reset(); prepared = await prepare(); mutateDuringNative = () => { documentId = 'replacement-document'; };
  assert.equal((await request({ type: 'fillAddress', token: prepared, id: 'abc', groupId: '0' })).code, 'address_page_changed');
  assert.equal(sent[1].options.documentId, 'original-document', 'same URL reload remains pinned to old document');

  reset(); prepared = await prepare(); nativeAddressId = 'other-id';
  assert.equal((await request({ type: 'fillAddress', token: prepared, id: 'abc', groupId: '0' })).code, 'address_unavailable');
  assert.equal(sent.length, 1, 'wrong native ID never leaves background');

  reset(); prepared = await prepare(); fillAck = { type: 'error', code: 'address_form_changed' };
  assert.equal((await request({ type: 'fillAddress', token: prepared, id: 'abc', groupId: '0' })).code, 'address_form_changed');

  reset(); prepared = await prepare();
  assert.equal((await request({ type: 'fillAddress', token: prepared, id: 'abc', groupId: '9' })).code, 'address_page_changed');
  assert.equal(native.length, 0);
  for (const url of ['http://shop.example.test/checkout', 'chrome://settings', 'https://a@shop.example.test/checkout']) {
    reset(); active = { id: 7, url };
    assert.equal((await request({ type: 'prepareAddressFill' })).code, 'address_no_form');
    assert.equal(sent.length, 0);
    assert.equal(native.length, 0);
  }
  console.log('addressRelay.test.js: authorization, exact document routing, replay and navigation races OK');
}
run().catch((error) => { console.error(error); process.exitCode = 1; });
