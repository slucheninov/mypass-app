const assert = require('node:assert');
const path = require('node:path');

// --- background.js: the allowlist gate --------------------------------------------

// background.js registers chrome.runtime.onMessage.addListener at module load time
// (top-level, unconditional — see pairing.test.js's identical stub), so even this
// first, read-only require of `knownTypes` needs a minimal global.chrome in place or
// node throws "chrome is not defined" before the array is ever inspected.
global.chrome = {
  runtime: {
    lastError: null,
    onMessage: { addListener: () => {} }
  }
};

const { knownTypes } = require(path.join(__dirname, '..', 'background.js'));
assert.ok(
  knownTypes.includes('generatePassword'),
  'generatePassword must be in the allowlist or the message is silently dropped before it ever reaches the native host'
);

// --- background.js: handleRuntimeMessage forwards it to the native host ------------
// ------ and relays a successful popup-originated response to the active tab -------

let sentNativeMessage = null;
const nativeMessages = [];
let tabsQueryCalls = [];
let tabsSendMessageCalls = [];
const storedLocal = {};
const storedSession = {};
global.chrome = {
  runtime: {
    lastError: null,
    id: 'test-extension-id',
    sendNativeMessage: (hostName, message, cb) => {
      nativeMessages.push(message);
      sentNativeMessage = message;
      if (message.type === 'fillCredential') {
        cb({ type: 'credential', username: 'ada', password: 'secret' });
        return;
      }
      if (message.type === 'recordGeneratedPassword' || message.type === 'deleteGeneratedPassword') {
        cb({ type: 'ok' });
        return;
      }
      if (message.type === 'getGeneratedPasswordHistory') {
        cb({
          type: 'generatedPasswordHistory',
          entries: [{ id: 'h1', password: 'from-vault', url: 'https://gitlab.com', createdAt: 1700000000000 }]
        });
        return;
      }
      cb({ type: 'generatedPassword', password: 'Tr0ub4dor&3' });
    },
    onMessage: { addListener: () => {} }
  },
  storage: {
    local: {
      get: (key, cb) => {
        if (Array.isArray(key)) {
          const result = {};
          key.forEach((name) => {
            if (storedLocal[name] !== undefined) result[name] = storedLocal[name];
          });
          cb(result);
          return;
        }
        cb(key ? { [key]: storedLocal[key] } : { ...storedLocal });
      },
      set: (obj, cb) => {
        Object.assign(storedLocal, obj);
        if (cb) cb();
      },
      remove: (key, cb) => {
        const names = Array.isArray(key) ? key : [key];
        names.forEach((name) => { delete storedLocal[name]; });
        if (cb) cb();
      }
    },
    session: {
      get: (key, cb) => {
        const names = Array.isArray(key) ? key : [key];
        const result = {};
        names.forEach((name) => {
          if (storedSession[name] !== undefined) result[name] = storedSession[name];
        });
        cb(result);
      },
      set: (obj, cb) => {
        Object.assign(storedSession, obj);
        if (cb) cb();
      },
      remove: (key, cb) => {
        const names = Array.isArray(key) ? key : [key];
        names.forEach((name) => { delete storedSession[name]; });
        if (cb) cb();
      }
    }
  },
  // Only reached by the popup-sender scenario below — a content-script sender must never
  // touch chrome.tabs at all (background.js gates the relay on !sender.tab), which the
  // second scenario proves by asserting these arrays stayed empty.
  tabs: {
    query: (opts, cb) => { tabsQueryCalls.push(opts); cb([{ id: 99, url: 'https://example.com/account' }]); },
    sendMessage: (tabId, message, options, cb) => {
      tabsSendMessageCalls.push({ tabId, message, options });
      if (cb) cb();
    }
  }
};

delete require.cache[require.resolve(path.join(__dirname, '..', 'background.js'))];
const { handleRuntimeMessage } = require(path.join(__dirname, '..', 'background.js'));

