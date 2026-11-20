// Localisation helper shared by the popup, the extension pages, the in-page panel and the
// service worker. Every user-visible string lives in `_locales/<lang>/messages.json` (the
// standard chrome.i18n layout, `en` is the manifest's default_locale) and is read through
// `t(key, substitutions)`.
//
// Pages mark static markup with `data-i18n` (text), `data-i18n-placeholder`, `data-i18n-title`
// and `data-i18n-aria-label`; `apply(document)` fills them. The English text is deliberately
// not repeated in the HTML: the catalog is the only copy.
//
// Node (the extension test suites) has no chrome.i18n. There `t` falls back to the English
// catalog read from disk, with the same `$NAME$` placeholder expansion Chrome performs, so
// suites keep asserting the English wording. A browser never takes that branch: `require` does
// not exist in a page, a content script or the service worker.
//
// This file declares exactly one top-level name (`mypassI18n`): it is loaded as a classic
// script next to many others (see tests/scriptScopes.test.js).

var mypassI18n = (function () {
  var nodeCatalog = null;

  function chromeI18n() {
    return typeof chrome !== 'undefined' && chrome && chrome.i18n
      && typeof chrome.i18n.getMessage === 'function' ? chrome.i18n : null;
  }

  function loadNodeCatalog() {
    if (nodeCatalog) return nodeCatalog;
    nodeCatalog = {};
    if (typeof require === 'function') {
      try {
        var fs = require('node:fs');
        var path = require('node:path');
        var file = path.join(__dirname, '..', '_locales', 'en', 'messages.json');
        nodeCatalog = JSON.parse(fs.readFileSync(file, 'utf8'));
      } catch (error) {
        nodeCatalog = {};
      }
    }
    return nodeCatalog;
  }

  // Chrome's own expansion: `$$` is a literal dollar, `$NAME$` is the placeholder whose content
  // is `$1`..`$9`, the position of the substitution passed to getMessage.
  function expand(entry, substitutions) {
    var list = substitutions === undefined || substitutions === null ? []
      : Array.isArray(substitutions) ? substitutions : [substitutions];
    var placeholders = entry.placeholders || {};
    var byName = {};
    Object.keys(placeholders).forEach(function (name) {
      byName[name.toLowerCase()] = placeholders[name] && placeholders[name].content;
    });
    return String(entry.message).replace(/\$([A-Za-z0-9_@]+)\$|\$\$/g, function (match, name) {
      if (match === '$$') return '$';
      var content = byName[name.toLowerCase()];
      if (typeof content !== 'string') return match;
      return content.replace(/\$([1-9])/g, function (_m, index) {
        var value = list[Number(index) - 1];
        return value === undefined ? '' : String(value);
      });
    });
  }

  function t(key, substitutions) {
    var api = chromeI18n();
    if (api) {
      var list = substitutions === undefined || substitutions === null ? undefined
        : (Array.isArray(substitutions) ? substitutions : [substitutions]).map(String);
      var message = api.getMessage(key, list);
      if (message) return message;
    }
    var entry = loadNodeCatalog()[key];
    return entry ? expand(entry, substitutions) : key;
  }

  var ATTRIBUTES = [
    ['data-i18n-placeholder', 'placeholder'],
    ['data-i18n-title', 'title'],
    ['data-i18n-aria-label', 'aria-label']
  ];

  // Fills every marked element under `root`. Extension pages only: never call it on a web page.
  function apply(root) {
    if (!root || typeof root.querySelectorAll !== 'function') return;
    var api = chromeI18n();
    var doc = root.documentElement ? root : root.ownerDocument;
    if (api && doc && doc.documentElement && typeof api.getUILanguage === 'function') {
      doc.documentElement.lang = api.getUILanguage();
    }
    root.querySelectorAll('[data-i18n]').forEach(function (node) {
      node.textContent = t(node.getAttribute('data-i18n'));
    });
    ATTRIBUTES.forEach(function (pair) {
      root.querySelectorAll('[' + pair[0] + ']').forEach(function (node) {
        node.setAttribute(pair[1], t(node.getAttribute(pair[0])));
      });
    });
  }

  return { t: t, apply: apply, expand: expand };
})();

if (typeof module !== 'undefined') {
  module.exports = mypassI18n;
}
