const assert = require('node:assert');
const card = require('../shared/prompt-card.js');
const autofill = require('../shared/autofill-settings.js');
const { createPromptPage } = require('../prompt.js');

function makeDoc() {
  function make(tag) {
    const node = {
      tag, className: '', textContent: '', hidden: false, disabled: false, checked: false, type: '',
      children: [], listeners: {}, attrs: {},
      get firstChild() { return this.children[0] || null; },
      appendChild(child) { this.children.push(child); return child; },
      removeChild(child) { this.children = this.children.filter((c) => c !== child); },
      replaceChildren() { this.children = []; },
      setAttribute(name, value) { this.attrs[name] = value; },
      addEventListener(name, fn) { (this.listeners[name] = this.listeners[name] || []).push(fn); },
      fire(name) { (this.listeners[name] || []).forEach((fn) => fn({})); }
    };
    return node;
  }
  return { createElement: make, make, getElementById: () => rootEl };
}
const rootEl = makeDoc().make('div');
const doc = makeDoc();
rootEl.children = [];

function walk(node, out = []) {
  out.push(node);
  node.children.forEach((c) => walk(c, out));
  return out;
}
const byText = (root, text) => walk(root).find((n) => n.textContent === text);
function allText(root) { return walk(root).map((n) => n.textContent).join('\n'); }

function makeApi(overrides = {}) {
  const sent = [];
  const stored = {};
  const api = {
    sent, stored, doneCalls: 0, reloadCalls: 0, timers: [],
    send: (message, cb) => { sent.push(message); cb(api.reply ? api.reply(message) : { type: 'ok' }); },
    done: () => { api.doneCalls += 1; },
    reload: () => { api.reloadCalls += 1; },
    settings: card.chromeSettingsStore({
      storage: { local: {
        get: (keys, cb) => cb(stored.settings === undefined ? {} : { [autofill.AUTOFILL_SETTINGS_KEY]: stored.settings }),
        set: (items) => { stored.settings = items[autofill.AUTOFILL_SETTINGS_KEY]; }
      } }
    }, autofill),
    setTimer: (fn, ms) => { api.timers.push({ fn, ms }); return api.timers.length; },
    clearTimer: () => {},
    now: () => 1000,
    ...overrides
  };
  return api;
}

const loginView = { id: 'p1', kind: 'saveLogin', origin: 'https://login.example.com', username: 'alice', passwordDetected: true, expiresAt: 61000 };

// --- save login: no secret on screen, Save sends only the prompt id and action ---
{
  const api = makeApi();
  const ctl = card.render(doc, rootEl, loginView, api);
  const text = allText(rootEl);
  assert.ok(text.includes('Save password?'));
  assert.strictEqual(walk(rootEl).find((node) => node.tag === 'dl').children[1].textContent, loginView.origin);
  assert.ok(text.includes('alice'));
  assert.ok(!text.includes('hunter2'), 'the card never holds the password');
  assert.strictEqual(ctl.id, 'p1');
  byText(rootEl, 'Save').fire('click');
  assert.deepStrictEqual(api.sent, [{ type: 'consumePrompt', promptId: 'p1', action: 'save' }], 'only id + action leave the card');
  assert.strictEqual(api.doneCalls, 1, 'a successful save finishes the prompt');
  assert.deepStrictEqual(api.timers.map((t) => t.ms), [60000], 'expiry timer = time left');

  byText(rootEl, 'Not now').fire('click');
  assert.strictEqual(api.sent[1].action, 'dismiss');
  assert.strictEqual(api.doneCalls, 2);
}

// --- the switch is the existing offerSaveLogin setting, not a new key ---
{
  const api = makeApi();
  api.stored.settings = { offerSaveLogin: true, showCards: false };
  card.render(doc, rootEl, loginView, api);
  const toggle = walk(rootEl).find((n) => n.tag === 'input' && n.type === 'checkbox');
  assert.strictEqual(toggle.checked, true);
  toggle.checked = false;
  toggle.fire('change');
  assert.strictEqual(api.stored.settings.offerSaveLogin, false, 'writes the existing setting');
  assert.strictEqual(api.stored.settings.showCards, false, 'and keeps the other switches as they were');
  assert.strictEqual(api.stored.settings.showFieldIcon, true, 'defaults are filled in, not lost');
}