// sendNativeMessageWithPairing (the generic-forwarding branch generatePassword falls
// through to) is async — it awaits readPairingSecret, sendNativeMessage,
// clearStaleSecretOnPairingRequired and persistPairingResponse in turn — so sendResponse
// fires on a later microtask even though every stubbed chrome callback above resolves
// synchronously. Wrap each scenario in a promise that resolves exactly when sendResponse
// runs, rather than asserting immediately after the synchronous call, so this doesn't
// race the chain. Both scenarios below share `sentNativeMessage`/`tabsQueryCalls`/
// `tabsSendMessageCalls`, so they run sequentially inside one async IIFE rather than as
// two independent ones — two independent async IIFEs would interleave their synchronous
// prologues (the second scenario's reset stomping the first scenario's in-flight state)
// before either one's response promise had a chance to settle.
(async () => {
  // Scenario 1: a popup-originated request (no sender.tab — the popup is not injected
  // into any tab) whose success must be relayed to the active tab.
  {
    let responded = null;
    let keepChannelOpen;
    await new Promise((resolve) => {
      keepChannelOpen = handleRuntimeMessage(
        // `url` travels alongside `domain` for the same reason popup.js's real request
        // does: the relay needs a page URL to hostname-match against the active tab
        // before it will forward the password anywhere.
        { type: 'generatePassword', domain: 'example.com', url: 'https://example.com/signup' },
        {},
        (response) => { responded = response; resolve(); }
      );
    });
    assert.strictEqual(keepChannelOpen, true, 'the async response path must keep the message channel open');
    const generated = nativeMessages.filter((message) => message.type === 'generatePassword');
    assert.strictEqual(generated.length, 1, 'generatePassword must be forwarded to the native host');
    assert.strictEqual(generated[0].domain, 'example.com');
    const recorded = nativeMessages.filter((message) => message.type === 'recordGeneratedPassword');
    assert.strictEqual(recorded.length, 1, 'the generated password is stored in the vault');
    assert.strictEqual(recorded[0].password, 'Tr0ub4dor&3');
    assert.strictEqual(recorded[0].url, 'https://example.com/signup');
    // Stripped, exactly like the fillCredential branch: the secret went to the content
    // script, so the popup's JS runtime must never see it. This assertion used to pin
    // the UNSTRIPPED shape, which is how the leak shipped — the popup only ever checked
    // the password for truthiness, and now branches on this type instead.
    assert.deepStrictEqual(responded, { type: 'generatedPasswordFilled' });

    // The popup has no DOM access of its own, so a successful response must be relayed
    // to the active tab through the exact same relayFillToActiveTab/sendApplyFill path
    // fillCredential already uses — hostname pre-check included — rather than a second,
    // less careful copy of that sequence living in popup.js.
    assert.strictEqual(tabsQueryCalls.length, 1, 'a popup-originated generatePassword success queries the active tab');
    assert.strictEqual(tabsSendMessageCalls.length, 1, 'and relays exactly one applyFill to it');
    const relayed = tabsSendMessageCalls[0];
    assert.strictEqual(relayed.tabId, 99);
    assert.strictEqual(relayed.message.type, 'applyFill');
    assert.strictEqual(relayed.message.password, 'Tr0ub4dor&3');
    assert.strictEqual(relayed.message.fillTarget, 'newPassword',
      'the relayed fill must name the new-password field, or content.js writes it over the current-password box');
    assert.deepStrictEqual(relayed.options, { frameId: 0 }, 'scoped to the top frame, exactly like the fillCredential relay');

    console.log('generatePassword.test.js: background.js allowlist + forwarding + popup relay OK');
  }

  // Scenario 2: a content-script-originated request (sender.tab present — the exact
  // signal resolveSenderFillTarget already keys on elsewhere in this file) must never
  // ALSO be relayed. The content script already wrote the password directly into the
  // field the user clicked (content.js's own setNativeValue fill); relaying it here too
  // would be a second, uncoordinated write into the same page.
  {
    sentNativeMessage = null;
    tabsQueryCalls = [];
    tabsSendMessageCalls = [];
    let responded = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'generatePassword', domain: 'example.com', url: 'https://example.com/signup' },
        { tab: { id: 7 }, url: 'https://example.com/signup', frameId: 0, documentId: 'doc-1' },
        (response) => { responded = response; resolve(); }
      );
    });
    // NOT stripped, deliberately: nothing relays for this sender, and the content script
    // is the thing that does the fill. Stripping here would leave the inline panel's
    // Generate row with nothing to type into the field.
    assert.deepStrictEqual(responded, { type: 'generatedPassword', password: 'Tr0ub4dor&3' });
    assert.strictEqual(tabsQueryCalls.length, 0, 'a content-script-originated request must never trigger the relay');
    assert.strictEqual(tabsSendMessageCalls.length, 0, 'and must never send a second applyFill');

    console.log('generatePassword.test.js: background.js content-script sender is never relayed OK');
  }

  const historyRecords = nativeMessages.filter((message) => message.type === 'recordGeneratedPassword');
  assert.strictEqual(historyRecords.length, 2, 'each generate is sent to the vault');
  assert.strictEqual(historyRecords[0].password, 'Tr0ub4dor&3');
  assert.strictEqual(historyRecords[0].url, 'https://example.com/signup');
  assert.strictEqual(historyRecords[1].url, 'https://example.com/signup');
  assert.strictEqual(storedLocal.generatedPasswordHistory, undefined, 'history is not kept in the browser profile');

  {
    tabsSendMessageCalls = [];
    sentNativeMessage = null;
    let armed = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'armOpenAndFill', tabId: 5, id: 'entry-1', url: 'https://alpha.example/login' },
        {},
        (response) => { armed = response; resolve(); }
      );
    });
    assert.deepStrictEqual(armed, { type: 'ok' });

    let ignored = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'claimPendingFill', url: 'https://alpha.example/login' },
        { tab: { id: 5 }, frameId: 3, url: 'https://alpha.example/login' },
        (response) => { ignored = response; resolve(); }
      );
    });
    assert.deepStrictEqual(ignored, { type: 'none' });
    assert.strictEqual(tabsSendMessageCalls.length, 0, 'a subframe must not take the pending fill');

    let claimed = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'claimPendingFill', url: 'https://alpha.example/login' },
        { tab: { id: 5 }, frameId: 0, url: 'https://alpha.example/login' },
        (response) => { claimed = response; resolve(); }
      );
    });
    assert.deepStrictEqual(claimed, { type: 'ok' });
    assert.strictEqual(sentNativeMessage.type, 'fillCredential');
    assert.strictEqual(sentNativeMessage.id, 'entry-1');
    assert.strictEqual(sentNativeMessage.url, 'https://alpha.example/login');
    assert.strictEqual(tabsSendMessageCalls.length, 1);
    assert.strictEqual(tabsSendMessageCalls[0].tabId, 5);
    assert.strictEqual(tabsSendMessageCalls[0].message.type, 'applyFill');
    assert.strictEqual(tabsSendMessageCalls[0].message.username, 'ada');
    assert.strictEqual(tabsSendMessageCalls[0].message.password, 'secret');
    console.log('generatePassword.test.js: open-and-fill claim delivers the login to the new tab OK');
  }

  {
    nativeMessages.length = 0;
    sentNativeMessage = null;
    tabsQueryCalls = [];
    tabsSendMessageCalls = [];
    let responded = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'generatePassword', domain: 'example.com', url: 'https://example.com/signup', preview: true },
        {},
        (response) => { responded = response; resolve(); }
      );
    });
    assert.deepStrictEqual(responded, { type: 'generatedPassword', password: 'Tr0ub4dor&3' });
    assert.ok(nativeMessages.some((message) => message.type === 'generatePassword'));
    assert.ok(nativeMessages.some((message) => message.type === 'recordGeneratedPassword' && message.password === 'Tr0ub4dor&3'));
    assert.strictEqual(tabsQueryCalls.length, 0, 'a generator preview must not fill the active tab');
    assert.strictEqual(tabsSendMessageCalls.length, 0);
    console.log('generatePassword.test.js: extension-page preview returns the password OK');
  }

  {
    storedLocal.passwordGeneratorSettings = {
      type: 'pin',
      useAsDefault: true,
      pin: { length: 4 }
    };
    nativeMessages.length = 0;
    sentNativeMessage = null;
    tabsQueryCalls = [];
    tabsSendMessageCalls = [];
    let responded = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'generatePassword', domain: 'example.com', url: 'https://example.com/signup' },
        {},
        (response) => { responded = response; resolve(); }
      );
    });
    assert.ok(!nativeMessages.some((message) => message.type === 'generatePassword'), 'a default PIN is generated in the extension');
    assert.deepStrictEqual(responded, { type: 'generatedPasswordFilled' });
    assert.strictEqual(tabsSendMessageCalls.length, 1);
    assert.match(tabsSendMessageCalls[0].message.password, /^\d{4}$/);
    assert.strictEqual(nativeMessages[0].type, 'recordGeneratedPassword');
    assert.strictEqual(nativeMessages[0].password, tabsSendMessageCalls[0].message.password);

    nativeMessages.length = 0;
    sentNativeMessage = null;
    tabsSendMessageCalls = [];
    let pageResponse = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'generatePassword', domain: 'example.com', url: 'https://example.com/signup', preview: true },
        { tab: { id: 7 }, url: 'https://example.com/signup', frameId: 0, documentId: 'doc-1' },
        (response) => { pageResponse = response; resolve(); }
      );
    });
    assert.ok(!nativeMessages.some((message) => message.type === 'generatePassword'), 'a content script cannot opt out of the default generator');
    assert.strictEqual(nativeMessages.filter((message) => message.type === 'recordGeneratedPassword').length, 1);
    assert.match(pageResponse.password, /^\d{4}$/);
    assert.strictEqual(pageResponse.type, 'generatedPassword');
    assert.strictEqual(tabsSendMessageCalls.length, 0);
    console.log('generatePassword.test.js: use-by-default PIN stays in the extension OK');
  }

  {
    delete storedLocal.passwordGeneratorSettings;
    nativeMessages.length = 0;
    let rejected = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'recordGeneratedPassword', password: 'from-page', url: 'https://evil.example' },
        { tab: { id: 7 }, url: 'https://evil.example' },
        (response) => { rejected = response; resolve(); }
      );
    });
    assert.deepStrictEqual(rejected, { type: 'error' });
    assert.strictEqual(nativeMessages.length, 0, 'a page must not write generator history');

    let recorded = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'recordGeneratedPassword', password: 'local-memorable', url: 'https://gitlab.com' },
        {},
        (response) => { recorded = response; resolve(); }
      );
    });
    assert.deepStrictEqual(recorded, { type: 'ok' });
    const saved = nativeMessages[nativeMessages.length - 1];
    assert.strictEqual(saved.type, 'recordGeneratedPassword');
    assert.strictEqual(saved.password, 'local-memorable');
    assert.strictEqual(saved.url, 'https://gitlab.com');

    let removed = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'deleteGeneratedPassword', id: 'saved-id' },
        {},
        (response) => { removed = response; resolve(); }
      );
    });
    assert.deepStrictEqual(removed, { type: 'ok' });
    assert.strictEqual(nativeMessages[nativeMessages.length - 1].type, 'deleteGeneratedPassword');
    assert.strictEqual(nativeMessages[nativeMessages.length - 1].id, 'saved-id');

    let pageDelete = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'deleteGeneratedPassword', id: 'saved-id' },
        { tab: { id: 7 } },
        (response) => { pageDelete = response; resolve(); }
      );
    });
    assert.deepStrictEqual(pageDelete, { type: 'error' });

    storedLocal.generatedPasswordHistory = [
      { id: 'old', password: 'from-chrome', url: 'https://old.example', createdAt: 10 }
    ];
    nativeMessages.length = 0;
    let listed = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'getGeneratedPasswordHistory' },
        {},
        (response) => { listed = response; resolve(); }
      );
    });
    assert.strictEqual(listed.type, 'generatedPasswordHistory');
    assert.strictEqual(listed.entries[0].password, 'from-vault');
    assert.ok(nativeMessages.some((message) => (
      message.type === 'recordGeneratedPassword' && message.password === 'from-chrome' && message.createdAt === 10
    )));
    assert.strictEqual(nativeMessages[nativeMessages.length - 1].type, 'getGeneratedPasswordHistory');
    assert.strictEqual(storedLocal.generatedPasswordHistory, undefined);

    const beforePageRead = nativeMessages.length;
    let hidden = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'getGeneratedPasswordHistory' },
        { tab: { id: 7 } },
        (response) => { hidden = response; resolve(); }
      );
    });
    assert.deepStrictEqual(hidden, { type: 'generatedPasswordHistory', entries: [] });
    assert.strictEqual(nativeMessages.length, beforePageRead, 'a page must not read generator history');

    tabsSendMessageCalls = [];
    let filled = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'fillGeneratedPassword', password: 'shown-password', url: 'https://example.com/signup' },
        {},
        (response) => { filled = response; resolve(); }
      );
    });
    assert.deepStrictEqual(filled, { type: 'generatedPasswordFilled' });
    assert.strictEqual(tabsSendMessageCalls[0].message.password, 'shown-password');
    assert.strictEqual(tabsSendMessageCalls[0].message.fillTarget, 'newPassword');

    let pageFill = null;
    await new Promise((resolve) => {
      handleRuntimeMessage(
        { type: 'fillGeneratedPassword', password: 'shown-password', url: 'https://example.com/signup' },
        { tab: { id: 7 } },
        (response) => { pageFill = response; resolve(); }
      );
    });
    assert.deepStrictEqual(pageFill, { type: 'error' });
    console.log('generatePassword.test.js: history record, delete, and autofill stay on the extension page OK');
  }
})();

