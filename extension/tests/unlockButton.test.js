const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const popup = require('../popup.js');

// Visibility is driven purely by whether this browser holds a pairing secret.
assert.strictEqual(popup.shouldShowUnlockButton('some-secret'), true);
assert.strictEqual(popup.shouldShowUnlockButton(''), false);
assert.strictEqual(popup.shouldShowUnlockButton(null), false);
assert.strictEqual(popup.shouldShowUnlockButton(undefined), false);

// Response classification drives what the popup does next.
assert.strictEqual(popup.classifyRequestUnlockResponse({ type: 'unlockPresented' }), 'presented');
assert.strictEqual(popup.classifyRequestUnlockResponse({ type: 'unlockNotNeeded' }), 'notNeeded');
assert.strictEqual(popup.classifyRequestUnlockResponse({ type: 'pairingRequired' }), 'unavailable');
assert.strictEqual(popup.classifyRequestUnlockResponse({ type: 'error' }), 'unavailable');
assert.strictEqual(popup.classifyRequestUnlockResponse(null), 'unavailable');
assert.strictEqual(popup.classifyRequestUnlockResponse(undefined), 'unavailable');

assert.strictEqual(typeof popup.UNLOCK_BUTTON_LABEL, 'string');
assert.ok(popup.UNLOCK_BUTTON_LABEL.length > 0);

// The popup reads the pairing secret straight from storage, so its key must match the
// one background.js writes. These are separate files with no shared module, so assert
// the literal rather than trusting a comment.
const backgroundSource = fs.readFileSync(path.join(__dirname, '..', 'background.js'), 'utf8');
assert.ok(
  backgroundSource.includes(`"${popup.POPUP_PAIRING_STORAGE_KEY}"`) ||
    backgroundSource.includes(`'${popup.POPUP_PAIRING_STORAGE_KEY}'`),
  'popup.js and background.js must agree on the pairing-secret storage key'
);

// requestUnlock must be relayed, not dropped as an unknown type.
assert.ok(backgroundSource.includes('"requestUnlock"'), 'background.js knownTypes must include requestUnlock');

// The button must exist in the markup with the id popup.js wires up.
const popupHtml = fs.readFileSync(path.join(__dirname, '..', 'popup.html'), 'utf8');
assert.ok(popupHtml.includes('id="locked-unlock-btn"'), 'locked panel must contain the unlock button');

// --- Polling after unlockPresented -------------------------------------------------
// The design originally assumed the popup closes when MyPass takes focus, so no waiting
// state was needed. In practice the popup stays open showing a stale locked panel, so it
// polls until the vault reports unlocked.

assert.strictEqual(typeof popup.UNLOCK_POLL_INTERVAL_MS, 'number');
assert.ok(popup.UNLOCK_POLL_INTERVAL_MS > 0);
assert.strictEqual(typeof popup.UNLOCK_POLL_MAX_ATTEMPTS, 'number');
assert.ok(popup.UNLOCK_POLL_MAX_ATTEMPTS > 0);
assert.strictEqual(typeof popup.WAITING_FOR_UNLOCK_TEXT, 'string');
assert.ok(popup.WAITING_FOR_UNLOCK_TEXT.length > 0);

// Still locked and attempts remain -> keep waiting.
assert.strictEqual(popup.shouldContinueUnlockPolling('locked', 0), true);
assert.strictEqual(popup.shouldContinueUnlockPolling('locked', popup.UNLOCK_POLL_MAX_ATTEMPTS - 1), true);

// Budget exhausted -> give up rather than poll forever.
assert.strictEqual(popup.shouldContinueUnlockPolling('locked', popup.UNLOCK_POLL_MAX_ATTEMPTS), false);

// Any non-locked outcome ends the poll immediately, including failures — otherwise a
// vault that never unlocks or an app that quit would spin for the full budget.
assert.strictEqual(popup.shouldContinueUnlockPolling('credentialList', 0), false);
assert.strictEqual(popup.shouldContinueUnlockPolling('pairingRequired', 0), false);
assert.strictEqual(popup.shouldContinueUnlockPolling('error', 0), false);
assert.strictEqual(popup.shouldContinueUnlockPolling('unknown', 0), false);

// The whole budget must be long enough for a human to reach the app and use Touch ID.
assert.ok(
  popup.UNLOCK_POLL_INTERVAL_MS * popup.UNLOCK_POLL_MAX_ATTEMPTS >= 30000,
  'polling budget must span at least 30 s of real unlocking time'
);

console.log('unlockButton.test.js OK');
