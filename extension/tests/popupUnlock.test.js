const assert = require('node:assert/strict');
const { test } = require('node:test');
const { openPopup, html, flush } = require('./popupHarness.js');
const popup = require('../popup.js');

function lockedPopup({ password, device, options, browse } = {}) {
  return openPopup({ reply(message, cb) {
    switch (message.type) {
      case 'getCredentials': return browse ? browse() : { type: 'locked' };
      case 'getUnlockOptions': return { type: 'unlockOptions', ...options };
      case 'unlockWithPassword': return password?.(cb);
      case 'unlockWithDevice': return device?.();
      case 'getPrompt': return { type: 'promptUnavailable' };
      default: return {};
    }
  } });
}

function submit(h, password = 'example') {
  h.el('unlock-password').value = password;
  h.fire('unlock-form', 'submit');
}

test('lockedPanelModel covers every mode', () => {
  for (const [mode, title, form, action, helper] of [
    ['locked', 'MyPass is locked', true, null, null],
    ['starting', 'Starting MyPass…', false, null, null],
    ['notRunning', "MyPass isn't running", false, 'launch', 'launchFailed'],
    ['hostMissing', "MyPass isn't set up for this browser", false, null, 'hostUnavailableStatus'],
    ['inApp', 'MyPass is locked', false, 'openApp', 'unlockInApp'],
  ]) {
    const model = popup.lockedPanelModel(mode, { deviceUnlock: true, touchId: true, launchFailed: true });
    assert.equal(model.title, title);
    assert.equal(model.showForm, form);
    assert.equal(model.showDevice, form);
    assert.equal(model.launchAction, action);
    assert.equal(model.helperKey, helper);
  }
});

test('submit clears the password input before the reply', async () => {
  let answer;
  const h = lockedPopup({ password: (cb) => { answer = cb; } });
  await flush();
  submit(h);
  assert.equal(h.el('unlock-password').value, '');
  assert.equal(h.el('unlock-submit-btn').disabled, true);
  assert.deepEqual(h.sent.filter((m) => m.type === 'unlockWithPassword'), [{ type: 'unlockWithPassword', password: 'example' }]);
  answer({ type: 'wrongPassword' });
  await flush();
});

test('wrong password shows an inline error and keeps the form focused', async () => {
  const h = lockedPopup({ password: () => ({ type: 'wrongPassword' }) });
  await flush();
  submit(h);
  await flush();
  assert.equal(h.el('unlock-form').hidden, false);
  assert.equal(h.el('unlock-error').textContent, 'Incorrect password.');
  assert.equal(h.focused(), 'unlock-password');
});

test('a throttle from the reply or options disables submit until the backoff ends', async () => {
  for (const fromOptions of [false, true]) {
    const h = lockedPopup({
      options: fromOptions ? { retryAfterSecs: 2 } : {},
      password: () => ({ type: 'unlockThrottled', retryAfterSecs: 2 }),
    });
    await flush();
    if (!fromOptions) submit(h);
    await flush();
    assert.equal(h.el('unlock-submit-btn').disabled, true);
    assert.match(h.el('unlock-error').textContent, /2 s/);
    await h.advance(1000);
    assert.match(h.el('unlock-error').textContent, /1 s/);
    await h.advance(1000);
    assert.equal(h.el('unlock-submit-btn').disabled, false);
    assert.equal(h.el('unlock-error').hidden, true);
  }
});

test('unlocked reloads the current browse context', async () => {
  let unlocked = false;
  const h = lockedPopup({
    browse: () => unlocked ? { type: 'credentialList', credentials: [] } : { type: 'locked' },
    password: () => { unlocked = true; return { type: 'unlocked' }; },
  });
  await flush();
  submit(h);
  await flush();
  assert.equal(h.el('empty-panel').hidden, false);
  assert.equal(h.el('locked-panel').hidden, true);
  assert.ok(h.sent.filter((m) => m.type === 'getCredentials').every((m) => m.url === 'https://example.com/'));
});

test('unlockInApp shows Open MyPass and keeps it on a subsequent poll', async () => {
  const h = lockedPopup({ password: () => ({ type: 'unlockInApp' }) });
  await flush();
  submit(h);
  await h.advance(2000);
  assert.equal(h.el('unlock-form').hidden, true);
  assert.equal(h.el('launch-app-btn').hidden, false);
  assert.match(h.el('locked-helper').textContent, /Open MyPass to unlock/);
  h.fire('launch-app-btn', 'click');
  assert.equal(h.sent.at(-1).type, 'openApp');
});

