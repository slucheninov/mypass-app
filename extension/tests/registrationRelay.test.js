// The registration form relay: the identity icon's list and fill requests reach the native host
// only from the top frame of the https page they name, with the address switch on; the fill
// answers with the address fields only. Also the content side: the panel choice for the list
// answer and the save prompt that carries the captured form fields.
const assert = require('node:assert/strict');
if (!global.crypto) global.crypto = require('node:crypto').webcrypto;
// Loaded before `chrome` exists, so content.js only defines its functions.
const { chooseIdentityPanel, requestSavePrompt } = require('../content.js');
const formDetection = require('../shared/form-detection.js');

let handler, native, showAddresses, nativeReply;
const extensionId = 'test-extension';
const page = 'https://shop.example.test/register';
const content = (extra = {}) => ({ id: extensionId, tab: { id: 7 }, frameId: 0, documentId: 'doc-1', url: page, ...extra });
global.chrome = {
  runtime: { id: extensionId, lastError: null, getURL: (path) => 'chrome-extension://' + extensionId + '/' + path,
    onMessage: { addListener: (fn) => { handler = fn; } },
    sendNativeMessage(_host, message, callback) {
      native.push(message);
      callback(nativeReply(message));
    },
  },
  storage: { local: { get: (_keys, cb) => cb({ pairingSecret: 'paired', mypassAutofillSettings: { showAddresses } }) } },
  tabs: { query: (_options, cb) => cb([]), sendMessage: () => {} },
};
require('../background.js');

function reset() {
  native = [];
  showAddresses = true;
  nativeReply = (message) => message.type === 'listEntries'
    ? { type: 'credentialList', credentials: [
      { id: 'a1', title: 'Home', username: 'Ann Lee', password: 'never-sent' },
      { id: 'x'.repeat(65), title: 'Too long an id' },
      { id: 'a2', title: 'Work' },
    ] }
    : { type: 'address', address: {
      id: 'A1', fullName: 'Ann Lee', city: 'Lviv', email: 'ann@example.test', notes: 'private', password: 'nope', phone: 7,
    } };
}
function request(message, sender) {
  return new Promise((resolve) => {
    if (!handler(message, sender, resolve)) resolve(undefined);
  });
}

