// Pending in-browser prompts (save a login, save a 2FA seed, open settings). Pure functions over a
// plain array of records, so the rules are testable without chrome.* and the service worker can
// keep the array in chrome.storage.session.
//
// A record is created ONLY by the background, from a content-script message whose sender Chrome
// authenticated (tab, frame, document, url). It holds the secret (password / seed) and nothing
// else ever does: the popup and the prompt window only receive redactedView(), and a page can
// neither read nor write a record.
//
// One var bag. A top-level const or function here would share the classic-script scope with the
// other extension scripts and can throw at load.

var mypassPromptRecord = (function () {
  // Matches the pairing code and the passkey ceremony watchdog: long enough to read and click,
  // short enough that a forgotten secret does not sit in memory.
  var PROMPT_TTL_MS = 120000;
  var PROMPT_STORAGE_KEY = 'pendingPrompts';
  // Kinds that carry page data and must come from the top frame only (the former banners).
  var SECRET_KINDS = ['saveLogin', 'saveTotp'];
  var PROMPT_KINDS = ['saveLogin', 'saveTotp', 'settings'];
  var MAX_FIELD = 2048;
  var MAX_PASSWORD = 1024;
  // A registration form's extra values (registration-form.js `capture`), held to the bridge's
  // limits (session_ops.rs `checked_extras`): a list that would be refused there is refused here.
  var EXTRA_FIELD_KINDS = ['name', 'givenName', 'familyName', 'email', 'phone', 'birthday', 'username',
    'organization', 'addressLine1', 'addressLine2', 'city', 'region', 'postalCode', 'country', 'other'];
  var MAX_EXTRA_FIELDS = 24;
  var MAX_EXTRA_LABEL = 64;
  var MAX_EXTRA_VALUE = 512;
  var RESERVED_LABEL_PREFIXES = ['mypass.', 'kpex_', 'kp2a_'];
  var UNSAFE_CHAR = /[\p{Cc}\p{Cf}]/u;

  function isString(value, max) {
    return typeof value === 'string' && value.length <= max;
  }

  function isCleanLine(value, max) {
    return typeof value === 'string' && Array.from(value).length <= max && !UNSAFE_CHAR.test(value);
  }

  // The checked copy of `fields`, or null when the list or any field breaks a rule.
  function checkFields(fields) {
    if (!Array.isArray(fields) || fields.length > MAX_EXTRA_FIELDS) return null;
    var out = [];
    for (var i = 0; i < fields.length; i += 1) {
      var field = fields[i];
      if (!field || typeof field !== 'object' || EXTRA_FIELD_KINDS.indexOf(field.kind) === -1) return null;
      if (!isCleanLine(field.label, MAX_EXTRA_LABEL) || !isCleanLine(field.value, MAX_EXTRA_VALUE)) return null;
      var label = field.label.trim();
      var value = field.value.trim();
      var lowered = label.toLowerCase();
      if (!value || (field.kind === 'other' && !label)) return null;
      if (RESERVED_LABEL_PREFIXES.some(function (prefix) { return lowered.indexOf(prefix) === 0; })) return null;
      out.push({ kind: field.kind, label: label, value: value });
    }
    return out;
  }

  function newId() {
    if (typeof crypto !== 'undefined' && crypto && typeof crypto.randomUUID === 'function') {
      return crypto.randomUUID();
    }
    // Only reached in an environment without WebCrypto (an old test runner).
    return 'p' + Math.random().toString(36).slice(2) + Date.now().toString(36);
  }

  // Returns null when the request must not create a record. Every refusal here is a security
  // property: no documentId means the prompt could later be delivered to a different document
  // (the same refusal relayFillToActiveTab makes), and a sub-frame must never raise a save prompt.
  function create(kind, sender, payload, now) {
    if (PROMPT_KINDS.indexOf(kind) === -1) return null;
    if (!sender || !sender.tab || typeof sender.tab.id !== 'number') return null;
    if (typeof sender.documentId !== 'string' || !sender.documentId) return null;
    if (typeof sender.frameId !== 'number') return null;
    if (!isString(sender.url, MAX_FIELD) || !sender.url) return null;
    var secretKind = SECRET_KINDS.indexOf(kind) !== -1;
    if (secretKind && sender.frameId !== 0) return null;
    var origin;
    try {
      origin = new URL(sender.url).origin;
    } catch (e) {
      return null;
    }
    if (!/^https?:/.test(origin)) return null;

    var data = {};
    var input = payload && typeof payload === 'object' ? payload : {};
    if (kind === 'saveLogin') {
      if (!isString(input.password, MAX_PASSWORD) || !input.password) return null;
      if (input.username !== undefined && !isString(input.username, MAX_FIELD)) return null;
      data = { username: input.username || '', password: input.password };
      if (input.fields !== undefined) {
        var fields = checkFields(input.fields);
        if (!fields) return null;
        if (fields.length) data.fields = fields;
      }
      if (input.usernameIsEmail !== undefined && typeof input.usernameIsEmail !== 'boolean') return null;
      if (input.usernameIsEmail === true && data.username) data.usernameIsEmail = true;
    } else if (kind === 'saveTotp') {
      if (!isString(input.otpauthOrSecret, MAX_FIELD) || !input.otpauthOrSecret) return null;
      data = { otpauthOrSecret: input.otpauthOrSecret };
    }

    return {
      id: newId(),
      kind: kind,
      tabId: sender.tab.id,
      windowId: typeof sender.tab.windowId === 'number' ? sender.tab.windowId : undefined,
      frameId: sender.frameId,
      documentId: sender.documentId,
      origin: origin,
      pageUrl: sender.url,
      createdAt: now,
      expiresAt: now + PROMPT_TTL_MS,
      // Set when the prompt window (not the toolbar popup) was the surface used, so closing that
      // window can dismiss exactly what it was showing.
      surface: '',
      payload: data
    };
  }

  function isExpired(record, now) {
    return !record || typeof record.expiresAt !== 'number' || now >= record.expiresAt;
  }

  function normalizeList(list) {
    return Array.isArray(list) ? list.filter(function (r) { return r && typeof r.id === 'string'; }) : [];
  }

  function prune(list, now) {
    return normalizeList(list).filter(function (r) { return !isExpired(r, now); });
  }

  // One active prompt per (tab, kind): a newer request replaces the older one.
  function upsert(list, record) {
    return normalizeList(list)
      .filter(function (r) { return !(r.tabId === record.tabId && r.kind === record.kind); })
      .concat([record]);
  }

  // A classic form POST replaces the document that captured a login, so the exact-document
  // binding (documentId) alone would drop a real "Save this password?" the moment the site
  // answers the submit. Only a top-frame saveLogin may outlive its source document, and only
  // while the tab still shows the SAME ORIGIN it was captured on. TOTP, fill and settings keep
  // the strict document binding: a 2FA seed is detected on the page it is shown on, and a
  // different document there is a different page.
  function survivesNavigation(record) {
    return !!record && record.kind === 'saveLogin' && record.frameId === 0;
  }

  // Exact origin (scheme + host + port), not hostname-only: https -> http (downgrade), a
  // different port and a different scheme must all be refused. `record.origin` was derived
  // from the authenticated sender url when the record was created, never from the message.
  // The captured pageUrl is NOT touched: the native save still names the page that was
  // submitted, not wherever the site redirected to.
  function liveOriginMatches(record, liveUrl) {
    if (!record || typeof record.origin !== 'string' || typeof liveUrl !== 'string' || !liveUrl) return false;
    try {
      var live = new URL(liveUrl);
      return /^https?:$/.test(live.protocol) && live.origin === record.origin;
    } catch (e) {
      return false;
    }
  }

  function newestFirst(list) {
    // Two records created in the same millisecond keep insertion order, newest (appended) first.
    return list.map(function (r, i) { return { r: r, i: i }; })
      .sort(function (a, b) { return (b.r.createdAt - a.r.createdAt) || (b.i - a.i); })
      .map(function (x) { return x.r; });
  }

  function forTab(list, tabId, now) {
    return newestFirst(prune(list, now).filter(function (r) { return r.tabId === tabId; }));
  }

  function find(list, id, now) {
    var hits = prune(list, now).filter(function (r) { return r.id === id; });
    return hits.length ? hits[0] : null;
  }

  // Take-and-delete in one step: the caller writes `rest` back, so a record can be consumed once.
  function take(list, id, now) {
    var all = normalizeList(list);
    var record = find(all, id, now);
    return {
      record: record,
      rest: prune(all.filter(function (r) { return r.id !== id; }), now)
    };
  }

  function removeTab(list, tabId) {
    return normalizeList(list).filter(function (r) { return r.tabId !== tabId; });
  }

  // The label of an otpauth:// URL ("Issuer:account") is shown so the user can recognise the
  // account. Never the query: that is where the secret lives.
  function totpLabel(value) {
    if (typeof value !== 'string' || value.indexOf('otpauth://') !== 0) return '';
    try {
      var path = new URL(value).pathname.replace(/^\/+/, '');
      return decodeURIComponent(path).slice(0, 120);
    } catch (e) {
      return '';
    }
  }

  // What the popup and the prompt window may see. No password, no seed, no otpauth string, no
  // documentId: only what is needed to draw the card and to name the prompt back to the background.
  function redactedView(record) {
    var view = {
      id: record.id,
      kind: record.kind,
      origin: record.origin,
      expiresAt: record.expiresAt
    };
    if (record.kind === 'saveLogin') {
      view.username = record.payload.username;
      view.passwordDetected = true;
      // Names only: the values stay in the record until the app shows them for review.
      if (Array.isArray(record.payload.fields) && record.payload.fields.length) {
        view.fields = record.payload.fields.map(function (field) { return { kind: field.kind, label: field.label }; });
      }
    } else if (record.kind === 'saveTotp') {
      view.label = totpLabel(record.payload.otpauthOrSecret);
      view.secretDetected = true;
    }
    return view;
  }

  return {
    PROMPT_TTL_MS: PROMPT_TTL_MS,
    PROMPT_STORAGE_KEY: PROMPT_STORAGE_KEY,
    PROMPT_KINDS: PROMPT_KINDS,
    MAX_EXTRA_FIELDS: MAX_EXTRA_FIELDS,
    checkFields: checkFields,
    create: create,
    isExpired: isExpired,
    prune: prune,
    upsert: upsert,
    forTab: forTab,
    find: find,
    take: take,
    removeTab: removeTab,
    survivesNavigation: survivesNavigation,
    liveOriginMatches: liveOriginMatches,
    newestFirst: newestFirst,
    redactedView: redactedView
  };
})();

if (typeof module !== 'undefined') {
  module.exports = mypassPromptRecord;
}
