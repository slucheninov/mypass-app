// Options page entry. Normalises the hash and shows the pane for the route.
// The settings pane holds what used to live behind the popup's gear menu; the element
// ids, storage keys and message types are unchanged so background behaviour is the same.

(function () {
  var router = window.mypassRouter;
  var securityWatch = window.mypassSecurityWatch;
  var autofillSettings = window.mypassAutofillSettings;
  var notificationSettings = window.mypassNotificationSettings;
  var appearanceSettings = window.mypassAppearanceSettings;
  var appearanceTheme = window.mypassTheme;
  var autofillPasskeyRevision = 0;

  function render() {
    var route = router.parseRoute(location.hash);
    var settings = document.getElementById('page-settings');
    var notFound = document.getElementById('page-not-found');
    if (settings) settings.hidden = route.name !== 'settings';
    if (notFound) notFound.hidden = route.name !== 'notFound';
  }

  // Single route today, so the sections are switched in place rather than through the hash.
  function showSection(name) {
    var buttons = document.querySelectorAll('#settings-nav button');
    for (var i = 0; i < buttons.length; i++) {
      var active = buttons[i].getAttribute('data-section') === name;
      if (active) buttons[i].setAttribute('aria-current', 'page');
      else buttons[i].removeAttribute('aria-current');
    }
    var sections = document.querySelectorAll('.settings-section');
    for (var j = 0; j < sections.length; j++) {
      sections[j].hidden = sections[j].id !== 'settings-' + name;
    }
    if (name === 'autofill') refreshAutofillPasskeyToggle();
    if (name === 'appearance') refreshShortcutLabels();
  }

  function initNav() {
    var buttons = document.querySelectorAll('#settings-nav button');
    for (var i = 0; i < buttons.length; i++) {
      (function (button) {
        button.addEventListener('click', function () { showSection(button.getAttribute('data-section')); });
      })(buttons[i]);
    }
    showSection('general');
  }

  function initGeneralSettings() {
    document.getElementById('open-settings-btn').addEventListener('click', () => {
      chrome.runtime.sendMessage({ type: 'openBrowserSettings' }, (result) => {
        void chrome.runtime.lastError;
        const el = document.getElementById('settings-open-error');
        // The app encodes ExtensionBridgeResponse.acknowledged as {"type":"ok"}.
        el.hidden = !!(result && result.type === 'ok');
        el.textContent = result && result.type === 'pairingRequired'
          ? 'Connect this browser to MyPass first, then open settings.'
          : 'Could not open settings. Open MyPass, then choose Settings → Browser.';
      });
    });
    document.getElementById('check-browser-managers-btn').addEventListener('click', () => {
      chrome.tabs.create({ url: chrome.runtime.getURL('browser-check.html') });
    });
  }

  function initSecuritySettings() {
    const pasteToggle = document.getElementById('warn-unrecognized-paste');
    const copyToggle = document.getElementById('warn-unrecognized-copy');
    const key = securityWatch.SECURITY_SETTINGS_KEY;
    function persist() {
      chrome.storage.local.set({
        [key]: { warnPaste: !!pasteToggle.checked, warnCopy: !!copyToggle.checked }
      });
    }
    chrome.storage.local.get([key], (stored) => {
      const settings = securityWatch.normalizeSecuritySettings(stored && stored[key]);
      pasteToggle.checked = settings.warnPaste;
      copyToggle.checked = settings.warnCopy;
    });
    pasteToggle.addEventListener('change', persist);
    copyToggle.addEventListener('change', persist);
    document.getElementById('open-mypass-app-btn').addEventListener('click', () => {
      chrome.runtime.sendMessage({ type: 'openApp' }, (result) => {
        void chrome.runtime.lastError;
        const el = document.getElementById('open-mypass-app-error');
        el.hidden = !!(result && result.type === 'ok');
        el.textContent = result && result.type === 'pairingRequired'
          ? 'Connect this browser to MyPass first.'
          : 'Could not open MyPass. Open the app and try again.';
      });
    });
  }

  function refreshAutofillPasskeyToggle() {
    const revision = ++autofillPasskeyRevision;
    const toggle = document.getElementById('autofill-passkey-signin');
    chrome.runtime.sendMessage({ type: 'getPasskeyProxyStatus' }, (status) => {
      void chrome.runtime.lastError;
      if (revision !== autofillPasskeyRevision) return;
      if (status && status.type === 'passkeyProxyStatus' && typeof status.enabled === 'boolean') {
        toggle.checked = status.enabled;
      }
    });
  }

  function initAutofillSettings() {
    const offerSaveLogin = document.getElementById('offer-save-login');
    const offerSaveTotp = document.getElementById('offer-save-totp');
    const showFieldIcon = document.getElementById('show-field-icon');
    const showLogins = document.getElementById('suggest-logins');
    const showCards = document.getElementById('suggest-cards');
    const showAddresses = document.getElementById('suggest-addresses');
    const autoSubmit = document.getElementById('autofill-auto-submit');
    const passkeyToggle = document.getElementById('autofill-passkey-signin');
    const passkeyError = document.getElementById('autofill-passkey-error');
    const key = autofillSettings.AUTOFILL_SETTINGS_KEY;
    function persistAutofill() {
      chrome.storage.local.set({
        [key]: {
          offerSaveLogin: !!offerSaveLogin.checked,
          offerSaveTotp: !!offerSaveTotp.checked,
          showFieldIcon: !!showFieldIcon.checked,
          showLogins: !!showLogins.checked,
          showCards: !!showCards.checked,
          showAddresses: !!showAddresses.checked,
          autoSubmit: !!autoSubmit.checked
        }
      });
    }
    chrome.storage.local.get([key], (stored) => {
      const settings = autofillSettings.normalizeAutofillSettings(stored && stored[key]);
      offerSaveLogin.checked = settings.offerSaveLogin;
      offerSaveTotp.checked = settings.offerSaveTotp;
      showFieldIcon.checked = settings.showFieldIcon;
      showLogins.checked = settings.showLogins;
      showCards.checked = settings.showCards;
      showAddresses.checked = settings.showAddresses;
      autoSubmit.checked = settings.autoSubmit;
    });
    [offerSaveLogin, offerSaveTotp, showFieldIcon, showLogins, showCards, showAddresses, autoSubmit].forEach((toggle) => {
      toggle.addEventListener('change', persistAutofill);
    });
    passkeyToggle.addEventListener('change', () => {
      const revision = ++autofillPasskeyRevision;
      const next = !!passkeyToggle.checked;
      chrome.runtime.sendMessage({ type: 'setPasskeyProxyEnabled', enabled: next }, (result) => {
        void chrome.runtime.lastError;
        if (revision !== autofillPasskeyRevision) return;
        if (result && result.type === 'passkeyProxyEnabled' && typeof result.enabled === 'boolean') {
          passkeyToggle.checked = result.enabled;
          passkeyError.hidden = true;
          return;
        }
        passkeyToggle.checked = !next;
        passkeyError.hidden = false;
        passkeyError.textContent = result && result.type === 'locked'
          ? 'Unlock MyPass, then change passkeys.'
          : result && result.type === 'pairingRequired'
            ? 'Connect this browser to MyPass first.'
            : 'Could not change passkeys. Open MyPass and try again.';
      });
    });
    refreshAutofillPasskeyToggle();
  }

  function initNotificationSettings() {
    const passkeyToggle = document.getElementById('notify-passkey-confirmation');
    const unlockToggle = document.getElementById('notify-vault-unlocked');
    const key = notificationSettings.NOTIFICATION_SETTINGS_KEY;
    function persist() {
      chrome.storage.local.set({
        [key]: {
          passkeyConfirmation: !!passkeyToggle.checked,
          vaultUnlocked: !!unlockToggle.checked
        }
      });
    }
    chrome.storage.local.get([key], (stored) => {
      const settings = notificationSettings.normalizeNotificationSettings(stored && stored[key]);
      passkeyToggle.checked = settings.passkeyConfirmation;
      unlockToggle.checked = settings.vaultUnlocked;
    });
    passkeyToggle.addEventListener('change', persist);
    unlockToggle.addEventListener('change', persist);
  }

  function paintShortcutLabels(commands) {
    document.getElementById('shortcut-open').textContent = appearanceSettings.commandShortcut(commands, '_execute_action');
    document.getElementById('shortcut-lock').textContent = appearanceSettings.commandShortcut(commands, 'lock-vault');
  }

  function refreshShortcutLabels() {
    if (!chrome.commands || typeof chrome.commands.getAll !== 'function') {
      paintShortcutLabels([]);
      return;
    }
    chrome.commands.getAll((commands) => {
      void chrome.runtime.lastError;
      paintShortcutLabels(commands);
    });
  }

  function initAppearanceSettings() {
    const themeSelect = document.getElementById('appearance-theme');
    const key = appearanceSettings.APPEARANCE_SETTINGS_KEY;
    function persistTheme() {
      const theme = appearanceSettings.normalizeAppearanceSettings({ theme: themeSelect.value }).theme;
      themeSelect.value = theme;
      appearanceTheme.applyAppearanceTheme(theme);
      chrome.storage.local.set({ [key]: { theme: theme } });
    }
    chrome.storage.local.get([key], (stored) => {
      const settings = appearanceSettings.normalizeAppearanceSettings(stored && stored[key]);
      themeSelect.value = settings.theme;
      appearanceTheme.applyAppearanceTheme(settings.theme);
    });
    themeSelect.addEventListener('change', persistTheme);
    document.getElementById('open-shortcut-settings').addEventListener('click', () => {
      chrome.tabs.create({ url: 'chrome://extensions/shortcuts' });
    });
    refreshShortcutLabels();
  }

  var canonical = router.canonicalHash(location.hash);
  if (canonical !== location.hash) {
    history.replaceState(null, '', canonical);
  }
  window.addEventListener('hashchange', render);
  initNav();
  initGeneralSettings();
  initSecuritySettings();
  initAutofillSettings();
  initNotificationSettings();
  initAppearanceSettings();
  render();
})();
