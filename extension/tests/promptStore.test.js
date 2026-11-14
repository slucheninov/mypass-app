const assert = require('node:assert');
const records = require('../shared/prompt-record.js');

const NOW = 1_000_000;
function sender(overrides = {}) {
  return {
    id: 'ext',
    url: 'https://login.example.com/signin?next=/home',
    frameId: 0,
    documentId: 'DOC-1',
    tab: { id: 7, windowId: 3 },
    ...overrides
  };
}
const loginPayload = { username: 'alice', password: 'hunter2-SECRET' };
const totpPayload = { otpauthOrSecret: 'otpauth://totp/Example:alice?secret=JBSWY3DPEHPK3PXP&issuer=Example' };

// --- create: bound to tab / window / frame / document, 120 s lifetime ---
const login = records.create('saveLogin', sender(), loginPayload, NOW);
assert.ok(login, 'a valid top-frame request creates a record');
assert.strictEqual(login.tabId, 7);
assert.strictEqual(login.windowId, 3);
assert.strictEqual(login.frameId, 0);
assert.strictEqual(login.documentId, 'DOC-1');
assert.strictEqual(login.origin, 'https://login.example.com', 'origin comes from sender.url, not the message');
assert.strictEqual(login.expiresAt, NOW + 120000, 'prompts live 120 s');
assert.strictEqual(records.PROMPT_TTL_MS, 120000);
assert.ok(login.id && login.id.length > 8, 'id is random');

// --- refusals ---
assert.strictEqual(records.create('saveLogin', sender({ documentId: undefined }), loginPayload, NOW), null,
  'no documentId: the prompt could land in another document later, so refuse');
assert.strictEqual(records.create('saveLogin', sender({ documentId: '' }), loginPayload, NOW), null);
assert.strictEqual(records.create('saveLogin', sender({ frameId: 4 }), loginPayload, NOW), null, 'save prompts are top frame only');
assert.strictEqual(records.create('saveTotp', sender({ frameId: 2 }), totpPayload, NOW), null);
assert.ok(records.create('settings', sender({ frameId: 2 }), {}, NOW), 'settings carries no page data, any frame');
assert.strictEqual(records.create('saveLogin', { ...sender(), tab: undefined }, loginPayload, NOW), null, 'no sender.tab');
assert.strictEqual(records.create('saveLogin', sender({ url: 'chrome-extension://abc/popup.html' }), loginPayload, NOW), null,
  'only http(s) pages');
assert.strictEqual(records.create('saveLogin', sender({ url: '' }), loginPayload, NOW), null);
assert.strictEqual(records.create('saveLogin', sender(), { username: 'a', password: '' }, NOW), null, 'empty password');
assert.strictEqual(records.create('saveLogin', sender(), { username: 'a', password: 'x'.repeat(5000) }, NOW), null, 'oversized password');
assert.strictEqual(records.create('saveTotp', sender(), { otpauthOrSecret: '' }, NOW), null);
assert.strictEqual(records.create('nope', sender(), {}, NOW), null, 'unknown kind');

// --- expiry ---
assert.strictEqual(records.isExpired(login, NOW + 119999), false);
assert.strictEqual(records.isExpired(login, NOW + 120000), true, 'expired at exactly expiresAt');
assert.deepStrictEqual(records.prune([login], NOW + 120000), [], 'prune drops expired records');
assert.strictEqual(records.find([login], login.id, NOW + 120001), null, 'an expired record is never found');

// --- one per (tab, kind); another kind queues; newest first ---
const totp = records.create('saveTotp', sender(), totpPayload, NOW + 10);
const login2 = records.create('saveLogin', sender(), { username: 'bob', password: 'second-SECRET' }, NOW + 20);
let list = records.upsert([], login);
list = records.upsert(list, totp);
assert.strictEqual(list.length, 2, 'a different kind for the same tab queues instead of replacing');
list = records.upsert(list, login2);
assert.strictEqual(list.length, 2, 'a newer request of the same (tab, kind) replaces the older');
assert.ok(!list.some((r) => r.id === login.id), 'the old record is gone');
assert.deepStrictEqual(records.forTab(list, 7, NOW + 30).map((r) => r.kind), ['saveLogin', 'saveTotp'], 'newest first');
const other = records.create('saveLogin', sender({ tab: { id: 8, windowId: 3 } }), loginPayload, NOW + 40);
list = records.upsert(list, other);
assert.strictEqual(list.length, 3, 'another tab has its own slot');
assert.deepStrictEqual(records.forTab(list, 8, NOW + 50).map((r) => r.id), [other.id]);

