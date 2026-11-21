// Settings live on the options page (app/app.html), not in the popup. The popup menu keeps
// entry creation, Lock and Help; the moved controls keep their element ids and storage keys.
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

const { test } = require('node:test');
const { openPopup } = require('./popupHarness.js');

function menuPopup(lock = { type: 'ok' }) {
  return openPopup({ reply(message) {
    if (message.type === 'getCredentials') return { type: 'credentialList', credentials: [] };
    if (message.type === 'lockVault') return lock;
    return {};
  } });
}

test('the plus menu lists New login, New card, New note, a separator, Lock MyPass and Help', () => {
  const h = menuPopup();
  h.fire('add-entry-btn', 'click');
  assert.deepStrictEqual(h.el('plus-entry-actions').children.map((row) => row.children[1].textContent),
    ['New login', 'New card', 'New note']);
  const root = popupHtml.match(/<div id="plus-menu-root">([\s\S]+?)<p id="lock-vault-error"/)?.[1];
  assert.match(root, /id="plus-entry-actions"[\s\S]*<hr class="menu-separator"[\s\S]*id="lock-vault-btn"[\s\S]*id="plus-menu-help-btn"/);
});

test('Help opens the help view, Back returns to root, and reopening resets to root', () => {
  const h = menuPopup();
  h.fire('add-entry-btn', 'click');
  assert.equal(h.el('add-entry-btn').getAttribute('aria-expanded'), 'true');
  h.fire('plus-menu-help-btn', 'click');
  assert.equal(h.el('plus-menu-help').hidden, false);
  assert.equal(h.el('plus-menu-root').hidden, true);
  h.fire('plus-menu-help-back', 'click');
  assert.equal(h.el('plus-menu-help').hidden, true);
  assert.equal(h.el('plus-menu-root').hidden, false);
  h.fire('plus-menu-help-btn', 'click');
  h.fire('add-entry-btn', 'click');
  assert.equal(h.el('add-entry-menu').hidden, true);
  h.fire('add-entry-btn', 'click');
  assert.equal(h.el('plus-menu-root').hidden, false);
  assert.equal(h.el('plus-menu-help').hidden, true);
});

test('Lock MyPass retains its replies and errors', () => {
  for (const outcome of ['ok', 'locked', 'pairingRequired', 'error']) {
    const h = menuPopup({ type: outcome });
    h.fire('add-entry-btn', 'click');
    h.fire('lock-vault-btn', 'click');
    if (outcome === 'ok' || outcome === 'locked') {
      assert.equal(h.el('locked-panel').hidden, false);
      assert.equal(h.el('add-entry-menu').hidden, true);
    } else {
      assert.equal(h.el('add-entry-menu').hidden, false);
      assert.equal(h.el('lock-vault-error').hidden, false);
      assert.match(h.el('lock-vault-error').textContent, outcome === 'error' ? /lock/i : /Connect this browser/);
    }
  }
});

test('the generator remains and the more menu is gone', () => {
  assert.match(popupHtml, /id="open-generator-btn"/);
  assert.doesNotMatch(popupHtml, /id="(?:popup-menu-btn|popup-menu)"/);
});
