const assert = require('node:assert');

// The options page lives in a tab (open_in_tab:true), so its sender carries sender.tab.
// background.js drops passkey-setting messages from any tab sender; this suite pins the one
// narrow exception for app/app.html and proves nothing else widened.
const EXT_ID = 'hjijbabodfldconmhdcdphlbgaajkejn';
const BASE = `chrome-extension://${EXT_ID}/`;
const store = {};
const area = {
  get: (keys, cb) => {
    const out = {};
    for (const k of (Array.isArray(keys) ? keys : [keys])) if (store[k] !== undefined) out[k] = store[k];
    cb(out);
  },
  set: (items, cb) => { Object.assign(store, items); if (cb) cb(); },
  remove: (keys, cb) => { for (const k of (Array.isArray(keys) ? keys : [keys])) delete store[k]; if (cb) cb(); }
};
const nativeSent = [];
global.chrome = {
  storage: { local: area, session: area },
  runtime: {
    id: EXT_ID,
    lastError: null,
    getURL: (p) => BASE + p,
    onMessage: { addListener: () => {} },
    sendNativeMessage: (_h, message, cb) => {
      nativeSent.push(message.type);
      cb(message.type === 'setPasskeyProxyEnabled'
        ? { type: 'passkeyProxyEnabled', enabled: true } : { type: 'ok' });
    }
  },
  alarms: { create() {}, clear() {}, onAlarm: { addListener() {} } }
};

const { handleRuntimeMessage } = require('../background.js');

async function dispatch(message, sender) {
  nativeSent.length = 0;
  let responded = false;
  const kept = handleRuntimeMessage(message, sender, () => { responded = true; });
  if (kept === true) await new Promise((r) => setTimeout(r, 20));
  return { kept, responded, forwarded: nativeSent.slice() };
}

(async () => {
  const options = { id: EXT_ID, tab: { id: 1 }, url: BASE + 'app/app.html#/page/settings' };
  const optionsQuery = { id: EXT_ID, tab: { id: 1 }, url: BASE + 'app/app.html?x=1#/page/settings' };
  const browserCheck = { id: EXT_ID, tab: { id: 2 }, url: BASE + 'browser-check.html' };
  const website = { tab: { id: 3 }, url: 'https://evil.example/app/app.html' };
  const foreignExt = { id: 'otherextensionidotherextensionid', tab: { id: 1 }, url: BASE + 'app/app.html' };
  const lookalike = { id: EXT_ID, tab: { id: 1 }, url: BASE + 'app/app.html.evil' };

  for (const sender of [options, optionsQuery]) {
    let r = await dispatch({ type: 'openApp' }, sender);
    assert.strictEqual(r.responded, true, 'options page may open the app');
    assert.deepStrictEqual(r.forwarded, ['openApp']);
    r = await dispatch({ type: 'setPasskeyProxyEnabled', enabled: true }, sender);
    assert.strictEqual(r.responded, true, 'options page may change the passkey switch');
    assert.ok(r.forwarded.includes('setPasskeyProxyEnabled'));
  }

  for (const type of ['passkeyCreateConfirm', 'passkeyGetApprove', 'lockVault']) {
    const r = await dispatch({ type }, options);
    assert.strictEqual(r.responded, false, `${type} stays rejected from the options page`);
    assert.deepStrictEqual(r.forwarded, [], `${type} must not reach the app from the options page`);
  }

  for (const [name, sender] of [['browser-check', browserCheck], ['website', website], ['foreign extension', foreignExt], ['lookalike url', lookalike]]) {
    for (const message of [{ type: 'setPasskeyProxyEnabled', enabled: true }, { type: 'openApp' }]) {
      const r = await dispatch(message, sender);
      assert.strictEqual(r.responded, false, `${name} sender is rejected for ${message.type}`);
      assert.deepStrictEqual(r.forwarded, [], `${name}: ${message.type} must not be forwarded`);
    }
  }
  console.log('optionsPageGates.test.js passed');
})().catch((e) => { console.error(e); process.exit(1); });
