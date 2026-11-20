// Extension-window theme. Stored in chrome.storage.local.
// The language follows the browser UI language (shared/i18n.js), and rows keep letter avatars.
// Neither is a setting here.
//
// One var bag. A top-level const or function here would share the classic-script
// scope with the other extension scripts and can throw at load.

var mypassAppearanceSettings = (function () {
  var i18n = typeof require === 'function' ? require('./i18n.js') : mypassI18n;
  var APPEARANCE_SETTINGS_KEY = 'mypassAppearanceSettings';

  function defaultAppearanceSettings() {
    return { theme: 'system' };
  }

  function normalizeAppearanceSettings(value) {
    var theme = value && typeof value.theme === 'string' ? value.theme : 'system';
    if (theme !== 'light' && theme !== 'dark' && theme !== 'system') theme = 'system';
    return { theme: theme };
  }

  // chrome.commands.getAll reports an empty shortcut when the user has not assigned one.
  function shortcutLabel(shortcut) {
    if (typeof shortcut !== 'string') return i18n.t('shortcutNotSet');
    var trimmed = shortcut.trim();
    return trimmed ? trimmed : i18n.t('shortcutNotSet');
  }

  function commandShortcut(commands, name) {
    var list = Array.isArray(commands) ? commands : [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] && list[i].name === name) return shortcutLabel(list[i].shortcut);
    }
    return i18n.t('shortcutNotSet');
  }

  return {
    APPEARANCE_SETTINGS_KEY: APPEARANCE_SETTINGS_KEY,
    defaultAppearanceSettings: defaultAppearanceSettings,
    normalizeAppearanceSettings: normalizeAppearanceSettings,
    shortcutLabel: shortcutLabel,
    commandShortcut: commandShortcut
  };
})();

if (typeof module !== 'undefined') {
  module.exports = mypassAppearanceSettings;
}
