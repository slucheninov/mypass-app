// The person relay: the identity list puts people first (with their linked addresses, the first
// one the default) and lists only the addresses no person links; a person fill asks the native
// host for `fillPerson` (with the picked address when there is one) and hands back the person
// fields and the address fields only. The popup's person fill goes through the same one-shot
// form token as an address fill. Also the content side: the panel rows and the address picker.
const assert = require('node:assert/strict');
if (!global.crypto) global.crypto = require('node:crypto').webcrypto;
// Loaded before `chrome` exists, so content.js only defines its functions.
const { chooseIdentityPanel, personAddressChoices } = require('../content.js');
const { identityListAsCredentials, addressFormName } = require('../popup.js');

let handler, native, sent, showAddresses, nativeReply, active, documentId, applyAck;
const extensionId = 'test-extension';
const page = 'https://shop.example.test/register';
const content = (extra = {}) => ({ id: extensionId, tab: { id: 7 }, frameId: 0, documentId: 'doc-1', url: page, ...extra });
const popup = { id: extensionId, url: 'chrome-extension://' + extensionId + '/popup.html' };
const forms = [
  { id: '0', purpose: 'person', number: 1, label: 'person-1', fields: 6, focused: false },
  { id: '1', purpose: 'shipping', number: 1, label: 'shipping-1', fields: 4, focused: false },
];
global.chrome = {
  runtime: { id: extensionId, lastError: null, getURL: (path) => 'chrome-extension://' + extensionId + '/' + path,
    onMessage: { addListener: (fn) => { handler = fn; } },
    sendNativeMessage(_host, message, callback) {
      native.push(message);
      callback(nativeReply(message));
    },
  },
  storage: { local: { get: (_keys, cb) => cb({ pairingSecret: 'paired', mypassAutofillSettings: { showAddresses } }) } },
  tabs: {
    query: (_options, cb) => cb(active ? [active] : []),
    sendMessage(tabId, message, options, callback) {
      sent.push({ tabId, message, options });
      if (message.type === 'queryAddressForm') {
        handler({ type: 'addressFormReport', challenge: message.challenge, forms },
          { id: extensionId, tab: { id: 7 }, frameId: 0, documentId, url: page }, () => {});
        callback({ type: 'ok' });
      } else callback(applyAck);
    },
  },
};
require('../background.js');

const ADA = { id: 'P-1', label: 'Me', fullName: 'Ada Lovelace', givenName: 'Ada', middleName: 'King',
  familyName: 'Lovelace', email: 'ada@example.test', birthDate: '1815-12-10', notes: 'private', password: 'nope' };
const HOME = { id: 'A-1', label: 'Home', city: 'London', addressLine1: '1 Main St', notes: 'private' };

function reset() {
  native = [];
  sent = [];
  showAddresses = true;
  active = { id: 7, url: page };
  documentId = 'doc-1';
  applyAck = { type: 'addressFilled', filled: 5 };
  nativeReply = (message) => {
    if (message.type === 'listEntries' && message.category === 'people') {
      return { type: 'credentialList', credentials: [
        { id: 'P-1', title: 'Ada', username: 'ada@example.test', addressIds: ['a-2', 'gone', 'A-1'], password: 'never' },
        { id: 'P-2', title: 'Bob', username: '' },
      ] };
    }
    if (message.type === 'listEntries') {
      return { type: 'credentialList', credentials: [
        { id: 'A-1', title: 'Home', username: 'Ada Lovelace' },
        { id: 'A-2', title: 'Work', username: '' },
        { id: 'A-3', title: 'Cottage', username: '' },
      ] };
    }
    if (message.type === 'fillPerson') {
      return { type: 'person', person: ADA, address: message.addressId ? { ...HOME, id: message.addressId } : HOME };
    }
    return { type: 'error', code: 'unexpected' };
  };
}
function request(message, sender) {
  return new Promise((resolve) => {
    if (!handler(message, sender, resolve)) resolve(undefined);
  });
}

