const assert = require('node:assert/strict');
const { describeBrowserCheck, settingsOpenConfirmed } = require('../browser-check.js');

assert.equal(settingsOpenConfirmed({ type: 'ok' }), true);
assert.equal(settingsOpenConfirmed({ type: 'acknowledged' }), false);
assert.equal(settingsOpenConfirmed({ type: 'pairingRequired' }), false);
assert.equal(settingsOpenConfirmed(undefined), false);

assert.equal(describeBrowserCheck({ permissionState: 'not-granted' }).visible, false);
assert.match(describeBrowserCheck({ permissionState: 'not-granted' }).status, /has not been requested/);
assert.match(describeBrowserCheck({ permissionState: 'denied' }).status, /was not granted/);
assert.equal(describeBrowserCheck({ permissionState: 'denied' }).visible, false);
assert.match(describeBrowserCheck({ permissionState: 'unavailable' }).status, /could not be read/);
const complete = { permissionState: 'granted', extensionScanState: 'complete', passwordSavingEnabled: false, extensions: [], reported: true };
assert.match(describeBrowserCheck(complete).passwordSaving, /manual check/);
assert.match(describeBrowserCheck(complete).extensions, /No known/);
assert.match(describeBrowserCheck({ ...complete, extensionScanState: 'unavailable' }).extensions, /Could not/);
assert.match(describeBrowserCheck({ ...complete, reported: false }).status, /check again/);
assert.equal(describeBrowserCheck({ ...complete, extensions: [{ name: '1Password' }, { name: 'iCloud Passwords' }] }).extensions, '1Password, iCloud Passwords');

