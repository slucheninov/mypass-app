const assert = require('node:assert');
const path = require('node:path');

const elements = {};
function makeEl(id) {
  const el = {
    id,
    hidden: id !== '',
    textContent: '',
    value: '',
    checked: false,
    disabled: false,
    onclick: null,
    className: '',
    style: {},
    children: [],
    listeners: {},
    appendChild(child) { this.children.push(child); },
    addEventListener(name, fn) { (this.listeners[name] = this.listeners[name] || []).push(fn); },
    setAttribute(name, value) { this[name] = value; }
  };
  let innerHTML = '';
  Object.defineProperty(el, 'innerHTML', {
    get() { return innerHTML; },
    set(value) {
      innerHTML = value;
      if (value === '') el.children = [];
    }
  });
  elements[id] = el;
  return el;
}

const sentMessages = [];
const createdTabs = [];
const createdWindows = [];
let activeTab = { url: 'chrome://extensions/', id: 7 };
let settingsResponse = { type: 'ok' };
let lockVaultResponse = { type: 'ok' };
let openAppResponse = { type: 'ok' };
let passkeyStatusResponse = { type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: true, enabled: true };
let passkeySetResponse = (msg) => ({ type: 'passkeyProxyEnabled', enabled: !!msg.enabled });

global.document = {
  getElementById: (id) => elements[id] || makeEl(id),
  createElement: () => makeEl(`created-${Object.keys(elements).length}`)
};
global.window = {
  close() { elClosed += 1; },
  screen: { availWidth: 1440, availHeight: 900, availLeft: 0, availTop: 0 }
};
let elClosed = 0;

global.chrome = {
  storage: {
    local: {
      get: (_keys, cb) => cb({}),
      set: (_items, cb) => cb && cb(),
      remove: (_keys, cb) => cb && cb()
    },
    session: {
      get: (_keys, cb) => cb({}),
      set: (_items, cb) => cb && cb(),
      remove: (_keys, cb) => cb && cb()
    }
  },
  runtime: {
    lastError: null,
    getURL(path) { return 'chrome-extension://hjijbabodfldconmhdcdphlbgaajkejn/' + path; },
    sendMessage(msg, cb) {
      sentMessages.push(msg);
      if (!cb) return;
      if (msg.type === 'openBrowserSettings') { cb(settingsResponse); return; }
      if (msg.type === 'lockVault') { cb(lockVaultResponse); return; }
      if (msg.type === 'openApp') { cb(openAppResponse); return; }
      if (msg.type === 'getPasskeyProxyStatus') { cb(passkeyStatusResponse); return; }
      if (msg.type === 'setPasskeyProxyEnabled') { cb(passkeySetResponse(msg)); return; }
      if (msg.type === 'listEntries') {
        cb({
          type: 'credentialList',
          credentials: msg.category === 'favorites'
            ? [
                { id: 'fav-1', title: 'Alpha', username: 'ada', url: 'https://alpha.example/login', hasTotp: false, hasPasskey: false },
                { id: 'fav-2', title: 'No Site', username: 'bob', hasTotp: true, hasPasskey: false }
              ]
            : [{ id: 'all-1', title: 'All Item', username: 'sam', url: 'https://all.example', hasTotp: false, hasPasskey: false }]
        });
        return;
      }
      if (msg.type === 'searchEntries') {
        cb({
          type: 'credentialList',
          credentials: [{ id: 's-1', title: 'Search Hit', username: 'sue', url: 'https://search.example', hasTotp: false, hasPasskey: false }]
        });
        return;
      }
      if (msg.type === 'getGeneratedPasswordHistory') {
        cb({
          type: 'generatedPasswordHistory',
          entries: [{ id: 'h1', password: 'secret-history', url: 'https://bank.example/signup', createdAt: 1 }]
        });
        return;
      }
      if (msg.type === 'getCredentials') {
        cb({ type: 'credentialList', credentials: [{ id: 'site-1', title: 'Example', username: 'alice', hasPasskey: false }] });
        return;
      }
      cb({ type: 'ok' });
    }
  },
  tabs: {
    query(_opts, cb) { cb([activeTab]); },
    create(props, cb) {
      createdTabs.push(props);
      if (cb) cb({ id: 42 });
    },
    sendMessage() {}
  },
  windows: {
    create(props, cb) {
      createdWindows.push(props);
      if (cb) cb({ id: 8 });
    }
  }
};