test('locked poll tick keeps typed password, error, reveal and focus', async () => {
  const h = lockedPopup({ password: () => ({ type: 'wrongPassword' }) });
  await flush();
  submit(h);
  await flush();
  h.el('unlock-password').value = 'abc';
  h.fire('unlock-reveal-btn', 'click');
  await h.advance(2000);
  assert.equal(h.el('unlock-password').value, 'abc');
  assert.equal(h.el('unlock-password').type, 'text');
  assert.equal(h.el('unlock-error').textContent, 'Incorrect password.');
  assert.equal(h.focused(), 'unlock-password');
  assert.equal(h.sent.filter((m) => m.type === 'getUnlockOptions').length, 1);
});

test('device button only appears when offered and uses the correct label', async () => {
  for (const [options, label] of [
    [{ deviceUnlock: false, touchId: true }, null],
    [{ deviceUnlock: true, touchId: true }, 'Unlock with Touch ID'],
    [{ deviceUnlock: true, touchId: false }, 'Unlock with system password'],
  ]) {
    const h = lockedPopup({ options });
    await flush();
    assert.equal(h.el('unlock-device-btn').hidden, label === null);
    if (label) assert.equal(h.el('unlock-device-btn').textContent, label);
  }
});

test('cancelled device unlock is silent and an unavailable device hides the button', async () => {
  let response = { type: 'unlockCancelled' };
  const h = lockedPopup({
    options: { deviceUnlock: true, touchId: true },
    device: () => response,
  });
  await flush();
  h.fire('unlock-device-btn', 'click');
  await flush();
  assert.equal(h.el('unlock-error').hidden, true);
  assert.equal(h.el('unlock-device-btn').hidden, false);
  response = { type: 'deviceUnlockUnavailable' };
  h.fire('unlock-device-btn', 'click');
  await flush();
  assert.equal(h.el('unlock-device-btn').hidden, true);
  assert.match(h.el('unlock-error').textContent, /Use your master password/);
});

test('reveal toggles the password input type and pressed state', async () => {
  const h = lockedPopup();
  await flush();
  for (const [type, pressed] of [['text', 'true'], ['password', 'false']]) {
    h.fire('unlock-reveal-btn', 'click');
    assert.equal(h.el('unlock-password').type, type);
    assert.equal(h.el('unlock-reveal-btn').getAttribute('aria-pressed'), pressed);
  }
});

function stoppedPopup(browseReply) {
  return openPopup({ reply(message) {
    if (message.type === 'launchApp') return { type: 'launchStarted' };
    if (message.type === 'getCredentials') return browseReply();
    return {};
  } });
}

test('app_not_running starts one hidden launch and polls until locked', async () => {
  let attempts = 0;
  const h = stoppedPopup(() => ++attempts <= 4 ? { type: 'error', message: 'app_not_running' } : { type: 'locked' });
  assert.equal(h.el('unlock-form').hidden, true);
  await h.advance(2000);
  assert.equal(attempts, 5);
  assert.equal(h.el('unlock-form').hidden, false);
  assert.equal(h.sent.filter((m) => m.type === 'launchApp').length, 1);
});

test('launch timeout exposes a retry that starts a new launch', async () => {
  const h = stoppedPopup(() => ({ type: 'error', code: 'connection_unavailable' }));
  await h.advance(15_000);
  assert.equal(h.sent.filter((m) => m.type === 'getCredentials').length, 31);
  assert.equal(h.el('locked-title').textContent, "MyPass isn't running");
  assert.equal(h.el('launch-app-btn').hidden, false);
  h.fire('launch-app-btn', 'click');
  assert.equal(h.sent.filter((m) => m.type === 'launchApp').length, 2);
});

test('native_host_not_found shows setup help without launching', async () => {
  const h = stoppedPopup(() => ({ type: 'error', code: 'native_host_not_found' }));
  await flush();
  assert.equal(h.el('locked-title').textContent, "MyPass isn't set up for this browser");
  assert.equal(h.el('launch-app-btn').hidden, true);
  assert.equal(h.sent.filter((m) => m.type === 'launchApp').length, 0);
});

test('the locked panel has neither a Retry nor a more button', () => {
  const panel = html.match(/<section id="locked-panel"[\s\S]*?<\/section>/)?.[0];
  assert.ok(panel);
  assert.doesNotMatch(panel, /id="(?:retry-btn|popup-menu-btn)"/);
  assert.match(panel, /<form id="unlock-form"/);
});