// --- content.js: buildNewPasswordFieldQueryResponse ---------------------------------

const { buildNewPasswordFieldQueryResponse } = require(path.join(__dirname, '..', 'content.js'));

function fakeInput(attrs) {
  return {
    getAttribute(name) { return Object.prototype.hasOwnProperty.call(attrs, name) ? attrs[name] : null; },
    closest() { return null; },
    disabled: false,
    readOnly: false,
    offsetParent: {}
  };
}

function fakeRoot(inputs) {
  return {
    querySelectorAll(selector) {
      if (selector === 'input[type="password"]') return inputs.filter((i) => i.getAttribute('type') === 'password');
      return [];
    }
  };
}

{
  const loginOnly = fakeRoot([fakeInput({ type: 'password', autocomplete: 'current-password' })]);
  assert.deepStrictEqual(buildNewPasswordFieldQueryResponse(loginOnly), { hasNewPasswordField: false });
}

{
  const signup = fakeRoot([fakeInput({ type: 'password', autocomplete: 'new-password' })]);
  assert.deepStrictEqual(buildNewPasswordFieldQueryResponse(signup), { hasNewPasswordField: true });
}

console.log('generatePassword.test.js: buildNewPasswordFieldQueryResponse OK');

// --- content.js: WHERE a popup-originated generated password actually lands -----------
// The assertions above prove the relay carries the right payload; these prove it reaches
// the right input. Nothing checked that before, which is exactly how a popup-originated
// generate came to write the new secret over the CURRENT-password field of a
// change-password form: with no clicked target, applyFill resolved its destination with
// findLoginForm(), the first visible password input in document order. Signup forms hid
// it, because there the first password field IS the new-password field.

