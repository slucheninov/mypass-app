const assert = require('node:assert/strict');
const { test } = require('node:test');
const { openPopup } = require('./popupHarness.js');

test('the locked panel runs one poll and clears the password and timer when leaving', async () => {
  let reply = { type: 'locked' };
  const h = openPopup({ reply: (message) => message.type === 'getCredentials' ? reply : {} });
  assert.equal(h.el('locked-panel').hidden, false);
  assert.equal(h.el('browser-chrome').hidden, true);
  assert.equal(h.el('add-totp-section').hidden, true);
  assert.equal(h.api.LOCKED_POLL_INTERVAL_MS, 2000);
  assert.equal(h.timers.size, 1);
  await h.advance(4000);
  assert.equal(h.timers.size, 1);
  assert.equal(h.sent.filter((message) => message.type === 'getCredentials').length, 3);
  h.el('unlock-password').value = 'typed';
  reply = { type: 'credentialList', credentials: [{ id: '1', title: 'Example', username: 'a' }] };
  await h.advance(2000);
  assert.equal(h.el('list-panel').hidden, false);
  assert.equal(h.el('locked-panel').hidden, true);
  assert.equal(h.el('unlock-password').value, '');
  assert.equal(h.timers.size, 0);
});

test('pagehide clears the password and stops the locked poll', () => {
  const h = openPopup({ reply: (message) => message.type === 'getCredentials' ? { type: 'locked' } : {} });
  h.el('unlock-password').value = 'typed';
  h.listeners.pagehide();
  assert.equal(h.el('unlock-password').value, '');
  assert.equal(h.timers.size, 0);
});
