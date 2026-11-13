// A small, manually maintained UI hint catalog, not a WebAuthn capability check.
// Never use this list to authorize a ceremony, match an RP ID, or release a
// credential. Those decisions remain in the app's existing security checks.
// Exact HTTPS hosts avoid implying support on unrelated or tenant subdomains.
// Nothing is fetched at runtime, so checking a hint does not disclose browsing.
var mypassPasskeySites = (() => {
  const sites = [
    {
      name: 'GitHub',
      hosts: ['github.com'],
      source: 'https://docs.github.com/en/authentication/authenticating-with-a-passkey/signing-in-with-a-passkey',
      verifiedOn: '2026-10-11',
      eligibility: 'Personal accounts that manage their own credentials.'
    },
    {
      name: 'Google',
      hosts: ['accounts.google.com', 'myaccount.google.com'],
      source: 'https://support.google.com/accounts/answer/13548313?hl=en',
      verifiedOn: '2026-10-11',
      eligibility: 'Account, organization and device requirements can apply.'
    },
    {
      name: 'Microsoft personal accounts',
      hosts: ['login.live.com', 'account.live.com', 'account.microsoft.com'],
      source: 'https://support.microsoft.com/en-us/accounts-billing/security/create-save-passkey',
      verifiedOn: '2026-10-11',
      eligibility: 'Personal account sign-in; work and school policies are not inferred.'
    },
    {
      name: 'PayPal',
      hosts: ['paypal.com', 'www.paypal.com'],
      source: 'https://www.paypal.com/us/cshelp/article/what-is-a-passkey-and-how-do-i-use-it-to-log-in-to-my-paypal-account-help997',
      verifiedOn: '2026-10-11',
      eligibility: 'Eligible accounts and supported devices/browsers; availability can vary.'
    },
    {
      name: 'Dropbox',
      hosts: ['dropbox.com', 'www.dropbox.com'],
      source: 'https://help.dropbox.com/account-access/passkey-log-in',
      verifiedOn: '2026-10-11',
      eligibility: 'Supported plans; team administrator and SSO restrictions can apply.'
    }
  ].map((site) => Object.freeze({ ...site, hosts: Object.freeze(site.hosts) }));
  const hosts = new Set(sites.flatMap((site) => site.hosts));

  function hasSiteHint(url, matchedCredentials) {
    if (typeof url !== 'string') return false;
    let parsed;
    try { parsed = new URL(url); } catch (_) { return false; }
    if (parsed.protocol !== 'https:') return false;
    // Only the app's response to getCredentials for THIS URL belongs here.
    // Favorites, search and category rows must not imply support for this site.
    return hosts.has(parsed.hostname)
      || (Array.isArray(matchedCredentials) && matchedCredentials.some((entry) => entry && entry.hasPasskey === true));
  }

  return Object.freeze({ sites: Object.freeze(sites), hasSiteHint });
})();

if (typeof module !== 'undefined') module.exports = mypassPasskeySites;
