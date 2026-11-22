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
  if (INVISIBLE_TAGS.has(element.tagName)) return true;
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

// A plain sliding window over every token (including surrounding label words) is unsafe:
// concatenating enough ordinary sentence words (e.g. "Your"+"manual"+"setup"+"key" ->
// "YOURMANUALSETUPKEY") can accidentally satisfy the base32 alphabet+length check before
// the real key is ever reached. Real setup keys are rendered either all-uppercase or
// all-lowercase (e.g. "JBSW Y3DP EHPK 3PXP" or "jbsw y3dp ehpk 3pxp"), so restrict
// the scan to maximal runs of tokens that are uniformly one case as written, which
// excludes ordinary mixed-case words. A token holding a character outside the base32
// alphabet (0, 1, 8, 9) can never be part of a key, so it ends the run.
//
// Inside a run the group structure decides what is a key, not a window size:
//   1. A token whose width differs from both neighbours is a standalone key candidate.
//      Those are tried first (unambiguous, no grouping to verify).
//   2. Real grouped keys use fixed-width groups (e.g. 8x4 chars); prose words have varied
//      lengths. So a candidate is a maximal block of consecutive equal-width tokens, taken
//      as a whole. The whole block is validated, never a sub-span or a prefix of it, so a
//      32-char key is not cut to its first valid 16-char prefix, and a key longer than 8
//      groups is not cut at an arbitrary token count. A block that does not validate (for
//      example one past the base32 length cap) is skipped, so detection fails closed.
//
// Limitations: an all-lowercase or all-uppercase key is indistinguishable from a run of
// same-width words (a 4-letter label word such as "NEXT" directly before or after a
// 4-wide grouped key joins its block, as does a 2-letter word such as "OK" after a key
// grouped in 2s). A key whose final group is narrower than the others (a 26-char key
// rendered as 6x4 + 2) is NOT supported: the short tail is indistinguishable from a
// trailing prose word ("OK", "IN", "AM"), and absorbing it would corrupt the secret of
// every ordinary key that is followed by such a word. Such a key is read as its
// uniform-width groups only, which may be a valid-looking incomplete secret; that is a
// known limitation, not a claim that the result is the complete key. The same holds for a
// key whose groups are not all the same width (mixed-width groups): only the uniform
// prefix block is read. Detection does not promise universal coverage; label words of
// other widths are never absorbed.
//
// Digit-only tokens (2-7) are valid in both cases, so they belong to an uppercase run and
// a lowercase run alike. Runs are therefore collected per case over the whole token list,
// which keeps "2345 6723 jbsw y3dp ..." from losing its leading numeric groups to an
// uppercase run that stops at the first lowercase letter.
const CAPS_TOKEN_REGEX = /^[A-Z2-7]+$/;
const LOWER_TOKEN_REGEX = /^[a-z2-7]+$/;

// Returns the normalized key found in one case-uniform run of tokens, or null.
function extractKeyFromRun(run) {
  const blocks = [];
  for (let start = 0; start < run.length;) {
    let end = start + 1;
    while (end < run.length && run[end].length === run[start].length) end += 1;
    blocks.push({ start, end });
    start = end;
  }
  for (const { start, end } of blocks) {
    // A lone token must hold a base32 digit: a 16+ letter word such as OUTDATEDINTERNET
    // is a word, not a key. Grouped keys may be all letters (the equal-width groups already
    // rule out prose).
    if (end - start === 1 && /[2-7]/.test(run[start]) && totpBase32.isBase32Secret(run[start])) {
      return totpBase32.normalizeBase32Candidate(run[start]);
    }
  }
  for (const { start, end } of blocks) {
    if (end - start < 2) continue;
    const candidate = run.slice(start, end).join('');
    if (totpBase32.isBase32Secret(candidate)) {
      return totpBase32.normalizeBase32Candidate(candidate);
    }
  }
  return null;
}

// Maximal runs of consecutive tokens that all match tokenRegex, with their start index.
function collectRuns(tokens, tokenRegex) {
  const runs = [];
  for (let i = 0; i < tokens.length;) {
    if (!tokenRegex.test(tokens[i])) {
      i += 1;
      continue;
    }
    let runEnd = i;
    while (runEnd < tokens.length && tokenRegex.test(tokens[runEnd])) runEnd += 1;
    runs.push({ start: i, tokens: tokens.slice(i, runEnd) });
    i = runEnd;
  }
  return runs;
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
  const tokens = [];
  const owners = [];
  for (const entry of visibleTextNodes(body)) {
    for (const token of entry.text.split(/\s+/)) {
      if (!token) continue;
      tokens.push(token);
      owners.push(entry.element);
    }
  }
  // Document order decides which key wins. At the same start (leading numeric groups are
  // valid in both cases, so an uppercase and a lowercase run begin together) the longer run
  // goes first: the shorter one is only the numeric prefix of the complete key and would
  // otherwise win and return that prefix as a valid-looking incomplete secret.
  const runs = collectRuns(tokens, CAPS_TOKEN_REGEX)
    .concat(collectRuns(tokens, LOWER_TOKEN_REGEX))
    .sort((a, b) => a.start - b.start || b.tokens.length - a.tokens.length);
  for (const run of runs) {
    const key = extractKeyFromRun(run.tokens);
    if (key && hasContextNear(owners[run.start])) return key;
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