const { resolveApplyFillTargets } = require(path.join(__dirname, '..', 'content.js'));

{
  // The canonical change-password form: current, new, confirm.
  const current = fakeInput({ type: 'password', autocomplete: 'current-password' });
  const fresh = fakeInput({ type: 'password', autocomplete: 'new-password' });
  const confirm = fakeInput({ type: 'password', autocomplete: 'new-password' });
  const root = fakeRoot([current, fresh, confirm]);

  const generated = resolveApplyFillTargets({ type: 'applyFill', fillTarget: 'newPassword' }, null, root);
  assert.strictEqual(generated.passwordInput, fresh,
    'a generated password must land in the new-password field, not the first password field on the page');
  assert.notStrictEqual(generated.passwordInput, current,
    'writing a generated password over the current-password box is the exact defect this asserts against');
  assert.strictEqual(generated.usernameInput, null, 'a generated password carries no username to fill');

  // The ordinary credential fill is unchanged and still targets findLoginForm's pick.
  const credential = resolveApplyFillTargets({ type: 'applyFill' }, null, root);
  assert.strictEqual(credential.passwordInput, current);
}

{
  // Three unlabelled password fields — no autocomplete attributes at all, so
  // isNewPasswordCandidate falls back to "any field after the first".
  const first = fakeInput({ type: 'password' });
  const second = fakeInput({ type: 'password' });
  const third = fakeInput({ type: 'password' });
  const root = fakeRoot([first, second, third]);
  const generated = resolveApplyFillTargets({ type: 'applyFill', fillTarget: 'newPassword' }, null, root);
  assert.strictEqual(generated.passwordInput, second);
}