async function run() {
  reset();
  const list = await request({ type: 'getIdentitySuggestions', url: page }, content());
  assert.deepEqual(list, { type: 'identityList', entries: [
    { id: 'a1', title: 'Home', username: 'Ann Lee' },
    { id: 'a2', title: 'Work', username: '' },
  ] });
  assert.deepEqual(native, [{ type: 'listEntries', category: 'addresses', pairing: 'paired' }]);

  reset();
  const fill = await request({ type: 'fillIdentity', id: 'a1', url: page }, content());
  assert.equal(fill.type, 'identity');
  assert.deepEqual(native, [{ type: 'fillAddress', id: 'a1', url: page, pairing: 'paired' }]);
  assert.equal(fill.address.city, 'Lviv');
  assert.equal(fill.address.phone, '', 'a non-string value is dropped');
  assert.equal(fill.address.notes, undefined, 'only the address fields pass');
  assert.equal(fill.address.password, undefined);
  assert.equal(fill.address.id, undefined);

  // Anything but the top frame of the named https page is refused before the native host.
  const refused = [
    content({ frameId: 3 }),
    content({ documentId: undefined }),
    content({ url: 'http://shop.example.test/register' }),
    content({ url: 'https://other.example.test/register' }),
    content({ tab: undefined }),
    { id: extensionId, url: 'chrome-extension://' + extensionId + '/popup.html' },
    content({ id: 'other-extension' }),
  ];
  for (const sender of refused) {
    reset();
    for (const message of [{ type: 'getIdentitySuggestions', url: page }, { type: 'fillIdentity', id: 'a1', url: page }]) {
      assert.equal((await request(message, sender)).code, 'address_unauthorized', JSON.stringify(sender));
    }
    assert.equal(native.length, 0);
  }

  // The address switch is read from storage on every request.
  reset();
  showAddresses = false;
  assert.equal((await request({ type: 'getIdentitySuggestions', url: page }, content())).code, 'address_disabled');
  assert.equal((await request({ type: 'fillIdentity', id: 'a1', url: page }, content())).code, 'address_disabled');
  assert.equal(native.length, 0);

  // A bad id never reaches the host; an answer for another entry is not handed out.
  reset();
  for (const id of ['', 'y'.repeat(65), 42]) {
    assert.equal((await request({ type: 'fillIdentity', id, url: page }, content())).code, 'address_unavailable');
  }
  assert.equal(native.length, 0);
  assert.equal((await request({ type: 'fillIdentity', id: 'b2', url: page }, content())).code, 'address_unavailable');

  reset();
  nativeReply = () => ({ type: 'locked' });
  assert.deepEqual(await request({ type: 'getIdentitySuggestions', url: page }, content()), { type: 'locked' });
  assert.deepEqual(await request({ type: 'fillIdentity', id: 'a1', url: page }, content()), { type: 'locked' });

  // The panel for each list answer.
  assert.equal(chooseIdentityPanel({ type: 'locked' }).panel, 'locked');
  assert.equal(chooseIdentityPanel({ type: 'pairingRequired' }).panel, 'pairingRequired');
  assert.equal(chooseIdentityPanel({ type: 'error', code: 'address_disabled' }).panel, 'unavailable');
  assert.equal(chooseIdentityPanel(undefined).panel, 'unavailable');
  assert.equal(chooseIdentityPanel({ type: 'identityList', entries: [] }).panel, 'identityEmpty');
  assert.deepEqual(chooseIdentityPanel({ type: 'identityList', entries: [{ id: 'a1', title: 'Home' }, { id: 3 }] }),
    { panel: 'entries', entries: [{ id: 'a1', title: 'Home', username: '', hasTotp: false }] });

  // The save prompt carries the captured fields, and only when there are some.
  const messages = [];
  const send = (message, reply) => { messages.push(message); reply({ type: 'ok', created: true }); };
  const fields = [{ kind: 'givenName', label: '', value: 'Ann' }];
  assert.equal(requestSavePrompt(page, 'ann@example.test', 'pw-1', {}, send, 1000, fields), true);
  assert.equal(requestSavePrompt(page, 'ann@example.test', 'pw-2', {}, send, 1000, []), true);
  assert.deepEqual(messages[0].fields, fields);
  assert.equal(Object.hasOwn(messages[1], 'fields'), false);

  // A corrected retry of the same form (same login, other field values) asks again; the very
  // same submit inside the dedupe window does not. The username-is-email flag travels.
  messages.length = 0;
  const retry = {};
  const first = [{ kind: 'givenName', label: '', value: 'Ann' }];
  const corrected = [{ kind: 'givenName', label: '', value: 'Anna' }];
  assert.equal(requestSavePrompt(page, 'ann@example.test', 'pw-3', retry, send, 1000, first, true), true);
  assert.equal(requestSavePrompt(page, 'ann@example.test', 'pw-3', retry, send, 2000, first, true), false);
  assert.equal(requestSavePrompt(page, 'ann@example.test', 'pw-3', retry, send, 3000, corrected, true), true);
  assert.equal(messages.length, 2);
  assert.equal(messages[0].usernameIsEmail, true);
  assert.ok(!JSON.stringify(retry).includes('Anna'), 'the dedupe state keeps no field value');
  assert.equal(requestSavePrompt(page, 'me', 'pw-4', {}, send, 1000), true);
  assert.equal(Object.hasOwn(messages[2], 'usernameIsEmail'), false);

  // The localized hints the detector relies on.
  const node = (attrs) => ({ name: '', id: '', labels: [], getAttribute: (name) => attrs[name] ?? null, ...attrs });
  assert.equal(formDetection.identityFieldHint(node({ name: 'given-name' })), 'givenName');
  assert.equal(formDetection.identityFieldHint(node({ placeholder: '\u041f\u0440\u0456\u0437\u0432\u0438\u0449\u0435' })), 'familyName');
  assert.equal(formDetection.identityFieldHint(node({ 'aria-label': 'Geburtsdatum' })), 'birthday');
  assert.equal(formDetection.isSensitiveFieldHint(node({ placeholder: 'Card number' })), true);
  assert.equal(formDetection.isRegistrationActionText('Konto erstellen'), true);
  assert.equal(formDetection.isRegistrationActionText('\u0417\u0430\u0440\u0435\u0454\u0441\u0442\u0440\u0443\u0432\u0430\u0442\u0438\u0441\u044f'), true);
  assert.equal(formDetection.isRegistrationActionText('Sign in'), false);
  console.log('registrationRelay.test.js: identity list and fill gating, field whitelist, panel, prompt fields OK');
}

run().catch((error) => { console.error(error); process.exit(1); });