async function run() {
  // People first, their links mapped to the listed addresses (case-insensitively, unknown ones
  // dropped, order kept), then only the addresses no person links. `addresses` has them all.
  reset();
  const list = await request({ type: 'getIdentitySuggestions', url: page }, content());
  assert.equal(list.type, 'identityList');
  assert.deepEqual(list.entries, [
    { id: 'P-1', kind: 'person', title: 'Ada', username: 'ada@example.test', addressIds: ['A-2', 'A-1'] },
    { id: 'P-2', kind: 'person', title: 'Bob', username: '', addressIds: [] },
    { id: 'A-3', kind: 'address', title: 'Cottage', username: '' },
  ]);
  assert.deepEqual(list.addresses.map((row) => row.id), ['A-1', 'A-2', 'A-3']);
  assert.ok(!JSON.stringify(list).includes('never'), 'the list never carries anything but ids and names');

  // An app without the people category answers an error: the addresses are still listed.
  reset();
  const older = nativeReply;
  nativeReply = (message) => message.category === 'people' ? { type: 'error', code: 'unknown_category' } : older(message);
  const plain = await request({ type: 'getIdentitySuggestions', url: page }, content());
  assert.deepEqual(plain.entries.map((row) => row.id), ['A-1', 'A-2', 'A-3']);

  // A person fill: native fillPerson, only the person and address fields come back.
  reset();
  const fill = await request({ type: 'fillIdentity', kind: 'person', id: 'P-1', url: page }, content());
  assert.deepEqual(native, [{ type: 'fillPerson', id: 'P-1', url: page, pairing: 'paired' }]);
  assert.equal(fill.type, 'identity');
  assert.deepEqual(fill.person, { fullName: 'Ada Lovelace', givenName: 'Ada', middleName: 'King',
    familyName: 'Lovelace', email: 'ada@example.test', birthDate: '1815-12-10' });
  assert.equal(fill.address.city, 'London');
  assert.equal(fill.address.notes, undefined);
  assert.equal(fill.address.id, undefined);

  // The picked address travels; an answer with another address is refused.
  reset();
  const picked = await request({ type: 'fillIdentity', kind: 'person', id: 'P-1', addressId: 'A-3', url: page }, content());
  assert.equal(native[0].addressId, 'A-3');
  assert.equal(picked.type, 'identity');
  reset();
  nativeReply = () => ({ type: 'person', person: ADA, address: HOME });
  assert.equal((await request({ type: 'fillIdentity', kind: 'person', id: 'P-1', addressId: 'A-3', url: page },
    content())).code, 'address_unavailable');

  // No live linked address: the person alone. A bad birth date is dropped, another person refused.
  reset();
  nativeReply = () => ({ type: 'person', person: { ...ADA, birthDate: '10.12.1815' } });
  const alone = await request({ type: 'fillIdentity', kind: 'person', id: 'p-1', url: page }, content());
  assert.equal(alone.address, null);
  assert.equal(alone.person.birthDate, '');
  reset();
  assert.equal((await request({ type: 'fillIdentity', kind: 'person', id: 'P-9', url: page }, content())).code,
    'address_unavailable');

  // Bad ids never reach the host; the address switch and the sender gate apply as for addresses.
  reset();
  for (const message of [
    { type: 'fillIdentity', kind: 'person', id: 'P-1', addressId: 'x'.repeat(65), url: page },
    { type: 'fillIdentity', kind: 'person', id: '', url: page },
  ]) assert.equal((await request(message, content())).code, 'address_unavailable');
  assert.equal(native.length, 0);
  showAddresses = false;
  assert.equal((await request({ type: 'fillIdentity', kind: 'person', id: 'P-1', url: page }, content())).code,
    'address_disabled');
  showAddresses = true;
  assert.equal((await request({ type: 'fillIdentity', kind: 'person', id: 'P-1', url: page },
    content({ frameId: 2 }))).code, 'address_unauthorized');
  assert.equal(native.length, 0);
  reset();
  nativeReply = () => ({ type: 'locked' });
  assert.deepEqual(await request({ type: 'fillIdentity', kind: 'person', id: 'P-1', url: page }, content()),
    { type: 'locked' });

  // The popup: prepare with the person flag, then one fillPerson for the reported form.
  reset();
  const scan = await request({ type: 'prepareAddressFill', person: true }, popup);
  assert.equal(scan.type, 'addressForms');
  assert.equal(sent[0].message.person, true, 'the page is asked for person forms too');
  const done = await request({ type: 'fillPerson', token: scan.token, groupId: '0', id: 'P-1', addressId: 'A-2' }, popup);
  assert.deepEqual(done, { type: 'addressFilled', filled: 5 });
  assert.deepEqual(native, [{ type: 'fillPerson', id: 'P-1', url: page, addressId: 'A-2', pairing: 'paired' }]);
  const apply = sent.find((item) => item.message.type === 'applyAddress').message;
  assert.equal(apply.groupId, '0');
  assert.equal(apply.person.givenName, 'Ada');
  assert.equal(apply.address.id, undefined);
  assert.equal(done.person, undefined, 'the popup never receives person values');
  assert.equal((await request({ type: 'fillPerson', token: scan.token, groupId: '0', id: 'P-1' }, popup)).code,
    'address_page_changed', 'a token fills once');

  // fillPerson and getIdentityList are popup-only.
  reset();
  for (const message of [{ type: 'fillPerson', token: 't', groupId: '0', id: 'P-1' }, { type: 'getIdentityList' }]) {
    assert.equal((await request(message, content())).code, 'address_unauthorized');
  }
  assert.equal(native.length, 0);
  const popupList = await request({ type: 'getIdentityList' }, popup);
  assert.equal(popupList.entries[0].kind, 'person');
  reset();
  nativeReply = () => ({ type: 'error', code: 'app_not_running' });
  assert.deepEqual(await request({ type: 'getIdentityList' }, popup), { type: 'error', code: 'app_not_running' },
    'the popup sees a failed listing as it came');

  // The popup renders the list as credential rows; person rows are Person entries.
  const rows = identityListAsCredentials(popupList).credentials;
  assert.deepEqual(rows.map((row) => [row.id, row.entryType]), [['P-1', 'Person'], ['P-2', 'Person'], ['A-3', 'Address']]);
  assert.deepEqual(rows[0].addressIds, ['A-2', 'A-1']);
  assert.deepEqual(identityListAsCredentials({ type: 'locked' }), { type: 'locked' });
  assert.equal(addressFormName({ purpose: 'person', number: 2 }), 'registration form 2');

  // The content panel: person rows with their links, the address picker linked-first.
  const routed = chooseIdentityPanel(list);
  assert.equal(routed.panel, 'entries');
  assert.deepEqual(routed.entries.map((row) => row.kind), ['person', 'person', 'address']);
  assert.equal(routed.addresses.length, 3);
  const choices = personAddressChoices(routed.entries[0], routed.addresses);
  assert.deepEqual(choices.linked.map((row) => row.id), ['A-2', 'A-1']);
  assert.deepEqual(choices.others.map((row) => row.id), ['A-3']);
  const none = personAddressChoices(routed.entries[1], routed.addresses);
  assert.deepEqual([none.linked.length, none.others.length], [0, 3]);
  console.log('personRelay.test.js: people-first list, person fill gating and fields, popup person fill, picker OK');
}

run().catch((error) => { console.error(error); process.exit(1); });
