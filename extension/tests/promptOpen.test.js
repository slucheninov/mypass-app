const assert = require('node:assert');

// A fake chrome.* with enough surface for the prompt store and opener: storage.local/session,
// action (popup + badge), windows, tabs.get/sendMessage/onRemoved, notifications, native messaging.
const local = {};
let session = {};
const log = { popup: [], badge: [], windowsCreated: [], windowsUpdated: [], tabsUpdated: [], notifications: [], native: [], tabMessages: [] };
const behavior = {
  openPopup: 'ok',               // 'ok' | 'reject' | 'missing'
  tab: { id: 7, active: true, windowId: 3 },
  windowFocused: true,
  tabReply: (msg, opts) => ({ type: 'ok', url: 'https://login.example.com/signin' }),
  nativeReply: (msg) => ({ type: 'credentialList', credentials: [] }),
  windowsCreateFails: false,
  existingWindows: new Set()
};
let tabRemovedListener = null;
let windowRemovedListener = null;
let nextWindowId = 100;

function area(store) {
  return {
    get: (keys, cb) => {
      const out = {};
      for (const k of (Array.isArray(keys) ? keys : [keys])) if (store[k] !== undefined) out[k] = store[k];
      cb(out);
    },
    set: (items, cb) => { Object.assign(store, items); if (cb) cb(); },
    remove: (keys, cb) => { for (const k of (Array.isArray(keys) ? keys : [keys])) delete store[k]; if (cb) cb(); }
  };
}
const sessionArea = {
  get: (keys, cb) => area(session).get(keys, cb),
  set: (items, cb) => area(session).set(items, cb),
  remove: (keys, cb) => area(session).remove(keys, cb)
};

global.chrome = {
  storage: { local: area(local), session: sessionArea },
  runtime: {
    id: 'ext',
    lastError: null,
    getURL: (p) => `chrome-extension://ext/${p}`,
    onMessage: { addListener: () => {} },
    sendNativeMessage: (_host, message, cb) => {
      log.native.push(message);
      cb(behavior.nativeReply(message));
    }
  },
  action: {
    setBadgeText: ({ text }) => { log.badge.push(text); badgeText = text; return Promise.resolve(); },
    getBadgeText: () => Promise.resolve(badgeText),
    openPopup: (opts) => {
      log.popup.push(opts || null);
      return behavior.openPopup === 'ok' ? Promise.resolve() : Promise.reject(new Error('No active browser window'));
    }
  },
  tabs: {
    query: async ({ windowId }) => [{ id: windowId + 1000 }],
    update: async (id, props) => { log.tabsUpdated.push({ id, props }); },
    get: async (id) => {
      if (!behavior.tab || behavior.tab.id !== id) throw new Error('No tab');
      return behavior.tab;
    },
    sendMessage: (tabId, msg, opts, cb) => {
      log.tabMessages.push({ tabId, msg, opts });
      const reply = behavior.tabReply(msg, opts);
      if (reply === null) { chrome.runtime.lastError = { message: 'Could not establish connection' }; cb(undefined); chrome.runtime.lastError = null; return; }
      cb(reply);
    },
    onRemoved: { addListener: (fn) => { tabRemovedListener = fn; } }
  },
  windows: {
    get: async () => ({ focused: behavior.windowFocused }),
    create: async (opts) => {
      if (behavior.windowsCreateFails) throw new Error('no windows');
      log.windowsCreated.push(opts);
      const id = nextWindowId += 1;
      behavior.existingWindows.add(id);
      return { id };
    },
    update: async (id, props) => {
      if (!behavior.existingWindows.has(id)) throw new Error('No window');
      log.windowsUpdated.push({ id, props });
    },
    onRemoved: { addListener: (fn) => { windowRemovedListener = fn; } }
  },
  notifications: { create: (id, opts) => { log.notifications.push({ id, opts }); return Promise.resolve(); } }
};
let badgeText = '';

const bg = require('../background.js');

