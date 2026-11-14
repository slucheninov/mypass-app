const assert = require('node:assert/strict');
const { sites, hasSiteHint } = require('../shared/passkey-sites.js');
const { passkeyProxyStatusText } = require('../popup.js');

const disabled = { proxyState: 'disabled' };
const savedPasskey = { id: 'saved', title: 'Example', hasPasskey: true };
const siteContext = (url) => ({ kind: 'site', url });
for (const site of sites) {
  assert.ok(site.source.startsWith('https://'));
  assert.match(site.verifiedOn, /^\d{4}-\d{2}-\d{2}$/);
  assert.ok(site.eligibility);
  for (const host of site.hosts) {
    assert.equal(hasSiteHint('https://' + host + '/login'), true, host);
    assert.equal(hasSiteHint('https://' + host.toUpperCase() + '/login'), true, 'URL hostname normalization');
    assert.equal(hasSiteHint('https://' + host + '.attacker.example'), false, 'suffix impersonation');
    assert.equal(hasSiteHint('https://attacker.' + host), false, 'subdomains are not implied');
    assert.equal(hasSiteHint('http://' + host), false, 'catalog hints require HTTPS');
  }
}
for (const url of ['', undefined, null, 'broken', 'chrome://extensions/', 'about:blank',
  'chrome-extension://test/popup.html', 'javascript:alert(1)', 'http://unknown.example']) {
  assert.equal(hasSiteHint(url, [savedPasskey]), false, String(url));
}
assert.equal(hasSiteHint('https://unknown.example', [savedPasskey]), true);
assert.equal(hasSiteHint('https://unknown.example', [{ hasPasskey: false }]), false);
assert.equal(hasSiteHint('https://unknown.example', [{ hasPasskey: 'true' }]), false);
assert.equal(hasSiteHint('https://github.com@attacker.example'), false);
assert.equal(hasSiteHint('https://unknown.example/?return_to=https://github.com'), false);
assert.equal(hasSiteHint('https://gist.github.com'), false);
assert.equal(hasSiteHint('https://login.microsoftonline.com'), false, 'organization support is not inferred');
assert.equal(passkeyProxyStatusText(disabled), '', 'context is required');
assert.match(passkeyProxyStatusText(disabled, siteContext('https://github.com/login')), /Passkeys are off/);
assert.equal(passkeyProxyStatusText(disabled, siteContext('https://unknown.example')), '');
assert.match(passkeyProxyStatusText(disabled, siteContext('https://unknown.example'), [savedPasskey]), /Passkeys are off/);
assert.equal(passkeyProxyStatusText(disabled, { kind: 'category', category: 'passkeys' }, [savedPasskey]), '');
assert.equal(passkeyProxyStatusText(disabled, { kind: 'search', query: 'GitHub' }, [savedPasskey]), '');
assert.equal(passkeyProxyStatusText({ proxyState: 'active' }, siteContext('https://github.com')), '');
assert.match(passkeyProxyStatusText({ proxyState: 'attachment-error' }), /MyPass settings/);

