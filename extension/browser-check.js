// ExtensionBridgeResponse.acknowledged is encoded on the wire as {"type":"ok"}.
function settingsOpenConfirmed(result) {
  return !!(result && result.type === 'ok');
}

function describeBrowserCheck(result) {
  if (!result || result.permissionState === 'unavailable') {
    return { status: 'Browser permissions could not be read. Try the check again.', visible: false };
  }
  // contains() is false both before Chrome asks and after a refusal. Only a request
  // callback in this page can tell those apart; the background reports the first as not-granted.
  if (result.permissionState === 'denied') {
    return { status: 'Permission was not granted. Choose Allow and check to try again.', visible: false };
  }
  if (result.permissionState !== 'granted') {
    return { status: 'Permission has not been requested.', visible: false };
  }
  return {
    status: result.reported
      ? 'Results sent to MyPass → Settings → Browser.'
      : 'Results are shown below. Open and connect MyPass, then check again to send them to the app.',
    visible: true,
    passwordSaving: result.passwordSavingEnabled === true ? 'On — may show duplicate save prompts'
      : result.passwordSavingEnabled === false ? 'Off — passkeys and autofill still need a manual check' : 'Could not be read',
    extensions: result.extensionScanState !== 'complete' ? 'Could not be checked'
      : result.extensions.length ? result.extensions.map((item) => item.name).join(', ') : 'No known competing extensions detected'
  };
}

if (typeof document !== 'undefined') {
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
      checkButton.textContent = result && result.permissionState === 'granted' ? 'Check again' : 'Allow and check';
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
      checkButton.textContent = 'Allow and check';
    });
  });
  document.getElementById('open-settings-btn').addEventListener('click', () => {
    chrome.runtime.sendMessage({ type: 'openBrowserSettings' }, (result) => {
      void chrome.runtime.lastError;
      if (!settingsOpenConfirmed(result)) {
        status.textContent = 'Could not open settings. Open MyPass and connect this browser, then choose Settings → Browser.';
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
