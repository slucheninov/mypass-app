const i18n = typeof require === 'function'
  ? require('./shared/i18n.js')
  : window.mypassI18n;

// ExtensionBridgeResponse.acknowledged is encoded on the wire as {"type":"ok"}.
function settingsOpenConfirmed(result) {
  return !!(result && result.type === 'ok');
}

function describeBrowserCheck(result) {
  if (!result || result.permissionState === 'unavailable') {
    return { status: i18n.t('checkPermissionsUnreadable'), visible: false };
  }
  // contains() is false both before Chrome asks and after a refusal. Only a request
  // callback in this page can tell those apart; the background reports the first as not-granted.
  if (result.permissionState === 'denied') {
    return { status: i18n.t('checkPermissionDenied'), visible: false };
  }
  if (result.permissionState !== 'granted') {
    return { status: i18n.t('permissionNotRequested'), visible: false };
  }
  return {
    status: result.reported
      ? i18n.t('checkResultsSent')
      : i18n.t('checkResultsLocal'),
    visible: true,
    passwordSaving: result.passwordSavingEnabled === true ? i18n.t('passwordSavingOn')
      : result.passwordSavingEnabled === false ? i18n.t('passwordSavingOff') : i18n.t('couldNotBeRead'),
    extensions: result.extensionScanState !== 'complete' ? i18n.t('couldNotBeChecked')
      : result.extensions.length ? result.extensions.map((item) => item.name).join(', ') : i18n.t('noCompetingExtensions')
  };
}

if (typeof document !== 'undefined') {
  i18n.apply(document);
  const checkButton = document.getElementById('check-btn');
  const status = document.getElementById('check-status');
  function refresh() {
    chrome.runtime.sendMessage({ type: 'getBrowserManagerDiagnostics' }, (result) => {
      void chrome.runtime.lastError;
      const description = describeBrowserCheck(result);
      status.textContent = description.status;
      document.getElementById('results').hidden = !description.visible;
      document.getElementById('password-saving-result').textContent = description.passwordSaving || '';
      document.getElementById('extensions-result').textContent = description.extensions || '';
      checkButton.disabled = false;
      checkButton.textContent = i18n.t(result && result.permissionState === 'granted' ? 'checkAgain' : 'allowAndCheck');
    });
  }
  checkButton.addEventListener('click', () => {
    checkButton.disabled = true;
    // Request directly from the click. An async status fetch first loses user activation.
    if (!chrome.permissions) { refresh(); return; }
    chrome.permissions.request({ permissions: ['privacy', 'management'] }, (granted) => {
      void chrome.runtime.lastError;
      if (granted) { refresh(); return; }
      const description = describeBrowserCheck({ permissionState: 'denied' });
      status.textContent = description.status;
      document.getElementById('results').hidden = true;
      checkButton.disabled = false;
      checkButton.textContent = i18n.t('allowAndCheck');
    });
  });
  document.getElementById('open-settings-btn').addEventListener('click', () => {
    chrome.runtime.sendMessage({ type: 'openBrowserSettings' }, (result) => {
      void chrome.runtime.lastError;
      if (!settingsOpenConfirmed(result)) {
        status.textContent = i18n.t('settingsOpenFailed');
      }
    });
  });
  document.getElementById('password-settings-btn').addEventListener('click', () => {
    chrome.tabs.create({ url: 'chrome://password-manager/settings' });
  });
  document.getElementById('extensions-settings-btn').addEventListener('click', () => {
    chrome.tabs.create({ url: 'chrome://extensions' });
  });
  refresh();
}

if (typeof module !== 'undefined') module.exports = { describeBrowserCheck, settingsOpenConfirmed };