// Drive actual popup callbacks in different orders, including stale site matches.
// A test of the pure helper alone would miss a status callback rendering before
// tabs.query/getCredentials or a category change retaining the previous hint.
function loadPopup() {
  const elements = {};
  const pending = [];
  let tabCallback;
  let created = 0;
  function element(id) {
    const el = {
      id, hidden: true, textContent: '', value: '', checked: false, disabled: false,
      className: '', style: {}, children: [], listeners: {},
      appendChild(child) { this.children.push(child); },
      addEventListener(name, fn) { (this.listeners[name] ||= []).push(fn); },
      setAttribute(name, value) { this[name] = value; }
    };
    Object.defineProperty(el, 'innerHTML', {
      get() { return ''; },
      set(value) { if (value === '') this.children = []; }
    });
    elements[id] = el;
    return el;
  }
  const storage = { get: (_keys, cb) => cb({}), set: (_items, cb) => cb && cb(), remove: (_keys, cb) => cb && cb() };
  global.document = {
    getElementById: (id) => elements[id] || element(id),
    createElement: () => element('created-' + ++created)
  };
  global.window = { close() {} };
  global.chrome = {
    storage: { local: storage, session: storage },
    runtime: {
      sendMessage(message, cb) {
        if (['getPasskeyProxyStatus', 'getCredentials', 'listEntries', 'searchEntries'].includes(message.type)) {
          pending.push({ message, cb });
        } else if (cb) cb({});
      }
    },
    tabs: { query: (_query, cb) => { tabCallback = cb; }, sendMessage: (_id, _message, cb) => cb({}) }
  };
  delete require.cache[require.resolve('../popup.js')];
  require('../popup.js');
  return {
    elements,
    tab(url) { tabCallback([{ id: 1, url }]); },
    respond(type, result) {
      const index = pending.findIndex((item) => item.message.type === type);
      assert.notEqual(index, -1, 'pending ' + type);
      pending.splice(index, 1)[0].cb(result);
    },
    select(title) {
      elements['category-menu'].children.find((child) => child.textContent.replace('✓ ', '') === title).listeners.click[0]();
    },
    search(query) { elements['vault-search'].value = query; elements['vault-search'].listeners.input[0](); },
    visible() { return !elements['passkey-proxy-status'].hidden; }
  };
}

// Status can arrive before the tab is known, after the tab, or after its matches.
for (const statusOrder of ['first', 'middle', 'last']) {
  for (const url of ['https://github.com/login', 'https://unknown.example']) {
    const popup = loadPopup();
    if (statusOrder === 'first') popup.respond('getPasskeyProxyStatus', disabled);
    popup.tab(url);
    if (statusOrder === 'middle') popup.respond('getPasskeyProxyStatus', disabled);
    popup.respond('getCredentials', { type: 'credentialList', credentials: [savedPasskey] });
    if (statusOrder === 'last') popup.respond('getPasskeyProxyStatus', disabled);
    assert.equal(popup.visible(), true, statusOrder + ': catalog or matched passkey');
    popup.select('Favorites');
    assert.equal(popup.visible(), false, 'hide immediately on category change');
    popup.respond('listEntries', { type: 'credentialList', credentials: [savedPasskey] });
    assert.equal(popup.visible(), false, 'global credentials cannot infer site support');
  }
}

for (const url of ['https://github.com/login', 'https://unknown.example']) {
  const popup = loadPopup();
  popup.respond('getPasskeyProxyStatus', disabled);
  popup.tab(url);
  popup.respond('getCredentials', { type: 'credentialList', credentials: [] });
  assert.equal(popup.visible(), url.includes('github'), 'empty catalog sites still get the hint');
  popup.search('GitHub');
  popup.respond('searchEntries', { type: 'credentialList', credentials: [savedPasskey] });
  assert.equal(popup.visible(), false, 'search results do not infer current-site support');
}

const stale = loadPopup();
stale.tab('https://unknown.example');
stale.select('All');
stale.respond('getCredentials', { type: 'credentialList', credentials: [savedPasskey] });
stale.respond('getPasskeyProxyStatus', disabled);
assert.equal(stale.visible(), false, 'late site matches cannot revive the hint in global browsing');
stale.respond('listEntries', { type: 'credentialList', credentials: [] });
stale.select('This site');
assert.equal(stale.visible(), false, 'previous matches were discarded before a new lookup');
stale.respond('getCredentials', { type: 'credentialList', credentials: [savedPasskey] });
assert.equal(stale.visible(), true);
stale.select('This site');
stale.respond('getCredentials', { type: 'credentialList', credentials: [] });
assert.equal(stale.visible(), false, 'removing the last matching passkey removes the hint');

for (const url of ['chrome://extensions/', 'http://github.com/login', 'https://github.com.attacker.example']) {
  const popup = loadPopup();
  popup.tab(url);
  popup.respond('getPasskeyProxyStatus', disabled);
  popup.respond(url.startsWith('chrome:') ? 'listEntries' : 'getCredentials', { type: 'credentialList', credentials: [] });
  assert.equal(popup.visible(), false, 'irrelevant page: ' + url);
}

console.log('passkeySites.test.js: exact local hints, context changes and callback ordering OK');
