const assert = require('node:assert/strict');
const { test } = require('node:test');
const { openPopup, html, flush } = require('./popupHarness.js');

function unpairedPopup({ startedAt, start, complete, now = 200_000 } = {}) {
  return openPopup({ startedAt, now, reply(message) {
    if (message.type === 'getCredentials') return { type: 'pairingRequired' };
    if (message.type === 'startPairing') return start || { type: 'pairingStarted' };
    if (message.type === 'completePairing') return complete || { type: 'error', message: 'invalid_code' };
    return {};
  } });
}

const countStarts = (h) => h.sent.filter((m) => m.type === 'startPairing').length;

test('pairingRequired auto-sends startPairing once and reveals the code from the app', async () => {
  const h = unpairedPopup();
  await flush();
  assert.equal(countStarts(h), 1);
  assert.equal(h.el('pairing-panel').hidden, false);
  assert.equal(h.el('code-entry').hidden, false);
  await h.advance(10_000);
  assert.equal(countStarts(h), 1);
});

test('a challenge younger than 120 seconds opens the code entry without starting again', async () => {
  const h = unpairedPopup({ startedAt: 81_000 });
  await flush();
  assert.equal(countStarts(h), 0);
  assert.equal(h.el('code-entry').hidden, false);
});

test('a cancelled reused challenge requests a new code once and clears stale storage first', async () => {
  const h = unpairedPopup({ startedAt: 81_000 });
  await flush();
  h.el('code-input').value = '123456';
  h.fire('submit-code-btn', 'click');
  await flush();
  assert.equal(h.storage.pairingStartedAt, undefined);
  assert.equal(countStarts(h), 1);
  assert.equal(h.el('code-entry').hidden, false);
  assert.equal(h.el('code-input').value, '');
  assert.match(h.el('pairing-error').textContent, /new code was requested/i);

  h.el('code-input').value = '654321';
  h.fire('submit-code-btn', 'click');
  await flush();
  assert.equal(countStarts(h), 1);
  assert.match(h.el('pairing-error').textContent, /incorrect|invalid|expired/i);
});

test('a challenge exactly 120 seconds old is refreshed', async () => {
  const h = unpairedPopup({ startedAt: 80_000 });
  await flush();
  assert.equal(countStarts(h), 1);
});

test('the pairing throttle shows its own localized error', async () => {
  const h = unpairedPopup({ start: { type: 'error', message: 'pairing_throttled' } });
  await flush();
  assert.match(h.el('pairing-error').textContent, /Too many pairing attempts/);
  assert.equal(h.el('code-entry').hidden, true);
});

test('the pairing panel has the heading, code input and Send only', () => {
  const panel = html.match(/<section id="pairing-panel"[\s\S]*?<\/section>/)?.[0];
  assert.match(panel, /<h1[^>]+id="pairing-title"/);
  assert.match(panel, /<input[^>]+id="code-input"/);
  assert.match(panel, /id="submit-code-btn"[^>]+data-i18n="sendCode"/);
  assert.doesNotMatch(panel, /id="(?:connect-btn|popup-menu-btn)"/);
});

test('a challenge from a locked vault can be entered and a wrong code still gets its own error', async () => {
  const h = unpairedPopup({ start: { type: 'pairingStarted' } });
  await flush();
  assert.equal(h.el('browser-chrome').hidden, true);
  h.el('code-input').value = '123 456';
  h.fire('submit-code-btn', 'click');
  await flush();
  assert.deepEqual(h.sent.at(-1), { type: 'completePairing', code: '123456' });
  assert.match(h.el('pairing-error').textContent, /incorrect|invalid|expired/i);
  assert.equal(h.storage.pairingStartedAt, undefined);
});
