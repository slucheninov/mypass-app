// Extension-window theme. Stored in chrome.storage.local.
// Language stays English, and rows keep letter avatars. Those are not settings here.
//
// One var bag. A top-level const or function here would share the classic-script
// scope with the other extension scripts and can throw at load.

var mypassAppearanceSettings = (function () {
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
    if (typeof shortcut !== 'string') return 'Not set';
    var trimmed = shortcut.trim();
    return trimmed ? trimmed : 'Not set';
  }

  function commandShortcut(commands, name) {
    var list = Array.isArray(commands) ? commands : [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] && list[i].name === name) return shortcutLabel(list[i].shortcut);
    }
    return 'Not set';
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
