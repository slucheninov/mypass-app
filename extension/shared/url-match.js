// Two URL checks, one copy for the service worker and the content script (the two used to drift in
// how they rejected an empty URL, and a mismatch here is a credential written into the wrong
// document).
//
// hostnameMatches: "is this message about the sender's own host". Hostname only, scheme and port
// ignored. Used by warning and detection paths (the paste/copy security warning), which must keep
// working on plain-http pages: those are exactly where a phishing page lives.
//
// fillTargetMatches: the check in front of anything that writes a credential or offers a credential
// flow (fill delivery, the content-script write, the pending open-fill, the save and 2FA
// prompts). The page must be https: the app offers credentials only on https pages (AGENTS.md,
// Domain matching), so a same-host https -> http navigation between the request and the fill
// must not pass, and the save and 2FA prompts are refused on http pages for the same reason. The
// message URL must carry the same scheme as the page, so http on both sides is refused too. The
// port is ignored.
//
// For both, either argument missing, or either argument not a URL, is a non-match, and the
// hostname compares case-insensitively.
//
// background-loader.js loads this with importScripts before background.js.
// content.js reaches it because manifest.json lists this file in the content
// script. Node tests require() it.

function hostnameMatches(messageUrl, pageUrl) {
  if (!messageUrl || !pageUrl) return false;
  try {
    const messageHost = new URL(messageUrl).hostname.toLowerCase();
    const pageHost = new URL(pageUrl).hostname.toLowerCase();
    return messageHost === pageHost;
  } catch (e) {
    return false;
  }
}

function fillTargetMatches(messageUrl, pageUrl) {
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
var mypassUrlMatch = { hostnameMatches, fillTargetMatches };

if (typeof module !== 'undefined') {
  module.exports = { hostnameMatches, fillTargetMatches };
}
