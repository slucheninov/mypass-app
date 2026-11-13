// Hostname check used before a fill is delivered (service worker) and again
// before a content script writes into the page. One copy: the two used to
// drift in how they rejected an empty URL, and a mismatch here is a credential
// written into the wrong document.
//
// Either argument missing, or either argument not a URL, is a non-match.
// The page must be https: the app offers credentials only on https pages (AGENTS.md, Domain
// matching), so a same-host https -> http navigation between the request and the fill must not
// pass, or the credential would be written into an http document. The message URL must carry
// the same scheme as the page, so http on both sides is refused too. The hostname is compared
// case-insensitively; the port is still ignored.
//
// background-loader.js loads this with importScripts before background.js.
// content.js reaches it because manifest.json lists this file in the content
// script. Node tests require() it.

function hostnameMatches(messageUrl, pageUrl) {
  if (!messageUrl || !pageUrl) return false;
  try {
    const message = new URL(messageUrl);
    const page = new URL(pageUrl);
    if (page.protocol !== 'https:' || message.protocol !== page.protocol) return false;
    return message.hostname.toLowerCase() === page.hostname.toLowerCase();
  } catch (e) {
    return false;
  }
}

// Bare name for later content scripts. Do not read this off globalThis:
// in a content script that object is the page, not this extension.
var mypassUrlMatch = { hostnameMatches };

if (typeof module !== 'undefined') {
  module.exports = { hostnameMatches };
}
