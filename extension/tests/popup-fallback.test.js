const assert = require('node:assert/strict');
const { test } = require('node:test');
const { openPopup, flush } = require('./popupHarness.js');

test('connection errors launch once and a reachable app renders the requested list', async () => {
  let stopped = true;
  const h = openPopup({ reply(message) {
    if (message.type === 'launchApp') {
      stopped = false;
      return { type: 'launchNotNeeded' };
    }
    if (message.type === 'getCredentials') return stopped
      ? { type: 'error', message: 'app_not_running' }
      : { type: 'credentialList', credentials: [{ id: '1', title: 'Example', username: 'alice@example.com' }] };
    return {};
  } });
  assert.equal(h.el('locked-title').textContent, 'Starting MyPass…');
  await h.advance(500);
  assert.equal(h.el('list-panel').hidden, false);
  assert.equal(h.el('list').children.length, 1);
  assert.equal(h.el('locked-panel').hidden, true);
  assert.equal(h.sent.filter((message) => message.type === 'launchApp').length, 1);
});

test('an application error does not launch automatically', () => {
  const h = openPopup({ reply: (message) => message.type === 'getCredentials'
    ? { type: 'error', message: 'MyPass is quitting' } : {} });
  assert.equal(h.el('launch-app-btn').hidden, false);
  assert.equal(h.sent.filter((message) => message.type === 'launchApp').length, 0);
});

test('a failed launch keeps a retry visible without polling', async () => {
  const h = openPopup({ reply(message) {
    if (message.type === 'launchApp') return { type: 'error', message: 'launch_unavailable' };
    if (message.type === 'getCredentials') return { type: 'error', message: 'app_not_running' };
    return {};
  } });
  await flush();
  assert.match(h.el('locked-helper').textContent, /couldn't be started/);
  assert.equal(h.el('launch-app-btn').hidden, false);
  assert.equal(h.timers.size, 0);
});
