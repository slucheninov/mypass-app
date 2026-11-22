// The 2FA offer appears only where a page really asks to set up an authenticator. A car
// rental's booking page (goldcar.es) produced "OUTDATEDINTERNET" as a setup key: the page's
// whole textContent was scanned, scripts, styles and a hidden outdated-browser banner
// included, and the weak words "manual" (a gearbox, CSS `hyphens: manual`) and "secret"
// (an OAuth `clientSecret` in a script) counted as setup context.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { parseHtml } = require('./miniDom.js');
const {
  detectTotpCandidateWithPriority, detectManualSetupKey, detectOtpauthText,
} = require('../shared/totp-detection.js');

const BOOKING_PAGE = '<style>.title { hyphens: manual; } .x { content: "secret key"; }</style>'
  + '<script>window.config = { clientSecret: "OUTDATEDINTERNET", flow: "two-factor", '
  + 'uri: "otpauth://totp/Goldcar:test?secret=JBSWY3DPEHPK3PXP" };</script>'
  + '<div id="outdated" hidden><p>Your browser is out of date</p><p>OUTDATEDINTERNET</p></div>'
  + '<div style="display:none">Authenticator setup key JBSW Y3DP EHPK 3PXP</div>'
  + '<header><div class="step">RENTAL LOCATION</div></header>'
  + '<section><h2>Your car</h2><p>Manual transmission, 5 doors, A/C</p>'
  + '<p>Secret deals for members: OUTDATEDINTERNETOFFERS</p></section>'
  + '<section><h2>Driver details</h2><label for="n">First name*</label><input id="n"></section>';

test('a booking page with scripts, styles and hidden banners offers no 2FA save', () => {
  const doc = parseHtml(BOOKING_PAGE);
  assert.equal(detectManualSetupKey(doc), null);
  assert.equal(detectOtpauthText(doc), null, 'an otpauth URL inside a script is not shown text');
  assert.equal(detectTotpCandidateWithPriority(doc), null);
});

const SETUP_PAGE = '<main><h1>Set up two-factor authentication</h1>'
  + '<ol><li><p>Scan the QR code with your authenticator app.</p></li>'
  + '<li><div class="card"><p>Can\'t scan it? Enter this setup key instead:</p>'
  + '<div class="key"><code>JBSW Y3DP EHPK 3PXP</code></div></div></li>'
  + '<li><label for="otp">Enter the 6-digit code</label>'
  + '<input id="otp" autocomplete="one-time-code" inputmode="numeric"></li></ol></main>';

test('a real authenticator setup page offers its setup key', () => {
  const doc = parseHtml(SETUP_PAGE);
  assert.deepEqual(detectTotpCandidateWithPriority(doc), { candidate: 'JBSWY3DPEHPK3PXP', priority: 4 });
});

test('a visible otpauth URL is offered, and beats a setup key', () => {
  const doc = parseHtml(SETUP_PAGE.replace('<code>JBSW Y3DP EHPK 3PXP</code>',
    '<code>otpauth://totp/Example:alice?secret=JBSWY3DPEHPK3PXP</code>'));
  assert.equal(detectTotpCandidateWithPriority(doc).priority, 2);
});

test('2FA wording elsewhere on the page does not vouch for a random-looking string', () => {
  const doc = parseHtml('<main><section><h2>Security</h2><p>Use an authenticator app for two-factor sign-in.</p></section>'
    + '<section><div><div><div><div><div><p>Order reference</p><p>K7QW M2ZD 4RXA P6TB</p></div></div></div></div></div></section></main>');
  assert.equal(detectManualSetupKey(doc), null);
});

test('a lone all-letter word is never a key, even with 2FA wording around it', () => {
  const doc = parseHtml('<main><h1>Two-factor authentication</h1><p>Secret key: OUTDATEDINTERNET</p></main>');
  assert.equal(detectManualSetupKey(doc), null);
  const real = parseHtml('<main><h1>Two-factor authentication</h1><p>Secret key: OUTDATED2NTERNET</p></main>');
  assert.equal(detectManualSetupKey(real), 'OUTDATED2NTERNET');
});

test('the setup context is recognised in German, Ukrainian and Russian', () => {
  const contexts = [
    'Zwei-Faktor-Authentifizierung einrichten: geben Sie diesen Schlüssel ein',
    '\u0414\u0432\u043e\u0444\u0430\u043a\u0442\u043e\u0440\u043d\u0430 \u0430\u0432\u0442\u0435\u043d\u0442\u0438\u0444\u0456\u043a\u0430\u0446\u0456\u044f: \u0441\u0435\u043a\u0440\u0435\u0442\u043d\u0438\u0439 \u043a\u043b\u044e\u0447',
    '\u0414\u0432\u0443\u0445\u0444\u0430\u043a\u0442\u043e\u0440\u043d\u0430\u044f \u0430\u0443\u0442\u0435\u043d\u0442\u0438\u0444\u0438\u043a\u0430\u0446\u0438\u044f: \u043a\u043b\u044e\u0447 \u043d\u0430\u0441\u0442\u0440\u043e\u0439\u043a\u0438',
  ];
  for (const context of contexts) {
    const doc = parseHtml(`<div><p>${context}</p><p>jbsw y3dp ehpk 3pxp</p></div>`);
    assert.equal(detectManualSetupKey(doc), 'JBSWY3DPEHPK3PXP', context);
  }
});
