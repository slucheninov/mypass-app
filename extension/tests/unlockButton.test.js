const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { html } = require('./popupHarness.js');
const { classifyLaunchResponse, LAUNCH_POLL_INTERVAL_MS, LAUNCH_POLL_MAX_ATTEMPTS } = require('../popup.js');

assert.match(html, /id="unlock-password" type="password" autocomplete="current-password"/);
assert.match(html, /id="unlock-reveal-btn" type="button"[^>]*aria-pressed="false"/);
assert.match(html, /id="unlock-submit-btn" type="submit"/);
assert.match(html, /id="unlock-error" role="alert"/);
assert.doesNotMatch(html, /id="(?:locked-unlock-btn|retry-btn|app-unavailable-panel)"/);

const background = fs.readFileSync(path.join(__dirname, '..', 'background.js'), 'utf8');
// Content-script users can still ask the desktop app to raise its unlock screen.
assert.match(background, /"requestUnlock"/);

assert.equal(LAUNCH_POLL_INTERVAL_MS, 500);
assert.equal(LAUNCH_POLL_MAX_ATTEMPTS, 30);
assert.equal(classifyLaunchResponse({ type: 'launchStarted' }), 'started');
assert.equal(classifyLaunchResponse({ type: 'launchNotNeeded' }), 'started');
assert.equal(classifyLaunchResponse({ type: 'error', message: 'launch_unavailable' }), 'unavailable');
assert.equal(classifyLaunchResponse({ type: 'error', message: 'launch_failed' }), 'failed');
assert.equal(classifyLaunchResponse(null), 'failed');