{
  // A login form has no new-password field at all. The safe failure for a generated
  // password is to go nowhere — never to fall back to the sign-in password box.
  const root = fakeRoot([fakeInput({ type: 'password', autocomplete: 'current-password' })]);
  const generated = resolveApplyFillTargets({ type: 'applyFill', fillTarget: 'newPassword' }, null, root);
  assert.strictEqual(generated.passwordInput, null,
    'with no new-password field the fill must land nowhere rather than in the current-password field');
}

{
  // A clicked field still wins outright: the user chose that group, and the popup's flag
  // must not override a deliberate in-page click.
  const clicked = fakeInput({ type: 'password', autocomplete: 'current-password' });
  const fresh = fakeInput({ type: 'password', autocomplete: 'new-password' });
  const root = fakeRoot([clicked, fresh]);
  const generated = resolveApplyFillTargets(
    { type: 'applyFill', fillTarget: 'newPassword' },
    { input: clicked, kind: 'password' },
    root
  );
  assert.strictEqual(generated.passwordInput, clicked);
}

console.log('generatePassword.test.js: resolveApplyFillTargets destination OK');

// --- content.js: the inline panel's generate-password failure path ------------------
// Covers exactly what previously shipped untested: togglePanel's generate-row click
// handler used to act only on a successful response and otherwise close the panel with
// no explanation at all. classifyGeneratePasswordResponse is the extracted decision
// logic that click handler now calls; showGeneratePasswordMessage is how it renders a
// failure using the panel's own vocabulary.