// --- locked app keeps the card and reports it; a double click sends once ---
{
  const api = makeApi({});
  api.reply = () => ({ type: 'locked' });
  card.render(doc, rootEl, loginView, api);
  byText(rootEl, 'Save').fire('click');
  assert.strictEqual(api.doneCalls, 0, 'locked: not finished');
  assert.ok(allText(rootEl).includes('MyPass is locked'), 'says why');
  assert.ok(byText(rootEl, 'Save'), 'Save is still there for the retry');

  let pending;
  const slow = makeApi({ send: (m, cb) => { slow.sent.push(m); pending = cb; } });
  card.render(doc, rootEl, loginView, slow);
  const save = byText(rootEl, 'Save');
  save.fire('click');
  save.fire('click');
  assert.strictEqual(slow.sent.length, 1, 'a second click while busy is ignored');
  assert.strictEqual(save.disabled, true);
  pending({ type: 'saveProposalAccepted' });
  assert.strictEqual(slow.doneCalls, 1);
}

// --- gone / page changed: only a way out ---
{
  const api = makeApi();
  api.reply = () => ({ type: 'error', code: 'prompt_page_changed' });
  card.render(doc, rootEl, loginView, api);
  byText(rootEl, 'Save').fire('click');
  assert.ok(allText(rootEl).includes('The page changed'));
  assert.strictEqual(walk(rootEl).find((n) => n.textContent === 'Save').hidden, true, 'Save hidden');
  assert.ok(byText(rootEl, 'Close'), 'Not now becomes Close');
}

// --- expiry while displayed ---
{
  const api = makeApi();
  card.render(doc, rootEl, loginView, api);
  api.timers[0].fn();
  assert.strictEqual(byText(rootEl, 'Save').hidden, true);
  assert.ok(allText(rootEl).includes('expired'));
}

// --- 2FA: zero / one / many accounts, states ---
const totpBase = { id: 't1', kind: 'saveTotp', origin: 'https://login.example.com', label: 'Example:alice', secretDetected: true, expiresAt: 61000 };
{
  const api = makeApi();
  card.render(doc, rootEl, { ...totpBase, state: 'ready', accounts: [] }, api);
  assert.ok(allText(rootEl).includes('Add 2FA code?'));
  assert.ok(allText(rootEl).includes('Example:alice'));
  assert.ok(allText(rootEl).includes('A new entry will be created'));
  byText(rootEl, 'Add').fire('click');
  assert.deepStrictEqual(api.sent[0], { type: 'consumePrompt', promptId: 't1', action: 'save' }, 'no entryId for a new entry');
}
{
  const api = makeApi();
  card.render(doc, rootEl, { ...totpBase, state: 'ready', accounts: [{ id: 'E1', title: 'Example', username: 'alice', hasTotp: false }] }, api);
  assert.ok(allText(rootEl).includes('Add to Example (alice).'));
  byText(rootEl, 'Add').fire('click');
  assert.strictEqual(api.sent[0].entryId, undefined, 'the background resolves the single account itself');
}
{
  const api = makeApi();
  const accounts = [
    { id: 'A', title: 'Work', username: 'a@x.com', hasTotp: false },
    { id: 'B', title: 'Personal', username: 'b@x.com', hasTotp: true }
  ];
  card.render(doc, rootEl, { ...totpBase, state: 'ready', accounts }, api);
  const add = byText(rootEl, 'Add');
  assert.strictEqual(add.disabled, true, 'a choice is required with several accounts');
  add.fire('click');
  assert.strictEqual(api.sent.length, 0, 'a disabled Add sends nothing');
  const personal = byText(rootEl, 'Personal — b@x.com');
  personal.fire('click');
  assert.strictEqual(personal.attrs['aria-pressed'], 'true');
  assert.strictEqual(byText(rootEl, 'Work — a@x.com').attrs['aria-pressed'], 'false');
  assert.strictEqual(add.disabled, false);
  add.fire('click');
  assert.deepStrictEqual(api.sent[0], { type: 'consumePrompt', promptId: 't1', action: 'save', entryId: 'B' });
}
for (const [state, snippet] of [['locked', 'MyPass is locked'], ['pairingRequired', 'not connected'], ['unavailable', "Couldn't reach MyPass"]]) {
  const api = makeApi();
  card.render(doc, rootEl, { ...totpBase, state, accounts: [] }, api);
  assert.ok(allText(rootEl).includes(snippet), state);
  assert.strictEqual(byText(rootEl, 'Add'), undefined, `${state}: nothing to save yet`);
  byText(rootEl, 'Try again').fire('click');
  assert.strictEqual(api.reloadCalls, 1);
}