function loadPopup() {
  const resolved = require.resolve(path.join(__dirname, '..', 'popup.js'));
  delete require.cache[resolved];
  sentMessages.length = 0;
  createdTabs.length = 0;
  createdWindows.length = 0;
  elClosed = 0;
  for (const key of Object.keys(elements)) delete elements[key];
  return require(resolved);
}

const siteProbe = loadPopup();
assert.strictEqual(siteProbe.isSiteUrl('https://example.com/login'), true);
assert.strictEqual(siteProbe.isSiteUrl('http://example.com'), true);
assert.strictEqual(siteProbe.isSiteUrl('chrome://extensions/'), false);
assert.strictEqual(siteProbe.isSiteUrl('chrome://newtab/'), false);
assert.strictEqual(siteProbe.isSiteUrl('about:blank'), false);
assert.strictEqual(siteProbe.isSiteUrl(''), false);
assert.strictEqual(siteProbe.isSiteUrl(undefined), false);
assert.strictEqual(siteProbe.FAVORITES_HEADER_TEXT, 'Favorites');
assert.strictEqual(siteProbe.NO_FAVORITES_TEXT, 'No favorites');

assert.ok(
  sentMessages.some((message) => message.type === 'listEntries' && message.category === 'favorites'),
  'chrome://extensions asks for favorites'
);
assert.ok(!sentMessages.some((message) => message.type === 'getCredentials'), 'a browser page does not look up a site');
assert.strictEqual(elements['category-btn'].textContent, 'Favorites ▾');
assert.strictEqual(elements['list-panel'].hidden, false);
assert.strictEqual(elements['browser-chrome'].hidden, false);
assert.strictEqual(elements['app-unavailable-panel'].hidden, true);
assert.strictEqual(elements['settings-panel'], undefined, 'settings are hosted by the native app');
assert.strictEqual(elements['popup-menu'].hidden, true, 'the menu stays closed on the credential list');
assert.strictEqual(elements['browser-chrome'].hidden, false, 'the gear stays with the search field');

const rows = elements['list'].children;
assert.strictEqual(rows.length, 2);
rows[0].listeners.click[0]();
assert.deepStrictEqual(createdTabs, [{ url: 'https://alpha.example/login', active: true }]);
assert.strictEqual(elClosed, 1);
assert.ok(
  sentMessages.some((message) => message.type === 'armOpenAndFill' && message.tabId === 42 && message.id === 'fav-1'),
  'opening a favorite arms a fill for the new tab'
);
assert.ok(!sentMessages.some((message) => message.type === 'fillCredential'), 'the popup does not fill the chrome:// tab');

rows[1].listeners.click[0]();
assert.strictEqual(createdTabs.length, 1, 'a favorite with no http(s) URL is not opened');
assert.strictEqual(rows[1].children.some((child) => child.className === 'totp-circle'), false, 'TOTP needs a site URL to ask the app');

function hasText(node, text) {
  if (!node) return false;
  if (node.textContent === text) return true;
  return (node.children || []).some((child) => hasText(child, text));
}

elements['vault-search'].value = 'ada';
elements['vault-search'].listeners.input[0]();
assert.ok(sentMessages.some((message) => message.type === 'searchEntries' && message.query === 'ada'));
assert.ok(hasText(elements['list'].children[0], 'Search Hit'));

