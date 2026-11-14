// Applies the saved theme to this extension page. The page icon on a website
// keeps following that site; this only paints MyPass windows.

var mypassTheme = (function () {
  function appearanceApi() {
    if (typeof mypassAppearanceSettings !== 'undefined') return mypassAppearanceSettings;
    if (typeof require === 'function') return require('./appearance-settings.js');
    return null;
  }

  function applyAppearanceTheme(theme) {
    var api = appearanceApi();
    var value = api
      ? api.normalizeAppearanceSettings({ theme: theme }).theme
      : (theme === 'light' || theme === 'dark' ? theme : 'system');
    if (typeof document !== 'undefined' && document.documentElement
      && typeof document.documentElement.setAttribute === 'function') {
      document.documentElement.setAttribute('data-theme', value);
    }
    return value;
  }

  function readAndApplyAppearanceTheme() {
    var api = appearanceApi();
    if (!api || typeof chrome === 'undefined' || !chrome.storage || !chrome.storage.local) return;
    var key = api.APPEARANCE_SETTINGS_KEY;
    chrome.storage.local.get([key], function (stored) {
      applyAppearanceTheme(api.normalizeAppearanceSettings(stored && stored[key]).theme);
    });
    if (chrome.storage.onChanged && !mypassTheme.watching) {
      mypassTheme.watching = true;
      chrome.storage.onChanged.addListener(function (changes, areaName) {
        if (areaName !== 'local' || !changes[key]) return;
        applyAppearanceTheme(api.normalizeAppearanceSettings(changes[key].newValue).theme);
      });
    }
  }

  return {
    watching: false,
    applyAppearanceTheme: applyAppearanceTheme,
    readAndApplyAppearanceTheme: readAndApplyAppearanceTheme
  };
})();

if (typeof document !== 'undefined' && typeof chrome !== 'undefined') {
  mypassTheme.readAndApplyAppearanceTheme();
}

if (typeof module !== 'undefined') {
  module.exports = mypassTheme;
}
