const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const appearance = require('../shared/appearance-settings.js');

assert.strictEqual(appearance.defaultAppearanceSettings().theme, 'system');
assert.deepStrictEqual(appearance.normalizeAppearanceSettings(null), { theme: 'system' });
assert.deepStrictEqual(appearance.normalizeAppearanceSettings({ theme: 'purple' }), { theme: 'system' });
assert.deepStrictEqual(appearance.normalizeAppearanceSettings({ theme: 'dark' }), { theme: 'dark' });
assert.strictEqual(appearance.shortcutLabel(''), 'Not set');
assert.strictEqual(appearance.shortcutLabel('  Command+Shift+L  '), 'Command+Shift+L');
assert.strictEqual(
  appearance.commandShortcut([{ name: '_execute_action', shortcut: 'Ctrl+Shift+Y' }, { name: 'lock-vault', shortcut: '' }], 'lock-vault'),
  'Not set'
);
assert.strictEqual(
  appearance.commandShortcut([{ name: '_execute_action', shortcut: 'Command+Shift+Y' }], '_execute_action'),
  'Command+Shift+Y'
);

const root = path.join(__dirname, '..');
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8'));
assert.strictEqual(manifest.commands._execute_action.description, 'Open MyPass');
assert.strictEqual(manifest.commands['lock-vault'].description, 'Lock MyPass');
assert.strictEqual(manifest.commands['lock-vault'].suggested_key.mac, 'Command+Shift+L');
assert.ok(!manifest.content_scripts.some((entry) => (entry.js || []).includes('shared/theme.js')));

const background = fs.readFileSync(path.join(root, 'background.js'), 'utf8');
assert.ok(background.includes('lock-vault'));
assert.ok(background.includes('vaultLockedNotice'));
assert.ok(!background.includes('"lock-vault"') || background.includes('command === "lock-vault"'));
const knownTypes = background.slice(background.indexOf('const knownTypes'), background.indexOf('];'));
assert.ok(!knownTypes.includes('lock-vault'), 'the shortcut is not a page message');

const popup = fs.readFileSync(path.join(root, 'app', 'app.js'), 'utf8');
assert.ok(popup.includes('mypassAppearanceSettings'));
assert.ok(popup.includes('chrome://extensions/shortcuts'));
assert.ok(!popup.includes('favicon'));

console.log('appearanceSettings.test.js: OK');
