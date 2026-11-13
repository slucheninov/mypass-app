// Which system notifications this browser may show. Stored in chrome.storage.local.
// Both default on. There is no breach list: MyPass does not scan sites for that.
//
// One var bag. A top-level const or function here would share the classic-script
// scope with the other extension scripts and can throw at load.

var mypassNotificationSettings = (function () {
  var NOTIFICATION_SETTINGS_KEY = 'mypassNotificationSettings';

  function defaultNotificationSettings() {
    return {
      passkeyConfirmation: true,
      vaultUnlocked: true
    };
  }

  function normalizeNotificationSettings(value) {
    var defaults = defaultNotificationSettings();
    if (!value || typeof value !== 'object') return defaults;
    function flag(name) {
      return typeof value[name] === 'boolean' ? value[name] : defaults[name];
    }
    return {
      passkeyConfirmation: flag('passkeyConfirmation'),
      vaultUnlocked: flag('vaultUnlocked')
    };
  }

  // previous is the last boolean unlock reading this worker saw, or null before
  // the first one. Notify only on false → true. null → true is a worker that
  // woke while the vault was already open, and must stay quiet. A reading that
  // is not a boolean leaves previous alone, so a host error cannot look like a lock.
  function noteVaultUnlock(previous, unlocked) {
    if (typeof unlocked !== 'boolean') {
      return { previous: previous, notify: false };
    }
    return {
      previous: unlocked,
      notify: previous === false && unlocked === true
    };
  }

  return {
    NOTIFICATION_SETTINGS_KEY: NOTIFICATION_SETTINGS_KEY,
    defaultNotificationSettings: defaultNotificationSettings,
    normalizeNotificationSettings: normalizeNotificationSettings,
    noteVaultUnlock: noteVaultUnlock
  };
})();

if (typeof module !== 'undefined') {
  module.exports = mypassNotificationSettings;
}