const sent = [];
const localStore = {};
const sessionStore = {};
let granted = false;
let managementFails = false;
let privacyFails = false;
let browserSettingsOpened = 0;
global.chrome = {
  runtime: {
    id: 'extension-id', getURL: (path) => 'chrome-extension://extension-id/' + path,
    onMessage: { addListener() {} },
    sendNativeMessage: (_host, message, callback) => {
      sent.push(message);
      if (message.type === 'openBrowserSettings') browserSettingsOpened++;
      callback({ type: 'ok' });
    }
  },
  storage: {
    local: {
      get: (_keys, cb) => cb(Object.assign({ pairingSecret: 'test-pairing' }, localStore)),
      set: (items, cb) => { Object.assign(localStore, items); if (cb) cb(); },
      remove: (_keys, cb) => cb && cb()
    },
    session: {
      get: (keys, cb) => {
        const result = {};
        const names = Array.isArray(keys) ? keys : [keys];
        names.forEach((name) => { if (sessionStore[name] !== undefined) result[name] = sessionStore[name]; });
        cb(result);
      },
      set: (items, cb) => { Object.assign(sessionStore, items); if (cb) cb(); },
      remove: (keys, cb) => {
        const names = Array.isArray(keys) ? keys : [keys];
        names.forEach((name) => { delete sessionStore[name]; });
        if (cb) cb();
      }
    }
  },
  permissions: { contains: async () => granted },
  privacy: { services: { passwordSavingEnabled: { get: async () => { if (privacyFails) throw Error('denied'); return { value: true }; } } } },
  management: { getAll: async () => {
    if (managementFails) throw Error('denied');
    return [{ id: 'aeblfdkhhhdcdjpifhhbdiojplfjncoa', name: '1Password', enabled: true },
      { id: 'pejdijmoenmkgeppbflobdenhhabjlaj', name: 'iCloud Passwords', enabled: true },
      { id: 'other', name: 'Unrelated', enabled: true }];
  } }
};
const { getBrowserManagerDiagnostics, reportBrowserDiagnostics, handleRuntimeMessage, noteFilledCredential } = require('../background.js');
(async () => {
  let result = await getBrowserManagerDiagnostics();
  assert.equal(result.permissionState, 'not-granted');
  assert.equal(result.extensionScanState, 'not-checked');
  granted = true;
  result = await getBrowserManagerDiagnostics();
  assert.equal(result.extensionScanState, 'complete');
  assert.deepEqual(result.extensions.map((item) => item.name), ['1Password', 'iCloud Passwords']);
  assert.equal(await reportBrowserDiagnostics({ proxyState: 'active' }, result), true);
  assert.deepEqual(sent.at(-1).report, {
    proxyState: 'active', permissionState: 'granted', passwordSavingEnabled: true,
    extensionScanState: 'complete', enabledManagers: ['1Password', 'iCloud Passwords']
  });
  managementFails = true;
  result = await getBrowserManagerDiagnostics();
  assert.equal(result.permissionState, 'granted', 'failed inspection does not imply missing permission');
  assert.equal(result.extensionScanState, 'unavailable');
  assert.equal(result.passwordSavingEnabled, true, 'partial result preserves privacy observation');
  privacyFails = true;
  result = await getBrowserManagerDiagnostics();
  assert.equal(result.passwordSavingEnabled, null);
  const website = { id: 'extension-id', tab: { id: 1 }, url: 'https://example.com' };
  assert.equal(handleRuntimeMessage({ type: 'openBrowserSettings' }, website, () => {}), false);
  assert.equal(handleRuntimeMessage({ type: 'getBrowserManagerDiagnostics' }, website, () => {}), false);
  assert.equal(browserSettingsOpened, 0);
  await new Promise((resolve) => {
    assert.equal(handleRuntimeMessage({ type: 'openBrowserSettings' }, {
      id: 'extension-id', tab: { id: 2 }, url: 'chrome-extension://extension-id/browser-check.html'
    }, (response) => { assert.equal(response.type, 'ok'); resolve(); }), true);
  });
  assert.equal(browserSettingsOpened, 1);
  const beforeLock = sent.length;
  assert.equal(handleRuntimeMessage({ type: 'lockVault' }, website, () => {}), false);
  assert.equal(handleRuntimeMessage({ type: 'lockVault' }, {
    id: 'extension-id', tab: { id: 2 }, url: 'chrome-extension://extension-id/browser-check.html'
  }, () => {}), false);
  assert.equal(sent.length, beforeLock, 'a page cannot lock the vault');
  await new Promise((resolve) => {
    assert.equal(handleRuntimeMessage({ type: 'lockVault' }, {
      id: 'extension-id', url: 'chrome-extension://extension-id/popup.html'
    }, (response) => {
      assert.equal(response.type, 'ok');
      resolve();
    }), true);
  });
  assert.equal(sent.at(-1).type, 'lockVault');
  assert.equal(handleRuntimeMessage({ type: 'openApp' }, website, () => {}), false);
  await new Promise((resolve) => {
    assert.equal(handleRuntimeMessage({ type: 'openApp' }, {
      id: 'extension-id', url: 'chrome-extension://extension-id/popup.html'
    }, (response) => {
      assert.equal(response.type, 'ok');
      resolve();
    }), true);
  });
  assert.equal(sent.at(-1).type, 'openApp');
  await noteFilledCredential({
    type: 'credential', username: 'alice', password: 's3cret-password', urls: ['https://bank.example', 'https://pay.bank.example']
  }, 'https://bank.example/login');
  const evil = { id: 'extension-id', tab: { id: 3 }, url: 'https://evil.example/pay' };
  const bank = { id: 'extension-id', tab: { id: 4 }, url: 'https://www.bank.example/login' };
  function check(message, sender) {
    return new Promise((resolve) => {
      const handled = handleRuntimeMessage(message, sender, resolve);
      if (handled !== true) resolve({ handled });
    });
  }
  assert.deepEqual(await check({ type: 'checkSecurityUse', action: 'paste', text: 's3cret-password', url: 'https://evil.example/pay' }, evil), { warn: true });
  assert.deepEqual(await check({ type: 'checkSecurityUse', action: 'paste', text: 's3cret-password', url: 'https://www.bank.example/login' }, bank), { warn: false });
  assert.deepEqual(await check({ type: 'checkSecurityUse', action: 'copy', text: 'alice', url: 'https://evil.example/pay' }, evil), { warn: true });
  assert.deepEqual(await check({ type: 'checkSecurityUse', action: 'paste', text: 's3cret-password', url: 'https://other.example' }, evil), { warn: false });
  assert.equal(handleRuntimeMessage({ type: 'checkSecurityUse', action: 'paste', text: 's3cret-password', url: 'https://evil.example' }, { id: 'extension-id' }, () => {}), false);
  localStore.mypassSecuritySettings = { warnPaste: false, warnCopy: true };
  assert.deepEqual(await check({ type: 'checkSecurityUse', action: 'paste', text: 's3cret-password', url: 'https://evil.example/pay' }, evil), { warn: false });
  assert.deepEqual(await check({ type: 'checkSecurityUse', action: 'copy', text: 's3cret-password', url: 'https://evil.example/pay' }, evil), { warn: true });
  await new Promise((resolve) => {
    handleRuntimeMessage({ type: 'lockVault' }, {
      id: 'extension-id', url: 'chrome-extension://extension-id/popup.html'
    }, () => resolve());
  });
  assert.equal(sessionStore.mypassSecurityWatch, undefined, 'locking clears remembered logins');
  console.log('browserSettings.test.js: separate permission/results, authenticated report and extension-page boundary OK');
})().catch((error) => { console.error(error); process.exitCode = 1; });
