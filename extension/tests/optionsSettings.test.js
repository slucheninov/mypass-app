// Behaviour of the settings pane on the options page (app/app.js). These checks moved here
// from the popup tests when the gear menu stopped hosting settings.
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const root = path.join(__dirname, '..');
const elements = {};
function el(id) {
  if (!elements[id]) {
    elements[id] = {
      id, hidden: false, checked: false, value: '', textContent: '', listeners: {},
      addEventListener(type, fn) { (this.listeners[type] = this.listeners[type] || []).push(fn); },
      setAttribute() {}, removeAttribute() {}, getAttribute() { return null; }
    };
  }
  return elements[id];
}
// Defaults match the markup: most switches start on, auto-submit and passkey start off.
for (const id of ['warn-unrecognized-paste', 'warn-unrecognized-copy', 'offer-save-login', 'offer-save-totp', 'show-field-icon',
  'suggest-logins', 'suggest-cards', 'suggest-addresses', 'notify-passkey-confirmation', 'notify-vault-unlocked']) el(id).checked = true;
el('appearance-theme').value = 'system';

const sent = [];
const created = [];
let respond = (msg) => ({ type: 'ok' });
const stored = {};
const chrome = {
  runtime: {
    lastError: undefined,
    getURL: (p) => 'chrome-extension://x/' + p,
    sendMessage(msg, cb) { sent.push(msg); if (cb) cb(msg.type === 'getPasskeyProxyStatus'
      ? { type: 'passkeyProxyStatus', enabled: true } : respond(msg)); }
  },
  tabs: { create: (o) => created.push(o) },
  commands: { getAll: (cb) => cb([]) },
  storage: { local: { get: (keys, cb) => cb(stored), set: (items, cb) => { Object.assign(stored, items); if (cb) cb(); } } }
};
const sandbox = {
  chrome, location: { hash: '' }, history: { replaceState() {} },
  window: null,
  document: {
    getElementById: el,
    querySelectorAll: () => [],
    documentElement: { setAttribute() {} }
  }
};
sandbox.window = sandbox;
sandbox.addEventListener = () => {};
vm.createContext(sandbox);
for (const file of ['shared/autofill-settings.js', 'shared/notification-settings.js', 'shared/security-watch.js',
  'shared/appearance-settings.js', 'shared/theme.js', 'app/router.js', 'app/app.js']) {
  vm.runInContext(fs.readFileSync(path.join(root, file), 'utf8') + '\n', sandbox, { filename: file });
}

// General: opens the macOS app settings with the existing message type.
el('open-settings-btn').listeners.click[0]();
assert.strictEqual(sent.at(-1).type, 'openBrowserSettings');
assert.strictEqual(el('settings-open-error').hidden, true);
respond = () => ({ type: 'error', code: 'app_not_running' });
el('open-settings-btn').listeners.click[0]();
assert.strictEqual(el('settings-open-error').hidden, false);
assert.match(el('settings-open-error').textContent, /Open MyPass.*Settings → Browser/);
respond = () => ({ type: 'ok' });
el('open-settings-btn').listeners.click[0]();
assert.strictEqual(el('settings-open-error').hidden, true);
el('check-browser-managers-btn').listeners.click[0]();
assert.strictEqual(created.at(-1).url, 'chrome-extension://x/browser-check.html');

// Security.
assert.strictEqual(el('warn-unrecognized-paste').checked, true);
el('warn-unrecognized-paste').checked = false;
el('warn-unrecognized-paste').listeners.change[0]();
assert.deepStrictEqual(JSON.parse(JSON.stringify(stored.mypassSecuritySettings)), { warnPaste: false, warnCopy: true });
el('open-mypass-app-btn').listeners.click[0]();
assert.strictEqual(sent.at(-1).type, 'openApp');
assert.strictEqual(el('open-mypass-app-error').hidden, true);
respond = () => ({ type: 'pairingRequired' });
el('open-mypass-app-btn').listeners.click[0]();
assert.match(el('open-mypass-app-error').textContent, /Connect this browser/);
respond = () => ({ type: 'ok' });

// Autofill and save.
assert.strictEqual(el('autofill-auto-submit').checked, false);
assert.strictEqual(el('autofill-passkey-signin').checked, true, 'the passkey switch follows the app');
el('offer-save-login').checked = false;
el('autofill-auto-submit').checked = true;
el('offer-save-login').listeners.change[0]();
const autofill = stored.mypassAutofillSettings;
assert.strictEqual(autofill.offerSaveLogin, false);
assert.strictEqual(autofill.autoSubmit, true);
assert.strictEqual(autofill.offerSaveTotp, true);
assert.ok(!Object.prototype.hasOwnProperty.call(autofill, 'passkeyProxyEnabled'));
el('autofill-passkey-signin').checked = false;
respond = (msg) => msg.type === 'setPasskeyProxyEnabled' ? { type: 'passkeyProxyEnabled', enabled: false } : { type: 'ok' };
el('autofill-passkey-signin').listeners.change[0]();
assert.strictEqual(sent.at(-1).type, 'setPasskeyProxyEnabled');
assert.strictEqual(sent.at(-1).enabled, false);
assert.strictEqual(el('autofill-passkey-error').hidden, true);
respond = () => ({ type: 'locked' });
el('autofill-passkey-signin').checked = true;
el('autofill-passkey-signin').listeners.change[0]();
assert.strictEqual(el('autofill-passkey-signin').checked, false, 'a locked vault puts the switch back');
assert.match(el('autofill-passkey-error').textContent, /Unlock MyPass/);

// Notifications.
el('notify-passkey-confirmation').checked = false;
el('notify-passkey-confirmation').listeners.change[0]();
assert.strictEqual(stored.mypassNotificationSettings.passkeyConfirmation, false);
assert.strictEqual(stored.mypassNotificationSettings.vaultUnlocked, true);

// Appearance and shortcuts.
assert.strictEqual(el('shortcut-open').textContent, 'Not set');
assert.strictEqual(el('shortcut-lock').textContent, 'Not set');
el('appearance-theme').value = 'dark';
el('appearance-theme').listeners.change[0]();
assert.strictEqual(stored.mypassAppearanceSettings.theme, 'dark');
el('open-shortcut-settings').listeners.click[0]();
assert.strictEqual(created.at(-1).url, 'chrome://extensions/shortcuts');

console.log('optionsSettings.test.js: OK');
