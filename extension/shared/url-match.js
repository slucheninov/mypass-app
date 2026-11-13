// Hostname check used before a fill is delivered (service worker) and again
// before a content script writes into the page. One copy: the two used to
// drift in how they rejected an empty URL, and a mismatch here is a credential
// written into the wrong document.
//
// Either argument missing, or either argument not a URL, is a non-match.
// Comparison is hostname only — scheme and port are ignored, on purpose.
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

// Bare name for later content scripts. Do not read this off globalThis:
// in a content script that object is the page, not this extension.
var mypassUrlMatch = { hostnameMatches };

if (typeof module !== 'undefined') {
  module.exports = { hostnameMatches };
}
