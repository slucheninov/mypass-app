// Touch ID first in the popup: a paired popup over a locked vault with Touch ID offered shows the
// Touch ID view and starts one prompt by itself on opening. The per-lock-spell failure count comes
// from the bridge (`deviceFailures`), so it survives closing and reopening the popup.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { openPopup, flush } = require('./popupHarness.js');
const popup = require('../popup.js');

const TOUCH_ID = { deviceUnlock: true, touchId: true, deviceFailures: 0 };
const THREE_TIMES = "Touch ID didn't recognize your fingerprint 3 times. Enter your master password.";

function lockedPopup({ options = TOUCH_ID, device, browse } = {}) {
  return openPopup({ reply(message, cb) {
    switch (message.type) {
      case 'getCredentials': return browse ? browse() : { type: 'locked' };
      case 'getUnlockOptions': return { type: 'unlockOptions', ...options };
      case 'unlockWithDevice': return device?.(message, cb);
      case 'getPrompt': return { type: 'promptUnavailable' };
      default: return {};
    }
  } });
}

const devicePrompts = (h) => h.sent.filter((m) => m.type === 'unlockWithDevice');
const touchView = (h) => !h.el('touch-id-view').hidden;

function notRecognizedFrom(start = 0) {
  let count = start;
  return () => ({ type: 'deviceNotRecognized', deviceFailures: ++count });
}

test('lockedPanelModel puts Touch ID first only when Touch ID is offered', () => {
  assert.equal(popup.lockedPanelModel('locked', TOUCH_ID).touchFirst, true);
  assert.equal(popup.lockedPanelModel('locked', { deviceUnlock: true, touchId: false }).touchFirst, false);
  assert.equal(popup.lockedPanelModel('locked', { deviceUnlock: false, touchId: true }).touchFirst, false);
  assert.equal(popup.lockedPanelModel('inApp', TOUCH_ID).touchFirst, false);
});

test('opening the locked popup prompts once by itself and hides the password field', async () => {
  const h = lockedPopup({ device: () => undefined });
  await flush();
  assert.ok(touchView(h));
  assert.equal(h.el('unlock-form').hidden, true);
  assert.equal(h.el('touch-id-caption').textContent, 'Touch your Touch ID sensor');
  assert.equal(h.el('touch-id-btn').textContent, 'Unlock with Touch ID');
  assert.equal(h.el('use-password-btn').hidden, false);
  assert.deepEqual(devicePrompts(h), [{ type: 'unlockWithDevice', automatic: true }]);
  await h.advance(10_000);
  assert.equal(devicePrompts(h).length, 1, 'the locked poll never prompts again');
});

test('a successful automatic prompt unlocks and shows the list', async () => {
  let unlocked = false;
  const h = lockedPopup({
    browse: () => unlocked ? { type: 'credentialList', credentials: [] } : { type: 'locked' },
    device: () => { unlocked = true; return { type: 'unlocked' }; },
  });
  await flush();
  await flush();
  assert.equal(h.el('locked-panel').hidden, true);
  assert.equal(h.el('empty-panel').hidden, false);
});

test('three unrecognized prompts switch to the password form with the reason', async () => {
  const h = lockedPopup({ device: notRecognizedFrom() });
  await flush();
  assert.ok(touchView(h));
  assert.equal(h.el('touch-id-error').textContent, 'Not recognized. Try again.');
  h.fire('touch-id-btn', 'click');
  await flush();
  assert.ok(touchView(h));
  h.fire('touch-id-btn', 'click');
  await flush();
  assert.equal(devicePrompts(h).length, 3);
  assert.deepEqual(devicePrompts(h).slice(1), [{ type: 'unlockWithDevice' }, { type: 'unlockWithDevice' }]);
  assert.equal(touchView(h), false);
  assert.equal(h.el('unlock-form').hidden, false);
  assert.equal(h.el('unlock-error').textContent, THREE_TIMES);
  assert.equal(h.el('try-touch-id-btn').hidden, false);
  assert.equal(h.el('unlock-device-btn').hidden, true);
  assert.equal(h.focused(), 'unlock-password');
});

