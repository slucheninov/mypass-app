// TOTP setup-key discovery: otpauth links, page text, QR images, and manual keys.
// The base32 check itself lives in shared/base32.js so the popup paste field
// accepts the same secrets this scanner finds.
//
// `totpBase32` is a namespace on purpose. base32.js declares isBase32Secret as
// a top-level function, and this file is a classic script in that same global
// scope. A const/let here with that name is a load-time SyntaxError.
//
// The browser branch names the functions directly (`isBase32Secret`, not
// `globalThis.isBase32Secret`). In a content script `globalThis` is the page,
// which does not have this extension's functions.

const totpBase32 = typeof require === 'function'
  ? require('./base32.js')
  : { isBase32Secret, normalizeBase32Candidate };

const OTPAUTH_REGEX = /otpauth:\/\/totp\/[^\s"'<>]+/i;
// A setup key is offered only next to words that ask for one. Weak words ("secret",
// "manual") are everywhere on ordinary pages (a "manual" gearbox, CSS `hyphens: manual`, an
// OAuth `clientSecret` in a script), so the context has to name 2FA itself: in English,
// German, Ukrainian and Russian. Cyrillic is written as escapes (this file is not one of
// the files allowed to hold literal Cyrillic).
const TOTP_CONTEXT_REGEX = new RegExp([
  'authenticat(?:or|ion app)', '\\b2fa\\b', '\\bmfa\\b', '\\btotp\\b', 'two[\\s-]?factor',
  'multi[\\s-]?factor', '(?:two|2)[\\s-]?step', 'setup[\\s-]?key', 'secret[\\s-]?key',
  // de: Zwei-Faktor, 2-Faktor, Authentifizierungs-App, Einrichtungsschlüssel, geheimer Schlüssel
  'zwei[\\s-]?faktor', '2[\\s-]?faktor', 'authentifizierungs[\\s-]?app',
  'einrichtungs[\\s-]?schlüssel', 'geheimer\\s+schlüssel', 'geheimschlüssel',
  // uk: authenticator, two-factor, two-step, secret key, setup key
  '\u0430\u0432\u0442\u0435\u043d\u0442\u0438\u0444\u0456\u043a\u0430\u0442\u043e\u0440',
  '\u0434\u0432\u043e\u0444\u0430\u043a\u0442\u043e\u0440\u043d',
  '\u0434\u0432\u043e\u0435\u0442\u0430\u043f\u043d',
  '\u0441\u0435\u043a\u0440\u0435\u0442\u043d\u0438\u0439\\s+\u043a\u043b\u044e\u0447',
  '\u043a\u043b\u044e\u0447\\s+\u043d\u0430\u043b\u0430\u0448\u0442\u0443\u0432\u0430\u043d\u043d\u044f',
  // ru: authenticator, two-factor, two-step, secret key, setup key
  '\u0430\u0443\u0442\u0435\u043d\u0442\u0438\u0444\u0438\u043a\u0430\u0442\u043e\u0440',
  '\u0434\u0432\u0443\u0445\u0444\u0430\u043a\u0442\u043e\u0440\u043d',
  '\u0434\u0432\u0443\u0445\u044d\u0442\u0430\u043f\u043d',
  '\u0441\u0435\u043a\u0440\u0435\u0442\u043d\u044b\u0439\\s+\u043a\u043b\u044e\u0447',
  '\u043a\u043b\u044e\u0447\\s+\u043d\u0430\u0441\u0442\u0440\u043e\u0439\u043a\u0438',
].join('|'), 'i');
// How far up from the key's own element the context may sit: the key's line, its card and
// the setup step around it, not the page as a whole.
const CONTEXT_LEVELS = 4;

// Text a person can see. Scripts, styles, metadata, templates and hidden subtrees hold
// strings no setup page shows (an OAuth client secret, an "outdated browser" banner kept
// in the DOM for later), so they never count as a key or as its context.
const INVISIBLE_TAGS = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'META', 'LINK',
  'TITLE', 'SVG', 'IFRAME', 'OBJECT', 'CANVAS', 'TEXTAREA', 'SELECT']);