const allButton = elements['category-menu'].children.find((child) => child.textContent === 'All');
allButton.listeners.click[0]();
assert.ok(sentMessages.some((message) => message.type === 'listEntries' && message.category === 'all'));
assert.strictEqual(elements['category-btn'].textContent, 'All ▾');
assert.strictEqual(elements['vault-search'].value, '');

elements['add-entry-btn'].listeners.click[0]();
assert.strictEqual(elements['add-entry-menu'].hidden, false);
const loginChoice = elements['add-entry-menu'].children.find((child) => hasText(child, 'New login'));
loginChoice.listeners.click[0]();
assert.strictEqual(createdWindows.length, 1);
assert.strictEqual(createdWindows[0].type, 'popup');
assert.strictEqual(createdWindows[0].width, 520);
assert.strictEqual(createdWindows[0].height, 760);
assert.strictEqual(createdWindows[0].left, Math.round((1440 - 520) / 2));
assert.ok(createdWindows[0].url.includes('create.html?'));
assert.ok(createdWindows[0].url.includes('type=Login'));
assert.strictEqual(elClosed, 2, 'the popup closes after the create window opens');

elements['open-generator-btn'].listeners.click[0]();
assert.ok(createdWindows[1].url.includes('generator.html?'));
assert.ok(createdWindows[1].url.includes('purpose=autofill'));
assert.strictEqual(createdWindows[1].width, 420);
assert.strictEqual(createdWindows[1].left, Math.round((1440 - 420) / 2));

activeTab = { id: 1 };
loadPopup();
assert.ok(
  sentMessages.some((message) => message.type === 'listEntries' && message.category === 'favorites'),
  'a tab with no URL asks for favorites'
);

activeTab = { url: 'chrome://newtab/', id: 2 };
loadPopup();
assert.ok(
  sentMessages.some((message) => message.type === 'listEntries' && message.category === 'favorites'),
  'the new tab asks for favorites'
);

activeTab = { url: 'https://example.com/login', id: 3 };
loadPopup();
assert.ok(sentMessages.some((message) => message.type === 'getCredentials'), 'a website still asks for matching logins');
assert.ok(!sentMessages.some((message) => message.type === 'listEntries' && message.category === 'favorites'));
assert.strictEqual(elements['category-btn'].textContent, 'This site ▾');

elements['add-entry-btn'].listeners.click[0]();
elements['add-entry-menu'].children.find((child) => hasText(child, 'New login')).listeners.click[0]();
assert.ok(decodeURIComponent(createdWindows[0].url).includes('type=Login'));
assert.ok(decodeURIComponent(createdWindows[0].url).includes('https://example.com/login'));
elements['add-entry-menu'].children.find((child) => hasText(child, 'New card')).listeners.click[0]();
assert.ok(createdWindows[1].url.includes('type=Card'));
assert.strictEqual(createdWindows[1].height, 680);
elements['add-entry-menu'].children.find((child) => hasText(child, 'New note')).listeners.click[0]();
assert.ok(createdWindows[2].url.includes('type=Notes'));
assert.strictEqual(createdWindows[2].height, 640);
elements['open-generator-btn'].listeners.click[0]();
assert.ok(decodeURIComponent(createdWindows[3].url).includes('https://example.com/login'));

const background = require('node:fs').readFileSync(path.join(__dirname, '..', 'background.js'), 'utf8');
assert.ok(background.includes('"listEntries"'), 'background.js forwards listEntries to the app');
assert.ok(background.includes('"searchEntries"'));
assert.ok(background.includes('"createEntry"'));