test('a reopened popup continues the lock spell count from the bridge', async () => {
  const second = lockedPopup({ options: { ...TOUCH_ID, deviceFailures: 2 }, device: notRecognizedFrom(2) });
  await flush();
  assert.equal(devicePrompts(second).length, 1);
  assert.equal(second.el('unlock-form').hidden, false);
  assert.equal(second.el('unlock-error').textContent, THREE_TIMES);

  const third = lockedPopup({ options: { ...TOUCH_ID, deviceFailures: 3 }, device: () => undefined });
  await flush();
  assert.equal(devicePrompts(third).length, 0, 'three failures in this lock spell: no automatic prompt');
  assert.equal(touchView(third), false);
  assert.equal(third.el('unlock-form').hidden, false);
  assert.equal(third.el('unlock-error').textContent, THREE_TIMES);
  assert.equal(third.el('try-touch-id-btn').hidden, false);
});

test('a cancel stays on the Touch ID view without another automatic prompt', async () => {
  const h = lockedPopup({ device: () => ({ type: 'unlockCancelled' }) });
  await flush();
  await h.advance(10_000);
  assert.equal(devicePrompts(h).length, 1);
  assert.ok(touchView(h));
  assert.equal(h.el('touch-id-error').hidden, true);
  h.fire('touch-id-btn', 'click');
  await flush();
  assert.equal(devicePrompts(h).length, 2, 'the button still prompts');
});

test('lockout or an unavailable device switches to the password form for the lock spell', async () => {
  const h = lockedPopup({ device: () => ({ type: 'deviceUnlockUnavailable' }) });
  await flush();
  assert.equal(touchView(h), false);
  assert.equal(h.el('unlock-form').hidden, false);
  assert.equal(h.el('unlock-error').textContent, "Touch ID isn't available. Use your master password.");
  assert.equal(h.el('try-touch-id-btn').hidden, true);
  assert.equal(h.el('unlock-device-btn').hidden, true);
  assert.equal(h.focused(), 'unlock-password');
});

test('a popup reopened during an in-flight prompt starts no second prompt and counts nothing', async () => {
  // The first popup closed when the system sheet took the focus; its request is still running.
  const first = lockedPopup({ device: () => undefined });
  await flush();
  assert.equal(devicePrompts(first).length, 1);
  // The bridge drops the second request without a prompt and answers it as cancelled.
  let unlocked = false;
  const second = lockedPopup({
    browse: () => unlocked ? { type: 'credentialList', credentials: [] } : { type: 'locked' },
    device: () => ({ type: 'unlockCancelled' }),
  });
  await flush();
  assert.equal(devicePrompts(second).length, 1);
  assert.ok(touchView(second));
  assert.equal(second.el('touch-id-error').hidden, true);
  // The first prompt completes in the app; the reopened popup follows on its next poll.
  unlocked = true;
  await second.advance(2000);
  assert.equal(second.el('empty-panel').hidden, false);
});

test('Use password shows the form without an error; Try Touch ID again prompts once', async () => {
  const h = lockedPopup({ device: () => ({ type: 'unlockCancelled' }) });
  await flush();
  h.fire('use-password-btn', 'click');
  assert.equal(touchView(h), false);
  assert.equal(h.el('unlock-form').hidden, false);
  assert.equal(h.el('unlock-error').hidden, true);
  assert.equal(h.focused(), 'unlock-password');
  assert.equal(devicePrompts(h).length, 1);
  h.fire('try-touch-id-btn', 'click');
  await flush();
  assert.ok(touchView(h));
  assert.equal(devicePrompts(h).length, 2);
  assert.deepEqual(devicePrompts(h)[1], { type: 'unlockWithDevice' });
});

test('without Touch ID the popup keeps the password form and never prompts by itself', async () => {
  for (const options of [{ deviceUnlock: true, touchId: false }, { deviceUnlock: false, touchId: true }, {}]) {
    const h = lockedPopup({ options, device: () => undefined });
    await flush();
    assert.equal(touchView(h), false);
    assert.equal(h.el('unlock-form').hidden, false);
    assert.equal(h.el('try-touch-id-btn').hidden, true);
    assert.equal(devicePrompts(h).length, 0);
  }
});