// --- consume is single-use ---
const first = records.take(list, other.id, NOW + 60);
assert.strictEqual(first.record.id, other.id);
assert.strictEqual(first.rest.length, 2);
const second = records.take(first.rest, other.id, NOW + 61);
assert.strictEqual(second.record, null, 'a second consume of the same id finds nothing');
assert.strictEqual(records.take(list, other.id, NOW + 120040).record, null, 'an expired record cannot be consumed');

// --- tab removal ---
assert.deepStrictEqual(records.removeTab(list, 7).map((r) => r.id), [other.id], 'closing a tab drops its prompts only');

// --- navigation exception: top-frame saveLogin only, exact origin ---
assert.strictEqual(records.survivesNavigation(login), true, 'a top-frame saveLogin may outlive its document');
assert.strictEqual(records.survivesNavigation(totp), false, 'a 2FA seed keeps the strict document binding');
assert.strictEqual(records.survivesNavigation(records.create('settings', sender(), {}, NOW)), false, 'settings too');
assert.strictEqual(records.survivesNavigation({ ...login, frameId: 2 }), false, 'never a sub-frame');
assert.strictEqual(records.survivesNavigation(null), false);
assert.strictEqual(records.liveOriginMatches(login, 'https://login.example.com/account?x=1#f'), true, 'same origin, any path');
assert.strictEqual(records.liveOriginMatches(login, 'https://login.example.com:443/'), true, 'the default port is the same origin');
assert.strictEqual(records.liveOriginMatches(login, 'http://login.example.com/signin'), false, 'downgrade to http');
assert.strictEqual(records.liveOriginMatches(login, 'https://login.example.com:8443/'), false, 'other port');
assert.strictEqual(records.liveOriginMatches(login, 'https://example.com/'), false, 'parent domain');
assert.strictEqual(records.liveOriginMatches(login, 'https://evil.example.net/'), false, 'other site');
assert.strictEqual(records.liveOriginMatches(login, 'about:blank'), false);
assert.strictEqual(records.liveOriginMatches(login, 'not a url'), false);
assert.strictEqual(records.liveOriginMatches(login, ''), false);
assert.strictEqual(records.liveOriginMatches(login, undefined), false);
assert.strictEqual(login.pageUrl, 'https://login.example.com/signin?next=/home', 'the captured page url is kept as created');

// --- redaction: the popup never sees the secret ---
const loginView = records.redactedView(login2);
const totpView = records.redactedView(totp);
function deepStrings(value, out = []) {
  if (typeof value === 'string') out.push(value);
  else if (value && typeof value === 'object') Object.values(value).forEach((v) => deepStrings(v, out));
  return out;
}
const everything = deepStrings([loginView, totpView]).join('\n');
assert.ok(!everything.includes('second-SECRET'), 'no password in the redacted view');
assert.ok(!everything.includes('JBSWY3DPEHPK3PXP'), 'no seed in the redacted view');
assert.ok(!everything.includes('otpauth://'), 'no otpauth URL in the redacted view');
assert.ok(!everything.includes('DOC-1'), 'no documentId in the redacted view');
assert.strictEqual(loginView.username, 'bob');
assert.strictEqual(loginView.passwordDetected, true);
assert.strictEqual(totpView.label, 'Example:alice', 'the account label is shown, the query string is not');
assert.strictEqual(totpView.secretDetected, true);
assert.strictEqual(records.redactedView(records.create('saveTotp', sender(), { otpauthOrSecret: 'JBSWY3DPEHPK3PXP' }, NOW)).label, '',
  'a bare base32 secret has no label to show');

console.log('promptStore.test.js: prompt records (create/expire/replace/queue/consume/redact) OK');
