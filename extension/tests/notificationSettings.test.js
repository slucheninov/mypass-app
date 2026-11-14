const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const notifications = require('../shared/notification-settings.js');

const defaults = notifications.defaultNotificationSettings();
assert.strictEqual(defaults.passkeyConfirmation, true);
assert.strictEqual(defaults.vaultUnlocked, true);
assert.deepStrictEqual(notifications.normalizeNotificationSettings(null), defaults);
assert.deepStrictEqual(notifications.normalizeNotificationSettings('on'), defaults);
assert.strictEqual(
  notifications.normalizeNotificationSettings({ passkeyConfirmation: false, vaultUnlocked: 'yes' }).passkeyConfirmation,
  false
);
assert.strictEqual(
  notifications.normalizeNotificationSettings({ passkeyConfirmation: false }).vaultUnlocked,
  true,
  'a partial record keeps the unlock notification on'
);
assert.ok(
  !Object.prototype.hasOwnProperty.call(defaults, 'watchtower'),
  'breach warnings are not a notification setting'
);

assert.deepStrictEqual(
  notifications.noteVaultUnlock(null, true),
  { previous: true, notify: false },
  'a worker that starts while the vault is open stays quiet'
);
assert.deepStrictEqual(notifications.noteVaultUnlock(null, false), { previous: false, notify: false });
assert.deepStrictEqual(
  notifications.noteVaultUnlock(false, true),
  { previous: true, notify: true },
  'locked then unlocked is the only transition that notifies'
);
assert.deepStrictEqual(notifications.noteVaultUnlock(true, true), { previous: true, notify: false });
assert.deepStrictEqual(notifications.noteVaultUnlock(true, false), { previous: false, notify: false });
assert.deepStrictEqual(
  notifications.noteVaultUnlock(true, undefined),
  { previous: true, notify: false },
  'a missing unlock flag is not a lock'
);
assert.deepStrictEqual(notifications.noteVaultUnlock(false, 'yes'), { previous: false, notify: false });

const root = path.join(__dirname, '..');
const background = fs.readFileSync(path.join(root, 'background.js'), 'utf8');
const popup = fs.readFileSync(path.join(root, 'app', 'app.js'), 'utf8');
const loader = fs.readFileSync(path.join(root, 'background-loader.js'), 'utf8');
const manifest = fs.readFileSync(path.join(root, 'manifest.json'), 'utf8');

assert.ok(background.includes('noteVaultUnlock'), 'the worker uses the unlock transition');
assert.ok(background.includes('passkeyConfirmation'), 'the passkey notification checks its switch');
assert.ok(background.includes('"mypass-unlocked"'));
assert.ok(background.includes('MyPass is unlocked.'));
assert.ok(background.includes('mypass-passkey'));
assert.ok(!background.includes('Watchtower'));
assert.ok(!background.includes('openAppRequested'));
assert.ok(popup.includes('mypassNotificationSettings'));
assert.ok(popup.includes('notify-passkey-confirmation'));
assert.ok(popup.includes('notify-vault-unlocked'));
const settingsImport = loader.indexOf("importScripts('shared/notification-settings.js')");
const backgroundImport = loader.indexOf("importScripts('background.js')");
assert.ok(settingsImport !== -1 && settingsImport < backgroundImport, 'the worker loads notification settings before background.js');
assert.ok(!manifest.includes('notification-settings.js'), 'notification settings are not a content script');

console.log('notificationSettings.test.js: OK');
