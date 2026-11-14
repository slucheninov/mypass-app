// Service worker entry. importScripts has to be the first thing this file
// does, as a direct call. Chrome rejects importScripts that runs after other
// startup work ("importScripts() of new scripts after service worker
// installation is not allowed"), and that abort happens before
// background.js registers chrome.runtime.onMessage. The popup then gets no
// reply and shows "MyPass not found" even while the app is running.
//
// Node tests require background.js directly and have no importScripts, so
// the shared helper is loaded there with require().
importScripts('shared/url-match.js');
importScripts('shared/password-generator.js');
importScripts('shared/security-watch.js');
importScripts('shared/notification-settings.js');
importScripts('shared/autofill-settings.js');
importScripts('shared/prompt-record.js');
importScripts('background.js');
