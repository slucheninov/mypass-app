// The popup's "Signed in with ... last time" row above the site's logins.
const assert = require('node:assert/strict');
const { openPopup } = require('./popupHarness.js');

const logins = [{ id: 'shop', title: 'shop.example', username: '', hasTotp: false, hasPasskey: false, identifiers: [] }];

function siteReply(social) {
  return (message) => {
    if (message.type === 'getCredentials') return { type: 'credentialList', credentials: logins };
    if (message.type === 'getSocialSignIn') return social;
    return { type: 'ok' };
  };
}

(async () => {
  {
    const popup = openPopup({ reply: siteReply({ type: 'socialSignIn', enabled: true, provider: 'github', lastUsed: '2026-11-20T09:30:00Z' }) });
    await popup.advance(0);
    const lookup = popup.sent.find((m) => m.type === 'getSocialSignIn');
    assert.equal(lookup.url, 'https://example.com/');
    assert.equal(popup.el('site-social-sign-in').hidden, false);
    assert.equal(popup.el('site-social-sign-in-text').textContent, 'Signed in with GitHub last time');
    assert.equal(popup.el('site-social-sign-in-mark').dataset.provider, 'github');
    assert.equal(popup.el('site-social-sign-in-mark').textContent, 'G');
  }
  for (const social of [
    { type: 'socialSignIn', enabled: true },
    { type: 'socialSignIn', enabled: false, provider: 'google' },
    { type: 'socialSignIn', enabled: true, provider: 'myspace' },
    { type: 'locked' },
  ]) {
    const popup = openPopup({ reply: siteReply(social) });
    await popup.advance(0);
    assert.equal(popup.el('site-social-sign-in').hidden, true, JSON.stringify(social));
  }
  console.log('socialSignInPopup.test.js: popup row OK');
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