function isElementHidden(element) {
  return INVISIBLE_TAGS.has(element.tagName) || isHiddenByStyle(element);
}

function isHiddenByStyle(element) {
  if (element.hidden || element.getAttribute('aria-hidden') === 'true') return true;
  const view = element.ownerDocument && element.ownerDocument.defaultView;
  if (!view || typeof view.getComputedStyle !== 'function') return false;
  const style = view.getComputedStyle(element);
  if (!style) return false;
  return style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse'
    || style.opacity === '0';
}

// The visible text nodes under `root`, in document order, as { text, element }.
function visibleTextNodes(root) {
  const out = [];
  const visit = (node) => {
    for (const child of node.childNodes || []) {
      if (child.nodeType === 3) {
        if (child.nodeValue && child.nodeValue.trim()) out.push({ text: child.nodeValue, element: node });
      } else if (child.nodeType === 1 && !isElementHidden(child)) {
        visit(child);
      }
    }
  };
  if (root && root.nodeType === 1 && isElementHidden(root)) return out;
  if (root) visit(root);
  return out;
}

function visibleText(root) {
  return visibleTextNodes(root).map((entry) => entry.text).join(' ');
}

function scanRoot(root) {
  const scope = root || (typeof document !== 'undefined' ? document : null);
  if (!scope) return null;
  return scope.body || scope;
}

function detectOtpauthLink(root) {
  const scope = root || document;
  const links = Array.from(scope.querySelectorAll('a[href]'));
  for (const link of links) {
    const href = link.getAttribute('href') || '';
    const match = href.match(OTPAUTH_REGEX);
    if (match) return match[0];
  }
  return null;
}

function detectOtpauthText(root) {
  const body = scanRoot(root);
  if (!body) return null;
  // Cheap check first: most pages hold no otpauth URL at all, in any text.
  if (!OTPAUTH_REGEX.test(body.textContent || '')) return null;
  const match = visibleText(body).match(OTPAUTH_REGEX);
  return match ? match[0] : null;
}

// A setup key is a distinct token on the page, never words joined across a sentence: joining
// the tokens of "Use the authenticator app to finish this step with code sent here." gave
// THISSTEPWITHCODESENT, a valid-looking base32 string. So a key candidate is one of:
//   - the entire (whitespace-collapsed) visible text of its own element: a <code>, <kbd>,
//     <samp>, <tt>, <strong> or a monospace <span> holding just the key is that element;
//   - that text after a short label ("Key:", "Secret key:", "Schlüssel:", "\u041a\u043b\u044e\u0447:" (uk/ru "key")): at most
//     three words and a colon in the same element, or the label in its own child element right
//     before the key's text node (<p><b>Secret</b> JBSW Y3DP …</p>);
//   - the value of a read-only <input> or <textarea>.
// The token itself must be all upper or all lower case and either
//   - grouped: 4 to 8 groups of exactly 4 base32 characters separated by single spaces, the
//     only form allowed without a digit (Google shows "aabb ccdd …" in lowercase), or
//   - a single token holding at least one base32 digit (2-7): a 16+ letter word such as
//     OUTDATEDINTERNET is a word, not a key.
// Every other shape (groups of 8, more than 8 groups, a trailing word) is rejected: detection
// fails closed and the user can still paste the key in the popup.
const GROUPED_KEY_REGEX = /^(?:[A-Z2-7]{4}(?: [A-Z2-7]{4}){3,7}|[a-z2-7]{4}(?: [a-z2-7]{4}){3,7})$/;
const SINGLE_KEY_REGEX = /^(?:[A-Z2-7]+|[a-z2-7]+)$/;
const KEY_LABEL_REGEX = /^[\p{L}\p{M}][\p{L}\p{M}'’-]*(?: [\p{L}\p{M}][\p{L}\p{M}'’-]*){0,2}$/u;
const MAX_LABEL_CHARS = 32;
// Longest element text worth looking at: a 128-char key in groups, plus a label.
const MAX_KEY_TEXT = 200;