function contentSender(overrides = {}) {
  return {
    id: 'ext',
    url: 'https://login.example.com/signin',
    frameId: 0,
    documentId: 'DOC-1',
    tab: { id: 7, windowId: 3 },
    ...overrides
  };
}
const uiSender = { id: 'ext', url: 'chrome-extension://ext/popup.html' };
const promptWindowSender = { id: 'ext', url: 'chrome-extension://ext/prompt.html?prompt=abc' };

function resetLogs() {
  for (const key of Object.keys(log)) log[key] = [];
  badgeText = '';
}

function deepStrings(value, out = []) {
  if (typeof value === 'string') out.push(value);
  else if (value && typeof value === 'object') Object.values(value).forEach((v) => deepStrings(v, out));
  return out;
}

const PASSWORD = 'hunter2-SECRET';
const SEED = 'JBSWY3DPEHPK3PXP';
const loginRequest = { type: 'requestPrompt', kind: 'saveLogin', url: 'https://login.example.com/signin', username: 'alice', password: PASSWORD };
const totpRequest = { type: 'requestPrompt', kind: 'saveTotp', url: 'https://login.example.com/signin', otpauthOrSecret: SEED };

function dispatch(message, sender) {
  return new Promise((resolve) => {
    const kept = bg.handleRuntimeMessage(message, sender, resolve);
    if (!kept) resolve(undefined);
  });
}

