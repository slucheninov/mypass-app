// Test-only support code, not a suite (the runner only picks up *.test.js).
// `install(chrome)` adds a chrome.i18n backed by the English catalog. Suites that evaluate classic
// scripts inside a vm context need it: there is no `require` in such a context, so
// shared/i18n.js has no on-disk fallback and would answer with the bare key.
const path = require('node:path');
const i18n = require('../shared/i18n.js');
const catalog = require(path.join(__dirname, '..', '_locales', 'en', 'messages.json'));

function install(chromeObject) {
  chromeObject.i18n = {
    getMessage: (key, substitutions) => (catalog[key] ? i18n.expand(catalog[key], substitutions) : ''),
    getUILanguage: () => 'en'
  };
  return chromeObject;
}

module.exports = { install };