const html = require('node:fs').readFileSync(path.join(__dirname, '..', 'popup.html'), 'utf8');
assert.ok(!html.includes('Use MyPass for Chrome passkeys'));
assert.ok(!html.includes('id="settings-panel"') && !html.includes('id="passkey-proxy-toggle"'));
assert.ok(!html.includes('id="passkey-settings-footer"'));
assert.ok(!html.includes('Watchtower') && !html.includes('favicon'));
assert.ok(html.includes('Lock MyPass'));
assert.ok(!html.includes('>Settings</button>'), 'the gear menu has no Settings row; settings live on the options page');
assert.ok(html.includes('>Help</button>'));
const searchAt = html.indexOf('id="vault-search"');
const gearAt = html.indexOf('id="popup-menu-btn"');
const generatorAt = html.indexOf('id="open-generator-btn"');
assert.ok(gearAt !== -1 && gearAt < searchAt && searchAt < generatorAt, 'the menu button sits to the left of search');
assert.ok(html.includes('id="vault-search"'));
assert.ok(html.includes('id="open-generator-btn"'));
assert.ok(html.includes('id="add-entry-btn"'));
assert.ok(html.includes('aria-label="Password generator"'));
assert.ok(html.includes('aria-label="More"') && html.includes('title="More"'), 'the menu button is a neutral More button');
assert.ok(!html.includes('aria-label="MyPass menu"'), 'the old gear label is gone');
const moreBtn = html.slice(gearAt, html.indexOf('</button>', gearAt));
assert.ok(moreBtn.includes('<circle') && !moreBtn.includes('M19.14 12.94'), 'More icon is dots, not the gear path');

elements['popup-menu-btn'].listeners.click[0]();
assert.strictEqual(elements['popup-menu'].hidden, false);
assert.strictEqual(elements['popup-menu-root'].hidden, false);
assert.strictEqual(elements['popup-menu-btn']['aria-expanded'], 'true');
elements['popup-menu-help-btn'].listeners.click[0]();
assert.strictEqual(elements['popup-menu-help'].hidden, false);
elements['popup-menu-help-back'].listeners.click[0]();
assert.strictEqual(elements['popup-menu-root'].hidden, false);
lockVaultResponse = { type: 'pairingRequired' };
elements['lock-vault-btn'].listeners.click[0]();
assert.strictEqual(sentMessages.at(-1).type, 'lockVault');
assert.strictEqual(elements['lock-vault-error'].hidden, false);
assert.match(elements['lock-vault-error'].textContent, /Connect this browser/);
assert.strictEqual(elements['list-panel'].hidden, false, 'a refused lock keeps the list');
lockVaultResponse = { type: 'ok' };
elements['lock-vault-btn'].listeners.click[0]();
assert.strictEqual(elements['locked-panel'].hidden, false);
assert.strictEqual(elements['popup-menu'].hidden, true, 'locking closes the menu');

console.log('favoritesPopup.test.js: OK');

// Passkeys browsing opens the relying-party site without arming password injection.
const passkeyPopup = loadPopup();
const passkeysButton = elements['category-menu'].children.find((child) => child.textContent === 'Passkeys');
passkeysButton.listeners.click[0]();
assert.ok(sentMessages.some((message) => message.type === 'listEntries' && message.category === 'passkeys'));
assert.strictEqual(elements['site-passkey-hint'].hidden, false);
assert.strictEqual(elements['site-passkey-hint'].textContent, 'Select an entry to open its website.');
sentMessages.length = 0;
elements['list'].children[0].listeners.click[0]();
assert.deepStrictEqual(createdTabs, [{ url: 'https://all.example', active: true }]);
assert.ok(!sentMessages.some((message) => ['armOpenAndFill', 'fillCredential'].includes(message.type)));
passkeyPopup.renderCredentials([{ id: 'unsafe', title: 'Unsafe', url: 'javascript:alert(1)', hasPasskey: true }], null, { openSavedUrl: true, passkeyOnly: true });
elements['list'].children[0].listeners.click[0]();
assert.strictEqual(createdTabs.length, 1, 'passkey navigation rejects non-http(s) URLs');
console.log('favoritesPopup.test.js: passkeys open safe websites without password fill OK');