function collapseSpaces(text) {
  return String(text || '').replace(/\s+/g, ' ').trim();
}

function keyFromToken(token) {
  if (GROUPED_KEY_REGEX.test(token)) {
    const joined = token.replace(/ /g, '');
    return totpBase32.isBase32Secret(joined) ? totpBase32.normalizeBase32Candidate(joined) : null;
  }
  if (SINGLE_KEY_REGEX.test(token) && /[2-7]/.test(token) && totpBase32.isBase32Secret(token)) {
    return totpBase32.normalizeBase32Candidate(token);
  }
  return null;
}

function isShortLabel(text) {
  const label = collapseSpaces(text).replace(/:$/, '').trim();
  return label.length > 0 && label.length <= MAX_LABEL_CHARS && KEY_LABEL_REGEX.test(label);
}

// The key `element` shows as a distinct token (see above), or null.
function keyShownBy(element) {
  const text = collapseSpaces(visibleText(element));
  if (!text || text.length > MAX_KEY_TEXT) return null;
  const whole = keyFromToken(text);
  if (whole) return whole;
  const colon = text.indexOf(':');
  if (colon > 0 && isShortLabel(text.slice(0, colon))) {
    const labelled = keyFromToken(text.slice(colon + 1).trim());
    if (labelled) return labelled;
  }
  const parts = Array.from(element.childNodes || []).filter((child) => (child.nodeType === 3
    ? !!(child.nodeValue && child.nodeValue.trim()) : child.nodeType === 1 && !isElementHidden(child)));
  if (parts.length === 2 && parts[0].nodeType === 1 && parts[1].nodeType === 3 && isShortLabel(visibleText(parts[0]))) {
    return keyFromToken(collapseSpaces(parts[1].nodeValue));
  }
  return null;
}

// Hidden by its own style or attributes, or by an ancestor's.
function isInHiddenSubtree(element) {
  for (let node = element; node && node.nodeType === 1; node = node.parentElement) {
    if (node === element ? isHiddenByStyle(node) : isElementHidden(node)) return true;
  }
  return false;
}

// The key in a read-only text field's value, or null.
function keyInReadOnlyField(field) {
  const type = String(field.getAttribute('type') || '').toLowerCase();
  if (field.tagName === 'INPUT' && ['hidden', 'password', 'checkbox', 'radio', 'button', 'submit'].includes(type)) return null;
  if (isInHiddenSubtree(field)) return null;
  const value = collapseSpaces(field.value);
  return value && value.length <= MAX_KEY_TEXT ? keyFromToken(value) : null;
}

// True when the key's element or one of its nearest ancestors shows 2FA wording.
function hasContextNear(element) {
  let node = element;
  for (let level = 0; node && level <= CONTEXT_LEVELS; level += 1) {
    if (TOTP_CONTEXT_REGEX.test(visibleText(node))) return true;
    if (node.tagName === 'BODY') return false;
    node = node.parentElement;
  }
  return false;
}

function detectManualSetupKey(root) {
  const body = scanRoot(root);
  if (!body) return null;
  // Cheap check first, over all text: no 2FA wording anywhere means no visible key context.
  if (!TOTP_CONTEXT_REGEX.test(body.textContent || '')) return null;
  // The elements that directly hold visible text, in document order: each is a candidate.
  const elements = [];
  const seen = new Set();
  for (const entry of visibleTextNodes(body)) {
    if (seen.has(entry.element)) continue;
    seen.add(entry.element);
    elements.push(entry.element);
  }
  for (const element of elements) {
    const key = keyShownBy(element);
    if (key && hasContextNear(element)) return key;
  }
  if (typeof body.querySelectorAll === 'function') {
    for (const field of body.querySelectorAll('input[readonly], textarea[readonly]')) {
      const key = keyInReadOnlyField(field);
      if (key && hasContextNear(field)) return key;
    }
  }
  return null;
}