// --- hostile strings stay text ---
{
  const api = makeApi();
  card.render(doc, rootEl, { ...loginView, username: '<img src=x onerror=alert(1)>', origin: 'https://x.example' }, api);
  assert.ok(walk(rootEl).every((n) => n.tag !== 'img'), 'a username is never parsed as markup');
}

// --- settings card ---
{
  const api = makeApi();
  api.stored.settings = { showFieldIcon: false };
  card.render(doc, rootEl, { id: 's1', kind: 'settings', origin: '', expiresAt: 61000 }, api);
  assert.ok(allText(rootEl).includes('MyPass autofill settings'));
  const boxes = walk(rootEl).filter((n) => n.tag === 'input');
  assert.strictEqual(boxes.length, card.SETTING_ROWS.length);
  assert.strictEqual(boxes[2].checked, false, 'reflects the stored value');
  boxes[2].checked = true;
  boxes[2].fire('change');
  assert.strictEqual(api.stored.settings.showFieldIcon, true);
  byText(rootEl, 'Done').fire('click');
  assert.strictEqual(api.sent[0].action, 'dismiss');
}

// --- prompt window: loads by id, finishes with the next prompt or closes ---
{
  const shown = [];
  let closed = 0;
  const replies = [{ type: 'prompt', prompt: loginView }];
  let storageListener;
  const env = {
    document: doc,
    chrome: {
      runtime: { lastError: null, sendMessage: (m, cb) => { shown.push(m); cb(m.type === 'getPrompt' ? replies[0] : { type: 'ok' }); } },
      storage: { local: { get: (k, cb) => cb({}), set: () => {} }, onChanged: { addListener: (fn) => { storageListener = fn; } } }
    },
    promptCard: card,
    autofillSettings: autofill,
    promptId: 'p1',
    closeWindow: () => { closed += 1; },
    setTimeout: () => 1,
    clearTimeout: () => {}
  };
  createPromptPage(env);
  assert.deepStrictEqual(shown[0], { type: 'getPrompt', promptId: 'p1' });
  assert.ok(allText(rootEl).includes('Save password?'));
  replies[0] = { type: 'noPrompt' };
  byText(rootEl, 'Save').fire('click');
  assert.strictEqual(closed, 1, 'answering the last prompt closes the window');

  // a session change that drops the shown prompt closes the empty window
  replies[0] = { type: 'prompt', prompt: loginView };
  closed = 0;
  createPromptPage(env);
  replies[0] = { type: 'noPrompt' };
  storageListener({ pendingPrompts: { newValue: undefined } }, 'session');
  assert.strictEqual(closed, 1, 'expiry/tab close empties the window');
  // an unrelated change or the same prompt still present is ignored
  replies[0] = { type: 'prompt', prompt: loginView };
  createPromptPage(env);
  const before = shown.length;
  storageListener({ pendingPrompts: { newValue: [{ id: 'p1' }] } }, 'session');
  storageListener({ other: {} }, 'local');
  assert.strictEqual(shown.length, before, 'no reload while the shown prompt is still pending');

  // nothing to show on open
  replies[0] = { type: 'noPrompt' };
  closed = 0;
  createPromptPage(env);
  assert.strictEqual(closed, 0, 'an empty first load explains instead of closing');
  assert.ok(allText(rootEl).includes('Nothing to confirm'));
}

console.log('promptCard.test.js: card render, selection, retry, expiry, settings, prompt window OK');