(async () => {
  // --- create: sender checks ---
  assert.deepStrictEqual(await dispatch(loginRequest, {}), { type: 'error', code: 'prompt_unauthorized' }, 'no tab: not a content script');
  assert.deepStrictEqual(await dispatch(loginRequest, contentSender({ url: 'chrome-extension://ext/popup.html' })),
    { type: 'error', code: 'prompt_unauthorized' }, 'an extension page is not a content script');
  assert.strictEqual((await dispatch({ ...loginRequest, url: 'https://evil.example.net/' }, contentSender())).code, 'prompt_origin_mismatch',
    'the message url must be the sender page');
  assert.strictEqual((await dispatch(loginRequest, contentSender({ frameId: 3 }))).code, 'prompt_rejected', 'sub-frame refused');
  assert.strictEqual((await dispatch(loginRequest, contentSender({ documentId: undefined }))).code, 'prompt_rejected', 'no documentId refused');
  assert.strictEqual((await dispatch({ ...loginRequest, kind: 'nope' }, contentSender())).code, 'prompt_rejected');
  assert.strictEqual((await bg.readPrompts()).length, 0, 'refusals create nothing');

  // --- offerSave* false creates nothing and opens nothing ---
  local.mypassAutofillSettings = { offerSaveLogin: false };
  resetLogs();
  assert.deepStrictEqual(await dispatch(loginRequest, contentSender()), { type: 'ok', created: false });
  assert.strictEqual((await bg.readPrompts()).length, 0);
  assert.strictEqual(log.popup.length + log.windowsCreated.length + log.badge.length, 0, 'switch off: no UI of any kind');
  local.mypassAutofillSettings = { offerSaveTotp: false };
  assert.deepStrictEqual(await dispatch(totpRequest, contentSender()), { type: 'ok', created: false });
  assert.strictEqual((await dispatch(loginRequest, contentSender())).created, true, 'offerSaveLogin still defaults to on');
  delete local.mypassAutofillSettings;
  session = {};

  // --- opener order: popup first ---
  resetLogs();
  assert.deepStrictEqual(await dispatch(loginRequest, contentSender()), { type: 'ok', created: true });
  assert.deepStrictEqual(log.popup, [{ windowId: 3 }], 'action popup is opened for the originating window');
  assert.strictEqual(log.windowsCreated.length, 0, 'no window when the popup opened');
  let stored = await bg.readPrompts();
  assert.strictEqual(stored.length, 1);
  assert.strictEqual(stored[0].documentId, 'DOC-1');
  assert.strictEqual(stored[0].expiresAt - stored[0].createdAt, 120000);

  // --- toolbarless: openPopup rejects -> exactly one window, reused the second time ---
  session = {};
  behavior.openPopup = 'reject';
  resetLogs();
  await dispatch(loginRequest, contentSender());
  assert.strictEqual(log.windowsCreated.length, 1, 'fallback window opened');
  assert.ok(log.windowsCreated[0].url.startsWith('chrome-extension://ext/prompt.html?prompt='), 'standalone prompt.html, never popup.html');
  assert.strictEqual(log.windowsCreated[0].type, 'popup');
  await dispatch({ type: 'openMyPassSettings' }, contentSender());
  assert.strictEqual(log.windowsCreated.length, 1, 'a second prompt reuses the window');
  assert.strictEqual(log.windowsUpdated.length, 1, 'the existing window is focused');
  const settingsRecord = (await bg.readPrompts()).find((record) => record.kind === 'settings');
  assert.deepStrictEqual(log.tabsUpdated, [{
    id: session[bg.PROMPT_WINDOW_KEY] + 1000,
    props: { url: 'chrome-extension://ext/prompt.html?prompt=' + encodeURIComponent(settingsRecord.id) }
  }], 'explicit reuse rebinds the window to the requested record');
  assert.strictEqual((await bg.readPrompts()).length, 2, 'different kinds queue');
  assert.deepStrictEqual((await bg.readPrompts()).filter((r) => r.surface === 'window').map((r) => r.id), [settingsRecord.id], 'only the displayed record belongs to the window');

  // closing the prompt window dismisses what it showed
  const windowId = session[bg.PROMPT_WINDOW_KEY];
  assert.strictEqual(typeof windowId, 'number');
  behavior.existingWindows.delete(windowId);
  windowRemovedListener(windowId);
  await new Promise((r) => setTimeout(r, 20));
  assert.deepStrictEqual((await bg.readPrompts()).map((r) => r.kind), ['saveLogin'], 'closing settings preserves the previously displayed login');
  assert.strictEqual(session[bg.PROMPT_WINDOW_KEY], undefined);

  // --- window creation fails -> badge ---
  session = {};
  behavior.windowsCreateFails = true;
  resetLogs();
  await dispatch(loginRequest, contentSender());
  assert.ok(log.badge.includes('!'), 'badge as the last resort');
  assert.strictEqual((await bg.readPrompts()).length, 1, 'the record survives for the badge click');
  assert.strictEqual(log.notifications.length, 0, 'a save prompt raises no notification');
  behavior.windowsCreateFails = false;

  // --- inactive tab / unfocused window: never steal focus, badge only ---
  session = {};
  behavior.openPopup = 'ok';
  behavior.tab = { id: 7, active: false, windowId: 3 };
  resetLogs();
  await dispatch(loginRequest, contentSender());
  assert.strictEqual(log.popup.length, 0, 'a background tab does not open a popup');
  assert.strictEqual(log.windowsCreated.length, 0);
  assert.ok(log.badge.includes('!'));
  behavior.tab = { id: 7, active: true, windowId: 3 };
  behavior.windowFocused = false;
  session = {};
  resetLogs();
  await dispatch(loginRequest, contentSender());
  assert.strictEqual(log.popup.length, 0, 'an unfocused window does not get a popup');
  behavior.windowFocused = true;


  // --- passive 2FA (page-load / mutation sweep, no gesture): badge only, never browser UI ---
  const SEED2 = 'KRSXG5CTMVRXEZLU';
  for (const popupBehavior of ['ok', 'reject']) {
    session = {};
    behavior.openPopup = popupBehavior;
    resetLogs();
    assert.deepStrictEqual(await dispatch(totpRequest, contentSender()), { type: 'ok', created: true });
    assert.strictEqual(log.popup.length, 0, 'a passive 2FA sweep never calls action.openPopup (' + popupBehavior + ')');
    assert.strictEqual(log.windowsCreated.length, 0, 'nor windows.create');
    assert.strictEqual(log.windowsUpdated.length, 0, 'nor focuses an existing window');
    assert.strictEqual(log.tabsUpdated.length, 0, 'nor rebinds an existing prompt window');
    assert.ok(log.badge.includes('!'), 'the badge is the only signal');
    assert.strictEqual((await bg.readPrompts()).length, 1, 'the record waits for an explicit open');
  }
  // The pending card is there when the user opens the extension themselves.
  behavior.nativeReply = () => ({ type: 'credentialList', credentials: [] });
  const waiting = await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender);
  assert.strictEqual(waiting.type, 'prompt');
  assert.strictEqual(waiting.prompt.kind, 'saveTotp', 'explicit extension open shows the pending 2FA card');
  // A second sweep result for the SAME document does not replace the snapshot that may be on screen...
  assert.deepStrictEqual(await dispatch({ ...totpRequest, otpauthOrSecret: SEED2 }, contentSender()), { type: 'ok', created: false });
  let pendingTotp = (await bg.readPrompts()).filter((r) => r.kind === 'saveTotp');
  assert.strictEqual(pendingTotp.length, 1);
  assert.strictEqual(pendingTotp[0].payload.otpauthOrSecret, SEED, 'the first snapshot stays');
  assert.strictEqual(pendingTotp[0].id, waiting.prompt.id, 'and keeps its id');
  // ...while a new document (the page navigated) starts afresh.
  await dispatch({ ...totpRequest, otpauthOrSecret: SEED2 }, contentSender({ documentId: 'DOC-2' }));
  pendingTotp = (await bg.readPrompts()).filter((r) => r.kind === 'saveTotp');
  assert.strictEqual(pendingTotp.length, 1);
  assert.strictEqual(pendingTotp[0].payload.otpauthOrSecret, SEED2, 'a new document replaces the stale snapshot');
  behavior.openPopup = 'ok';

  // --- per-tab throttle on automatic opening ---
  const realClock = Date.now;
  const T0 = realClock();
  Date.now = () => T0;
  session = {};
  resetLogs();
  await dispatch(loginRequest, contentSender());
  assert.strictEqual(log.popup.length, 1, 'the first trusted login prompt opens the popup');
  const second = await dispatch({ ...loginRequest, password: 'second-SECRET' }, contentSender({ documentId: 'DOC-1B' }));
  assert.strictEqual(second.created, true);
  assert.strictEqual(log.popup.length, 1, 'a repeated prompt within the window does not open again');
  assert.strictEqual(log.windowsCreated.length, 0);
  assert.ok(log.badge.includes('!'), 'it only moves the badge');
  assert.strictEqual((await bg.readPrompts()).filter((r) => r.kind === 'saveLogin').length, 1, 'the newest login replaces the older record');
  behavior.openPopup = 'reject';
  resetLogs();
  for (let i = 0; i < 5; i += 1) await dispatch({ ...loginRequest, password: 'again-' + i }, contentSender({ documentId: 'DOC-1C' }));
  assert.strictEqual(log.windowsCreated.length + log.windowsUpdated.length, 0, 'no toolbarless window churn either');
  assert.strictEqual(log.popup.length, 0, 'five more requests: no popup attempt at all');
  behavior.openPopup = 'ok';
  // another tab has its own allowance; settings (a click on the picker row) is never throttled
  resetLogs();
  behavior.tab = { id: 9, active: true, windowId: 3 };
  await dispatch(loginRequest, contentSender({ tab: { id: 9, windowId: 3 }, documentId: 'DOC-9' }));
  assert.strictEqual(log.popup.length, 1, 'tab 9 has its own allowance while tab 7 is throttled');
  behavior.tab = { id: 7, active: true, windowId: 3 };
  resetLogs();
  await dispatch({ type: 'openMyPassSettings' }, contentSender());
  assert.strictEqual(log.popup.length, 1, 'an explicit settings click always opens');
  // edge of the window
  Date.now = () => T0 + bg.PROMPT_AUTO_OPEN_MS - 1;
  resetLogs();
  await dispatch({ ...loginRequest, password: 'edge-1' }, contentSender());
  assert.strictEqual(log.popup.length, 0, 'still throttled 1 ms before the window ends');
  Date.now = () => T0 + bg.PROMPT_AUTO_OPEN_MS;
  await dispatch({ ...loginRequest, password: 'edge-2' }, contentSender());
  assert.strictEqual(log.popup.length, 1, 'an expired throttle lets a trusted request open again');
  // closing the tab forgets its allowance
  tabRemovedListener(7);
  await new Promise((r) => setTimeout(r, 20));
  resetLogs();
  await dispatch({ ...loginRequest, password: 'after-close' }, contentSender());
  assert.strictEqual(log.popup.length, 1, 'a closed tab leaves no throttle behind');
  Date.now = realClock;

  // --- getPrompt: sender gate, redaction, document binding ---
  session = {};
  resetLogs();
  await dispatch(loginRequest, contentSender());
  await dispatch(totpRequest, contentSender());
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, contentSender())).code, 'prompt_unauthorized', 'a web page cannot read prompts');
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, { id: 'other', url: 'chrome-extension://ext/popup.html' })).code, 'prompt_unauthorized');
  behavior.nativeReply = () => ({ type: 'credentialList', credentials: [{ id: 'E1', title: 'Example', username: 'alice', hasTotp: false, password: PASSWORD }] });
  const responses = [];
  const first = await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender);
  responses.push(first);
  assert.strictEqual(first.type, 'prompt');
  assert.strictEqual(first.prompt.kind, 'saveTotp', 'newest first');
  assert.strictEqual(first.prompt.state, 'ready');
  assert.deepStrictEqual(first.prompt.accounts, [{ id: 'E1', title: 'Example', username: 'alice', hasTotp: false }],
    'only the listed fields of an account are passed on');
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 99 }, uiSender)).type, 'noPrompt', 'another tab has none');
  responses.push(await dispatch({ type: 'getPrompt' }, promptWindowSender));
  const text = deepStrings(responses).join('\n');
  assert.ok(!text.includes(PASSWORD), 'no password in any response to the UI');
  assert.ok(!text.includes(SEED), 'no seed in any response to the UI');
  assert.ok(!text.includes('DOC-1'), 'no documentId in any response to the UI');

  // navigated document: the verify message fails -> the prompt is dropped, never retargeted
  behavior.tabReply = () => null;
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'noPrompt');
  assert.strictEqual((await bg.readPrompts()).length, 0, 'stale prompts are removed');
  assert.deepStrictEqual(log.tabMessages[0].opts, { frameId: 0, documentId: 'DOC-1' }, 'verification targets the recorded frame and document');
  behavior.tabReply = () => ({ type: 'ok', url: 'https://login.example.com/signin' });

  // same document, but it moved to another host via history.pushState
  session = {};
  await dispatch(loginRequest, contentSender());
  behavior.tabReply = () => ({ type: 'ok', url: 'https://other.example.org/' });
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'noPrompt', 'host changed in-document');
  behavior.tabReply = () => ({ type: 'ok', url: 'https://login.example.com/signin' });

  // --- consume: single use, validated, mapped to the native message ---
  session = {};
  behavior.nativeReply = (msg) => (msg.type === 'proposeSaveLogin' ? { type: 'saveProposalAccepted' } : { type: 'credentialList', credentials: [] });
  await dispatch(loginRequest, contentSender());
  const promptId = (await bg.readPrompts())[0].id;
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId, action: 'save' }, contentSender())).code, 'prompt_unauthorized');
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId, action: 'weird' }, uiSender)).code, 'prompt_bad_request');
  resetLogs();
  const saved = await dispatch({ type: 'consumePrompt', promptId, action: 'save' }, uiSender);
  assert.deepStrictEqual(saved, { type: 'saveProposalAccepted' });
  const proposal = log.native.find((m) => m.type === 'proposeSaveLogin');
  assert.deepStrictEqual(
    { url: proposal.url, username: proposal.username, password: proposal.password },
    { url: 'https://login.example.com/signin', username: 'alice', password: PASSWORD },
    'the background, not the popup, supplies the secret'
  );
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId, action: 'save' }, uiSender)).code, 'prompt_gone', 'second consume finds nothing');
  assert.strictEqual(log.native.filter((m) => m.type === 'proposeSaveLogin').length, 1, 'saved exactly once');

  // dismiss sends nothing to the app
  await dispatch(loginRequest, contentSender());
  resetLogs();
  const dismissId = (await bg.readPrompts())[0].id;
  assert.deepStrictEqual(await dispatch({ type: 'consumePrompt', promptId: dismissId, action: 'dismiss' }, uiSender), { type: 'ok' });
  assert.strictEqual(log.native.length, 0);
  assert.strictEqual((await bg.readPrompts()).length, 0);

  // page changed between display and Save: nothing is sent
  await dispatch(loginRequest, contentSender());
  behavior.tabReply = () => null;
  resetLogs();
  const staleId = (await bg.readPrompts())[0].id;
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: staleId, action: 'save' }, uiSender)).code, 'prompt_page_changed');
  assert.strictEqual(log.native.length, 0, 'a changed document never receives a save');
  behavior.tabReply = () => ({ type: 'ok', url: 'https://login.example.com/signin' });


  // --- a classic form POST replaces the document: saveLogin survives a SAME-ORIGIN navigation ---
  const navigated = (url) => (msg, opts) => (opts && opts.documentId ? null : { type: 'ok', url });
  async function pendingLogin() {
    session = {};
    behavior.tab = { id: 7, active: true, windowId: 3 };
    behavior.nativeReply = (msg) => (msg.type === 'proposeSaveLogin' ? { type: 'saveProposalAccepted' } : { type: 'credentialList', credentials: [] });
    await dispatch(loginRequest, contentSender());
    return (await bg.readPrompts())[0].id;
  }
  let navId = await pendingLogin();
  behavior.tabReply = navigated('https://login.example.com/account');
  const shown = await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender);
  assert.strictEqual(shown.type, 'prompt', 'the prompt is still shown after the submit navigated the document');
  assert.strictEqual(shown.prompt.kind, 'saveLogin');
  assert.ok(log.tabMessages.some((m) => m.opts && m.opts.documentId === 'DOC-1'), 'the strict document check ran first');
  assert.ok(log.tabMessages.some((m) => m.opts && m.opts.frameId === 0 && m.opts.documentId === undefined), 'then the live top frame was asked');
  resetLogs();
  assert.deepStrictEqual(await dispatch({ type: 'consumePrompt', promptId: navId, action: 'save' }, uiSender), { type: 'saveProposalAccepted' });
  assert.strictEqual(log.native.find((m) => m.type === 'proposeSaveLogin').url, 'https://login.example.com/signin',
    'the native save names the page that was captured, not the page the site redirected to');

  // refusals, at display time and at Save time
  const refusals = [
    ['cross-origin navigation', navigated('https://evil.example.net/'), null],
    ['scheme downgrade', navigated('http://login.example.com/signin'), null],
    ['different port', navigated('https://login.example.com:8443/signin'), null],
    ['sibling subdomain', navigated('https://www.login.example.com/signin'), null],
    ['non-web page', navigated('chrome-error://chromewebdata/'), null],
    ['content script says same origin but tab.url disagrees', navigated('https://login.example.com/account'), 'https://evil.example.net/']
  ];
  for (const [label, reply, tabUrl] of refusals) {
    for (const stage of ['getPrompt', 'consume']) {
      navId = await pendingLogin();
      behavior.tabReply = reply;
      if (tabUrl) behavior.tab = { id: 7, active: true, windowId: 3, url: tabUrl };
      resetLogs();
      if (stage === 'getPrompt') {
        assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'noPrompt', label + ': not shown');
        assert.strictEqual((await bg.readPrompts()).length, 0, label + ': dropped');
      } else {
        assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: navId, action: 'save' }, uiSender)).code, 'prompt_page_changed', label + ': not saved');
      }
      assert.strictEqual(log.native.length, 0, label + ' (' + stage + '): nothing reaches the app');
    }
  }
  // tab.url, when Chrome exposes it, is accepted if it agrees
  navId = await pendingLogin();
  behavior.tabReply = navigated('https://login.example.com/account');
  behavior.tab = { id: 7, active: true, windowId: 3, url: 'https://login.example.com/account' };
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'prompt');
  // closed tab
  navId = await pendingLogin();
  behavior.tabReply = navigated('https://login.example.com/account');
  behavior.tab = null;
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'noPrompt', 'closed tab');
  navId = await pendingLogin();
  behavior.tabReply = navigated('https://login.example.com/account');
  behavior.tab = null;
  resetLogs();
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: navId, action: 'save' }, uiSender)).code, 'prompt_page_changed', 'closed tab at Save');
  assert.strictEqual(log.native.length, 0);
  // TTL still applies to a navigated-but-same-origin prompt
  navId = await pendingLogin();
  behavior.tabReply = navigated('https://login.example.com/account');
  const ttlClock = Date.now;
  Date.now = () => ttlClock() + 120001;
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'noPrompt', 'expired');
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: navId, action: 'save' }, uiSender)).code, 'prompt_gone');
  Date.now = ttlClock;
  // the exception is saveLogin only: a 2FA seed keeps the strict document binding
  session = {};
  behavior.tab = { id: 7, active: true, windowId: 3 };
  await dispatch(totpRequest, contentSender());
  behavior.tabReply = navigated('https://login.example.com/signin');
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'noPrompt', 'saveTotp does not survive navigation, even on the same origin');
  behavior.tabReply = () => ({ type: 'ok', url: 'https://login.example.com/signin' });
  behavior.tab = { id: 7, active: true, windowId: 3 };

  // locked / unpaired app keeps the record until it expires
  session = {};
  await dispatch(loginRequest, contentSender());
  behavior.nativeReply = () => ({ type: 'locked' });
  const lockedId = (await bg.readPrompts())[0].id;
  assert.deepStrictEqual(await dispatch({ type: 'consumePrompt', promptId: lockedId, action: 'save' }, uiSender), { type: 'locked' });
  assert.strictEqual((await bg.readPrompts()).length, 1, 'locked: the prompt is kept for a retry');
  behavior.nativeReply = () => ({ type: 'pairingRequired' });
  const retryId = (await bg.readPrompts())[0].id;
  assert.deepStrictEqual(await dispatch({ type: 'consumePrompt', promptId: retryId, action: 'save' }, uiSender), { type: 'pairingRequired' });
  assert.strictEqual((await bg.readPrompts()).length, 1, 'pairing required: still kept');

  // an error is reported without echoing arbitrary native fields
  behavior.nativeReply = () => ({ type: 'error', message: 'boom', password: PASSWORD });
  const errId = (await bg.readPrompts())[0].id;
  const errResult = await dispatch({ type: 'consumePrompt', promptId: errId, action: 'save' }, uiSender);
  assert.deepStrictEqual(errResult, { type: 'error', message: 'boom' });
  assert.strictEqual((await bg.readPrompts()).length, 1, 'an app error (e.g. not running) keeps the prompt for a retry');
  behavior.nativeReply = () => null;
  const nullId = (await bg.readPrompts())[0].id;
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: nullId, action: 'save' }, uiSender)).type, 'error');
  assert.strictEqual((await bg.readPrompts()).length, 1, 'no answer at all keeps it too');

  // --- 2FA save: account choice must come from the live list ---
  session = {};
  behavior.nativeReply = (msg) => (msg.type === 'getCredentials'
    ? { type: 'credentialList', credentials: [{ id: 'A', title: 'A', username: 'a' }, { id: 'B', title: 'B', username: 'b' }] }
    : { type: 'saveProposalAccepted' });
  await dispatch(totpRequest, contentSender());
  let totpId = (await bg.readPrompts())[0].id;
  resetLogs();
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: totpId, action: 'save' }, uiSender)).code, 'prompt_choice_required',
    'two candidate accounts need an explicit choice');
  assert.strictEqual((await bg.readPrompts()).length, 1, 'kept so the user can choose');
  totpId = (await bg.readPrompts())[0].id;
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: totpId, action: 'save', entryId: 'Z' }, uiSender)).code, 'prompt_bad_choice',
    'an id the app did not list is refused');
  assert.strictEqual(log.native.filter((m) => m.type === 'proposeSaveTotp').length, 0);
  totpId = (await bg.readPrompts())[0].id;
  assert.deepStrictEqual(await dispatch({ type: 'consumePrompt', promptId: totpId, action: 'save', entryId: 'B' }, uiSender), { type: 'saveProposalAccepted' });
  const totpProposal = log.native.find((m) => m.type === 'proposeSaveTotp');
  assert.strictEqual(totpProposal.entryId, 'B');
  assert.strictEqual(totpProposal.otpauthOrSecret, SEED);

  // zero accounts: a new entry (no entryId), one account: that account
  session = {};
  behavior.nativeReply = (msg) => (msg.type === 'getCredentials' ? { type: 'credentialList', credentials: [] } : { type: 'saveProposalAccepted' });
  await dispatch(totpRequest, contentSender());
  resetLogs();
  await dispatch({ type: 'consumePrompt', promptId: (await bg.readPrompts())[0].id, action: 'save' }, uiSender);
  assert.strictEqual(Object.prototype.hasOwnProperty.call(log.native.find((m) => m.type === 'proposeSaveTotp'), 'entryId'), false, 'new entry');
  behavior.nativeReply = (msg) => (msg.type === 'getCredentials' ? { type: 'credentialList', credentials: [{ id: 'ONLY', title: 'x', username: 'y' }] } : { type: 'saveProposalAccepted' });
  await dispatch(totpRequest, contentSender());
  resetLogs();
  await dispatch({ type: 'consumePrompt', promptId: (await bg.readPrompts())[0].id, action: 'save' }, uiSender);
  assert.strictEqual(log.native.find((m) => m.type === 'proposeSaveTotp').entryId, 'ONLY');

  // --- legacy direct save proposals from a content script are rejected ---
  assert.strictEqual((await dispatch({ type: 'proposeSaveLogin', url: 'https://login.example.com/', username: 'a', password: 'b' }, contentSender())).code, 'prompt_unauthorized');
  assert.strictEqual((await dispatch({ type: 'proposeSaveTotp', url: 'https://login.example.com/', otpauthOrSecret: SEED }, contentSender())).code, 'prompt_unauthorized');

  // --- tab closed drops its prompts ---
  session = {};
  await dispatch(loginRequest, contentSender());
  await dispatch(loginRequest, contentSender({ tab: { id: 9, windowId: 3 }, documentId: 'DOC-9' }));
  assert.strictEqual((await bg.readPrompts()).length, 2);
  tabRemovedListener(7);
  await new Promise((r) => setTimeout(r, 20));
  assert.deepStrictEqual((await bg.readPrompts()).map((r) => r.tabId), [9], 'closing tab 7 only drops tab 7');

  // --- expiry ---
  const realNow = Date.now;
  session = {};
  await dispatch(loginRequest, contentSender());
  const expiryId = (await bg.readPrompts())[0].id;
  Date.now = () => realNow() + 120001;
  assert.strictEqual((await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender)).type, 'noPrompt', 'expired prompts are not shown');
  assert.strictEqual((await dispatch({ type: 'consumePrompt', promptId: expiryId, action: 'save' }, uiSender)).code, 'prompt_gone', 'or consumed');
  Date.now = realNow;

  // --- settings prompt: any frame, no page data, consume needs no document ---
  session = {};
  resetLogs();
  const settingsResult = await dispatch({ type: 'openMyPassSettings' }, contentSender({ frameId: 2 }));
  assert.strictEqual(settingsResult.created, true);
  assert.strictEqual(log.popup.length, 1);
  const settingsView = await dispatch({ type: 'getPrompt', tabId: 7 }, uiSender);
  assert.strictEqual(settingsView.prompt.kind, 'settings');
  assert.strictEqual(log.tabMessages.length, 0, 'settings needs no document round trip');

  // --- passkey path through the same opener keeps its notification gate ---
  behavior.openPopup = 'reject';
  resetLogs();
  local.mypassNotificationSettings = { passkeyConfirmation: true };
  await bg.openPasskeyPopup();
  await new Promise((r) => setTimeout(r, 20));
  assert.ok(log.badge.includes('KEY'), 'passkey badge preserved');
  assert.strictEqual(log.windowsCreated.length, 0, 'a passkey ceremony never opens the prompt window');
  assert.strictEqual(log.notifications.length, 1, 'notification when the setting is on');
  local.mypassNotificationSettings = { passkeyConfirmation: false };
  resetLogs();
  await bg.openPasskeyPopup();
  await new Promise((r) => setTimeout(r, 20));
  assert.strictEqual(log.notifications.length, 0, 'and none when it is off');

  console.log('promptOpen.test.js: prompt store, opener order, window reuse, consume rules OK');
})().catch((error) => { console.error(error); process.exit(1); });