const { classifyGeneratePasswordResponse, showGeneratePasswordMessage } = require(path.join(__dirname, '..', 'content.js'));

assert.deepStrictEqual(
  classifyGeneratePasswordResponse({ type: 'generatedPassword', password: 'Tr0ub4dor&3' }),
  { action: 'fill', password: 'Tr0ub4dor&3' }
);
assert.deepStrictEqual(
  classifyGeneratePasswordResponse({ type: 'locked' }),
  { action: 'showMessage', message: 'MyPass is locked' }
);
assert.deepStrictEqual(
  classifyGeneratePasswordResponse({ type: 'pairingRequired' }),
  { action: 'showMessage', message: 'Pair MyPass to use it here' }
);
// The two distinct messages Task 11 split apart — one about the user's own generator
// settings, one about the site's requirements — must each reach the user verbatim,
// never collapsed into one generic string.
assert.deepStrictEqual(
  classifyGeneratePasswordResponse({
    type: 'error',
    message: 'Your password generator has no character types enabled. Turn on at least one in Settings and try again.'
  }),
  {
    action: 'showMessage',
    message: 'Your password generator has no character types enabled. Turn on at least one in Settings and try again.'
  }
);
assert.deepStrictEqual(
  classifyGeneratePasswordResponse({ type: 'error', message: "This site's password requirements could not be satisfied" }),
  { action: 'showMessage', message: "This site's password requirements could not be satisfied" }
);
// No usable response at all (extension context lost, native host unreachable with no
// message attached, or a malformed "success" missing its password) falls back to the
// panel's own generic "not running" message rather than throwing or showing nothing.
assert.deepStrictEqual(classifyGeneratePasswordResponse(null), { action: 'showMessage', message: 'MyPass is not running' });
assert.deepStrictEqual(classifyGeneratePasswordResponse({ type: 'error' }), { action: 'showMessage', message: 'MyPass is not running' });
assert.deepStrictEqual(
  classifyGeneratePasswordResponse({ type: 'generatedPassword' }),
  { action: 'showMessage', message: 'MyPass is not running' }
);

console.log('generatePassword.test.js: classifyGeneratePasswordResponse OK');

function fakeMessageDoc() {
  return {
    createElement(tag) {
      return {
        tagName: tag.toUpperCase(),
        attrs: {},
        textContent: '',
        setAttribute(name, value) { this.attrs[name] = value; },
        getAttribute(name) { return Object.prototype.hasOwnProperty.call(this.attrs, name) ? this.attrs[name] : null; }
      };
    }
  };
}

function fakeMessageRoot() {
  const children = [];
  return {
    children,
    appendChild(el) {
      children.push(el);
      el.remove = () => {
        const index = children.indexOf(el);
        if (index !== -1) children.splice(index, 1);
      };
      return el;
    },
    querySelector(selector) {
      const cls = selector.replace('.', '');
      return children.find((el) => (el.attrs.class || '').split(' ').includes(cls)) || null;
    }
  };
}

{
  const doc = fakeMessageDoc();
  const root = fakeMessageRoot();
  showGeneratePasswordMessage(doc, root, 'MyPass is locked');
  assert.strictEqual(root.children.length, 1, 'renders exactly one message element');
  assert.strictEqual(root.children[0].getAttribute('class'), 'message generate-password-message');
  assert.strictEqual(root.children[0].textContent, 'MyPass is locked');

  showGeneratePasswordMessage(doc, root, 'Pair MyPass to use it here');
  assert.strictEqual(root.children.length, 1, 'a later failure replaces the earlier message rather than stacking it');
  assert.strictEqual(root.children[0].textContent, 'Pair MyPass to use it here');
}

console.log('generatePassword.test.js: showGeneratePasswordMessage OK');
