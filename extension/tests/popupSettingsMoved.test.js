// Settings live on the options page (app/app.html), not in the popup. The popup menu keeps
// Lock and Help only; the moved controls keep their element ids and storage keys.
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const popupHtml = fs.readFileSync(path.join(root, 'popup.html'), 'utf8');
const popupJs = fs.readFileSync(path.join(root, 'popup.js'), 'utf8');
const appHtml = fs.readFileSync(path.join(root, 'app', 'app.html'), 'utf8');
const appJs = fs.readFileSync(path.join(root, 'app', 'app.js'), 'utf8');

const movedIds = [
  'popup-menu-settings-btn', 'popup-menu-open-appearance', 'appearance-theme',
  'warn-unrecognized-paste', 'warn-unrecognized-copy', 'open-settings-btn', 'settings-open-error',
  'check-browser-managers-btn', 'open-mypass-app-btn', 'offer-save-login', 'offer-save-totp',
  'show-field-icon', 'autofill-auto-submit', 'autofill-passkey-signin', 'suggest-logins',
  'suggest-cards', 'suggest-addresses', 'notify-passkey-confirmation', 'notify-vault-unlocked',
  'shortcut-open', 'shortcut-lock', 'open-shortcut-settings'
];
for (const id of movedIds) {
  assert.ok(!popupHtml.includes(id), 'popup.html must not contain ' + id);
}
for (const id of movedIds.filter((id) => !id.startsWith('popup-menu-'))) {
  assert.ok(appHtml.includes('id="' + id + '"'), 'app.html must contain ' + id);
}

assert.ok(popupHtml.includes('lock-vault-btn'));
assert.ok(popupHtml.includes('popup-menu-help-btn'));

for (const name of ['initAppearanceSettings', 'initNotificationSettings', 'initAutofillSettings', 'initSecuritySettings']) {
  assert.ok(!popupJs.includes(name), 'popup.js must not contain ' + name);
}
assert.ok(!popupJs.includes('openSettingsBtn'));

assert.ok(appJs.includes('normalizeAutofillSettings'));
assert.ok(appJs.includes('normalizeNotificationSettings'));
assert.ok(appJs.includes('normalizeAppearanceSettings'));
assert.ok(appJs.includes('normalizeSecuritySettings'));
assert.ok(appJs.includes("type: 'openBrowserSettings'"));

// No inline scripts (MV3 CSP): every <script> carries a src.
assert.ok(!/<script(?![^>]*\ssrc=)[^>]*>/i.test(appHtml), 'app.html must not use inline scripts');
for (const src of ['autofill-settings', 'notification-settings', 'appearance-settings', 'security-watch', 'theme']) {
  assert.ok(appHtml.includes('../shared/' + src + '.js'), 'app.html loads ' + src);
}

console.log('popupSettingsMoved ok');