// Per-image QR decode memoization. detectQrCandidate re-scans every <img> on the page on
// each debounced mutation sweep (~400ms), and the jsQR decode — not the DOM walk — is the
// expensive part. Keyed on the image element itself (a WeakMap, so a removed <img> is
// simply forgotten rather than pinned in memory) plus a content key so an SPA that reuses
// the same <img> node for a different picture (a src swap) still gets a fresh decode
// instead of a stale cached result. `naturalWidth`/`naturalHeight` are folded into the key
// for the same reason: some sites redraw the same URL at a different size.
const qrDecodeCache = new WeakMap();

function qrImageCacheKey(imgEl) {
  return (imgEl.currentSrc || imgEl.src) + '|' + imgEl.naturalWidth + 'x' + imgEl.naturalHeight;
}

function decodeQrFromImage(imgEl) {
  try {
    if (!imgEl.complete || !imgEl.naturalWidth) return null;
    const key = qrImageCacheKey(imgEl);
    const cached = qrDecodeCache.get(imgEl);
    if (cached && cached.key === key) return cached.result;
    const canvas = document.createElement('canvas');
    canvas.width = imgEl.naturalWidth;
    canvas.height = imgEl.naturalHeight;
    const ctx = canvas.getContext('2d');
    ctx.drawImage(imgEl, 0, 0);
    const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
    const result = typeof jsQR === 'function' ? jsQR(imageData.data, imageData.width, imageData.height) : null;
    const decoded = result ? result.data : null;
    qrDecodeCache.set(imgEl, { key, result: decoded });
    return decoded;
  } catch (error) {
    // getImageData throws on a CORS-tainted canvas (cross-origin image) —
    // skip rather than treat as a hard failure. Not cached: a CORS failure is about the
    // canvas/context, not the image's content, so nothing here is safe to remember.
    return null;
  }
}

function detectQrCandidate(root) {
  const scope = root || document;
  const images = Array.from(scope.querySelectorAll('img'));
  for (const img of images) {
    if ((img.naturalWidth || 0) < 40 || (img.naturalHeight || 0) < 40) continue;
    const decoded = decodeQrFromImage(img);
    if (decoded && OTPAUTH_REGEX.test(decoded)) {
      return decoded.match(OTPAUTH_REGEX)[0];
    }
  }
  return null;
}

function detectTotpCandidateWithPriority(root) {
  const link = detectOtpauthLink(root);
  if (link) return { candidate: link, priority: 1 };
  const text = detectOtpauthText(root);
  if (text) return { candidate: text, priority: 2 };
  const qr = detectQrCandidate(root);
  if (qr) return { candidate: qr, priority: 3 };
  const manual = detectManualSetupKey(root);
  if (manual) return { candidate: manual, priority: 4 };
  return null;
}

function shouldShowTotpBanner(detected, state) {
  if (!detected) return false;
  if (state.vaultLocked) return false;
  if (!state.totpBannerDismissed) return true;
  return detected.priority < state.totpBannerDismissed.priority;
}

function forgetDecodedQr(imgEl) {
  if (imgEl) qrDecodeCache.delete(imgEl);
}

var mypassTotpDetection = {
  detectOtpauthLink,
  detectOtpauthText,
  detectManualSetupKey,
  decodeQrFromImage,
  detectQrCandidate,
  detectTotpCandidateWithPriority,
  shouldShowTotpBanner,
  forgetDecodedQr,
};

if (typeof module !== 'undefined') {
  module.exports = {
    detectOtpauthLink,
    detectOtpauthText,
    detectManualSetupKey,
    decodeQrFromImage,
    detectQrCandidate,
    detectTotpCandidateWithPriority,
    shouldShowTotpBanner,
    forgetDecodedQr,
    isBase32Secret: totpBase32.isBase32Secret,
  };
}
