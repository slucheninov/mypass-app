const PAIRING_STARTED_AT_KEY = 'pairingStartedAt';
const PAIRING_CHALLENGE_TTL_MS = 120_000;

const APP_UNAVAILABLE_STATUS_TEXT =
  'Cannot connect to MyPass. Make sure the app is open, then click Retry.';
const HOST_UNAVAILABLE_STATUS_TEXT =
  'The MyPass browser connector was not found. Complete browser setup, then click Retry.';
const APP_UNAVAILABLE_OPEN_HELPER_TEXT =
  'Open MyPass from Applications or the Dock. Unlock your vault if needed, then tap Retry.';

function shouldShowAppUnavailableActions(kind) {
  return kind === 'error';
}

const LOCKED_TITLE_TEXT = 'MyPass is locked';
const LOCKED_HELPER_TEXT = 'Open MyPass and unlock the vault';
const UNLOCK_UNREACHABLE_TEXT = 'Could not reach MyPass. Open the app and try again.';
const EMPTY_LIST_TITLE_TEXT = 'No saved logins for this site';
const LIST_HEADER_TEXT = 'Select an entry to autofill:';
const FAVORITES_HEADER_TEXT = 'Favorites';
const NO_FAVORITES_TEXT = 'No favorites';
const NO_ITEMS_TEXT = 'No items';
const NO_SEARCH_RESULTS_TEXT = 'No matches';
const BROWSER_CATEGORIES = [
  { id: 'thisSite', title: 'This site' },
  { id: 'favorites', title: 'Favorites' },
  { id: 'all', title: 'All' },
  { id: 'logins', title: 'Logins' },
  { id: 'passkeys', title: 'Passkeys' },
  { id: 'codes', title: 'Codes' },
  { id: 'cards', title: 'Cards' },
  { id: 'addresses', title: 'Addresses' },
  { id: 'notes', title: 'Notes' }
];

function passkeyProxyStatusText(status, context, matchedCredentials) {
  if (status && status.proxyState === 'disabled'
    && (!context || context.kind !== 'site' || !passkeySiteHints.hasSiteHint(context.url, matchedCredentials))) return '';
  const messages = {
    active: '',
    disabled: 'Passkeys are off. Enable them in MyPass settings.',
    locked: '', // The locked panel already explains how to unlock.
    unpaired: '', // Pairing has its own panel.
    unavailable: '', // Connection errors have their own panel.
    'attachment-error': 'Another provider may be blocking passkeys. Open MyPass settings for help.'
  };
  return messages[status && status.proxyState] || '';
}

function browserManagerDiagnosticsText(result) {
  if (!result) return '';
  const names = (result.extensions || []).map((item) => item.name);
  if (result.passwordSavingEnabled === true) names.unshift('Browser password saving');
  return names.length ? 'Also enabled: ' + names.join(', ') + '. Review in MyPass settings.' : '';
}

function categoryTitle(categoryId) {
  const match = BROWSER_CATEGORIES.find((category) => category.id === categoryId);
  return match ? match.title : FAVORITES_HEADER_TEXT;
}

// chrome://extensions, the new tab, and a tab with no URL have nothing to
// autofill. The popup lists favorites there instead of treating the missing
// page as "MyPass not found".
function isSiteUrl(url) {
  if (!url || typeof url !== 'string') return false;
  try {
    const protocol = new URL(url).protocol;
    return protocol === 'http:' || protocol === 'https:';
  } catch (e) {
    return false;
  }
}
const PAIRING_PANEL_TITLE_TEXT = 'Connect browser extension';
const UNLOCK_BUTTON_LABEL = 'Unlock MyPass';
// Must match PAIRING_STORAGE_KEY in background.js — asserted by unlockButton.test.js.
const POPUP_PAIRING_STORAGE_KEY = 'pairingSecret';
// The popup was expected to close when MyPass takes focus, so no waiting state was
// planned. It does not reliably close, so it polls the vault instead of stranding the
// user on a stale locked panel.
const UNLOCK_POLL_INTERVAL_MS = 800;

// Locked-panel auto refresh. One module-level handle so repeated showPanel('locked')
// calls cannot stack intervals and leaving the locked state always clears it.
const LOCKED_POLL_INTERVAL_MS = 2000;
let lockedPollHandle = null;
function startLockedPolling(reload) {
  if (lockedPollHandle !== null) return;
  lockedPollHandle = setInterval(reload, LOCKED_POLL_INTERVAL_MS);
}
function stopLockedPolling() {
  if (lockedPollHandle === null) return;
  clearInterval(lockedPollHandle);
  lockedPollHandle = null;
}
const UNLOCK_POLL_MAX_ATTEMPTS = 75; // 800 ms x 75 = 60 s of real unlocking time
const WAITING_FOR_UNLOCK_TEXT = 'Waiting for you to unlock MyPass\u2026';

function shouldShowUnlockButton(pairingSecret) {
  return typeof pairingSecret === 'string' && pairingSecret.length > 0;
}

function classifyRequestUnlockResponse(response) {
  if (!response) return 'unavailable';
  if (response.type === 'unlockPresented') return 'presented';
  if (response.type === 'unlockNotNeeded') return 'notNeeded';
  return 'unavailable';
}

function shouldContinueUnlockPolling(kind, attempt) {
  // Any non-locked outcome is terminal: unlocked means we are done, and an error or a
  // pairing prompt means polling cannot help.
  if (kind !== 'locked') return false;
  return attempt < UNLOCK_POLL_MAX_ATTEMPTS;
}

function isPairingChallengeFresh(startedAt, now) {
  if (startedAt == null || typeof startedAt !== 'number') return false;
  return now - startedAt < PAIRING_CHALLENGE_TTL_MS;
}

function normalizePairingCode(raw) {
  return String(raw).replace(/\D/g, '').slice(0, 6);
}

// letterFromTitle/colorForTitle (and the palette colorForTitle hashes into) moved to
// shared/entry-visuals.js so the inline panel can share them without drifting — see that
// file's header comment. popup.html loads entry-visuals.js as a <script> immediately before
// this one, so both names would already be globals in the browser — but several node tests
// (totpCircle, popup-fallback, totpPicker, passkeyPopup) require this file directly and drive
// it all the way into renderCredentials, so a bare global reference throws under node. require
// them there instead — the same three-world seam entry-visuals.js itself is built for.
// Bound as a namespace object, NOT destructured into `letterFromTitle`/`colorForTitle`
// names: popup.html loads entry-visuals.js as a classic <script> immediately before this
// one, so both scripts share ONE global scope, and entry-visuals.js declares those two
// names as top-level function declarations there. A top-level `const`/`let` here with the
// SAME names would be a lexical redeclaration of names the earlier script already put in
// the global environment — that is a load-time SyntaxError under
// GlobalDeclarationInstantiation, thrown before a single statement of this file runs (see
// ChromeExtension/tests/scriptScopes.test.js). Node can't see this: under `require`, this
// file is a CommonJS module with its own scope, so no collision exists there. Do not
// "simplify" this back into `const { letterFromTitle, colorForTitle } = ...`.
const entryVisuals = typeof require === 'function'
  ? require('./shared/entry-visuals.js')
  : { letterFromTitle: window.letterFromTitle, colorForTitle: window.colorForTitle };

// classifyGetCredentialsResponse is shared with the in-page panel
// (shared/bridge-status.js). isBase32Secret is shared with the page scanner
// (shared/base32.js). Same global-scope rule as entryVisuals above: popup.html
// loads those scripts first, so do not destructure their function names into
// top-level consts here.
const bridgeStatus = typeof require === 'function'
  ? require('./shared/bridge-status.js')
  : window.mypassBridgeStatus;
const base32 = typeof require === 'function'
  ? require('./shared/base32.js')
  : { isBase32Secret: window.isBase32Secret };
const passkeySiteHints = typeof require === 'function'
  ? require('./shared/passkey-sites.js')
  : window.mypassPasskeySites;
const pageIdentifier = typeof require === 'function'
  ? require('./shared/page-identifier.js')
  : window.mypassPageIdentifier;
const autofillSettings = typeof require === 'function'
  ? require('./shared/autofill-settings.js')
  : window.mypassAutofillSettings;
const promptCardApi = typeof require === 'function'
  ? require('./shared/prompt-card.js')
  : window.mypassPromptCard;
const appearanceTheme = typeof require === 'function'
  ? require('./shared/theme.js')
  : window.mypassTheme;

function getPairingStorageArea() {
  if (typeof chrome !== 'undefined' && chrome.storage && chrome.storage.session) {
    return chrome.storage.session;
  }
  return chrome.storage.local;
}

function readPairingStartedAt(cb) {
  getPairingStorageArea().get(PAIRING_STARTED_AT_KEY, (result) => {
    cb(result[PAIRING_STARTED_AT_KEY]);
  });
}

function writePairingStartedAt(cb) {
  getPairingStorageArea().set({ [PAIRING_STARTED_AT_KEY]: Date.now() }, cb);
}

function clearPairingStartedAt(cb) {
  getPairingStorageArea().remove(PAIRING_STARTED_AT_KEY, cb);
}

const TOTP_AUTO_HIDE_MS = 5000;

// Live countdown ring, mirroring MyPass/UI/EntryDetail/TOTPCountdownRingView.swift:
// a green stroke whose arc length equals the REMAINING fraction of the 30 s TOTP
// window, shrinking clockwise from 12 o'clock and resetting to full each window.
const TOTP_RING_PERIOD_SECONDS = 30;
const TOTP_RING_CIRCUMFERENCE = 2 * Math.PI * 14;

function totpRemainingFraction(nowMs, periodSeconds = TOTP_RING_PERIOD_SECONDS) {
  const remaining = periodSeconds - ((nowMs / 1000) % periodSeconds);
  return remaining / periodSeconds;
}

function totpRingDashOffset(fraction, circumference) {
  return circumference * (1 - fraction);
}

// Single shared 1 Hz ticker for every ring currently on screen, started lazily
// when the first ring registers and torn down once the last one disconnects.
let totpRingRegistry = [];
let totpRingInterval = null;

function applyTotpRingOffset(entry, fraction, snap) {
  const offset = totpRingDashOffset(fraction, entry.circumference);
  const style = entry.svgCircleEl.style;
  if (snap) {
    // Window rollover: jump back to (near) full without animating backwards.
    style.transition = 'none';
    style.strokeDashoffset = String(offset);
    // Force a reflow so the "no transition" write lands before the class's own
    // 1 s linear transition is restored on the next line.
    void entry.svgCircleEl.offsetWidth;
    style.transition = '';
  } else {
    style.strokeDashoffset = String(offset);
  }
}

function tickTotpRings(nowMs = Date.now()) {
  // Self-clean mirroring the outside-click listener pattern: drop rings whose
  // host circle has left the DOM (list re-rendered), and once the registry is
  // empty stop the interval so node tests (and idle popups) don't keep ticking.
  totpRingRegistry = totpRingRegistry.filter((entry) => entry.hostEl.isConnected !== false);
  if (totpRingRegistry.length === 0) {
    if (totpRingInterval !== null) {
      clearInterval(totpRingInterval);
      totpRingInterval = null;
    }
    return;
  }

  const fraction = totpRemainingFraction(nowMs, TOTP_RING_PERIOD_SECONDS);
  const windowIndex = Math.floor(nowMs / 1000 / TOTP_RING_PERIOD_SECONDS);

  totpRingRegistry.forEach((entry) => {
    const rolledOver = windowIndex !== entry.lastWindowIndex;
    applyTotpRingOffset(entry, fraction, rolledOver);
    // The window rolled over while digits were revealed (not the copied flash) —
    // the shown code is now stale, so re-request it in place without touching
    // the auto-hide timer.
    if (rolledOver && entry.hostEl.classList.contains('revealed')) {
      entry.requestTotp().then((code) => {
        if (code) (entry.textEl || entry.hostEl).textContent = code;
      }).catch(() => {});
    }
    entry.lastWindowIndex = windowIndex;
  });
}

function registerTotpRing(entry, nowMs = Date.now()) {
  totpRingRegistry.push({
    ...entry,
    lastWindowIndex: Math.floor(nowMs / 1000 / TOTP_RING_PERIOD_SECONDS)
  });
  if (totpRingInterval === null) {
    totpRingInterval = setInterval(() => tickTotpRings(Date.now()), 1000);
  }
}

function shouldShowTotpCircle(hasTotp) {
  return hasTotp === true;
}

/// Shared by the ordinary credential list and the passkey picker rows, so the two
/// surfaces that can name "this entry has a passkey" always agree on markup/class.
function buildPasskeyBadgeElement() {
  const badge = document.createElement('span');
  badge.className = 'passkey-badge';
  badge.textContent = 'Passkey';
  return badge;
}

function createTotpCircleController(circleEl, requestTotp, options = {}) {
  const textEl = options.textEl || circleEl;
  let hideTimer = null;
  // "active" covers both the revealed-digits state and the copied-flash state — either
  // one must be dismissible by a second left-click or by clicking outside the circle.
  let active = false;

  function clearHideTimer() {
    if (hideTimer) {
      clearTimeout(hideTimer);
      hideTimer = null;
    }
  }

  function showIdle() {
    clearHideTimer();
    active = false;
    textEl.textContent = '';
    circleEl.classList.remove('revealed', 'copied');
  }

  function showDigits(code) {
    active = true;
    textEl.textContent = code;
    circleEl.classList.add('revealed');
    circleEl.classList.remove('copied');
    clearHideTimer();
    hideTimer = setTimeout(showIdle, TOTP_AUTO_HIDE_MS);
  }

  function showCopiedFlash() {
    active = true;
    textEl.textContent = 'Copied';
    circleEl.classList.add('copied');
    circleEl.classList.remove('revealed');
    clearHideTimer();
    hideTimer = setTimeout(showIdle, TOTP_AUTO_HIDE_MS);
  }

  function handleLeftClick(event) {
    event.stopPropagation();
    if (active) {
      showIdle();
      return;
    }
    requestTotp().then((code) => {
      if (code) showDigits(code);
    }).catch(() => {});
  }

  function handleContextMenu(event) {
    event.preventDefault();
    event.stopPropagation();
    requestTotp().then((code) => {
      if (!code) return;
      navigator.clipboard.writeText(code).then(showCopiedFlash).catch(() => {});
    }).catch(() => {});
  }

  function handleMouseDown(event) {
    event.stopPropagation();
  }

  function handleOutsideClick(event) {
    // Self-clean once this row's circle has been removed from the DOM (e.g.
    // renderCredentials re-rendered the list on retry/unlock) — otherwise this
    // document-level listener would accumulate across every re-render.
    if (circleEl.isConnected === false) {
      document.removeEventListener('click', handleOutsideClick, true);
      return;
    }
    if (active && !circleEl.contains(event.target)) {
      showIdle();
    }
  }

  circleEl.addEventListener('mousedown', handleMouseDown);
  circleEl.addEventListener('click', handleLeftClick);
  circleEl.addEventListener('contextmenu', handleContextMenu);
  // Capture phase: a sibling circle's own click handler calls stopPropagation on the
  // bubble phase, which would otherwise stop this document-level listener from ever
  // seeing clicks on OTHER circles/rows. Capture runs before that stopPropagation.
  document.addEventListener('click', handleOutsideClick, true);

  return { showIdle, showDigits, showCopiedFlash };
}

const SVG_NS = 'http://www.w3.org/2000/svg';

function buildTotpRingSvg() {
  const ring = document.createElementNS(SVG_NS, 'svg');
  ring.setAttribute('viewBox', '0 0 32 32');
  ring.setAttribute('class', 'totp-ring');
  ring.setAttribute('aria-hidden', 'true');
  ring.setAttribute('focusable', 'false');

  const ringCircle = document.createElementNS(SVG_NS, 'circle');
  ringCircle.setAttribute('cx', '16');
  ringCircle.setAttribute('cy', '16');
  ringCircle.setAttribute('r', '14');
  ringCircle.setAttribute('fill', 'none');
  ringCircle.setAttribute('stroke', '#34c759');
  ringCircle.setAttribute('stroke-width', '2');
  ringCircle.setAttribute('stroke-linecap', 'round');
  ringCircle.setAttribute('transform', 'rotate(-90 16 16)');
  ringCircle.setAttribute('stroke-dasharray', String(TOTP_RING_CIRCUMFERENCE));
  const initialFraction = totpRemainingFraction(Date.now(), TOTP_RING_PERIOD_SECONDS);
  ringCircle.style.strokeDashoffset = String(totpRingDashOffset(initialFraction, TOTP_RING_CIRCUMFERENCE));

  ring.appendChild(ringCircle);
  return { ring, ringCircle };
}

function buildTotpCircleElement(credential, url) {
  const circle = document.createElement('div');
  circle.className = 'totp-circle';
  circle.setAttribute('role', 'button');
  circle.setAttribute('aria-label', 'Reveal or copy verification code');

  const requestTotp = () => new Promise((resolve, reject) => {
    chrome.runtime.sendMessage({ type: 'getTotp', id: credential.id, url }, (response) => {
      if (response && response.type === 'totp' && response.code) {
        resolve(response.code);
      } else {
        reject(new Error('getTotp failed'));
      }
    });
  });

  // The ring lives underneath as its own SVG (pointer-events: none, so it never
  // intercepts clicks) and the digits/"Copied" text live in a sibling label span
  // above it — kept separate so revealing/hiding the code (which replaces
  // textContent) never wipes out the ring.
  const { ring, ringCircle } = buildTotpRingSvg();
  circle.appendChild(ring);

  const label = document.createElement('span');
  label.className = 'totp-circle-label';
  circle.appendChild(label);

  createTotpCircleController(circle, requestTotp, { textEl: label });

  registerTotpRing({
    hostEl: circle,
    textEl: label,
    svgCircleEl: ringCircle,
    circumference: TOTP_RING_CIRCUMFERENCE,
    requestTotp
  });

  return circle;
}

function validateTotpPasteInput(raw) {
  const trimmed = (raw || '').trim();
  if (trimmed.toLowerCase().startsWith('otpauth://totp/')) {
    return { valid: true, value: trimmed };
  }
  if (isBase32PasteSecret(trimmed)) {
    return { valid: true, value: trimmed.replace(/\s+/g, '').toUpperCase() };
  }
  return { valid: false, value: null };
}

function isBase32PasteSecret(candidate) {
  return base32.isBase32Secret(candidate);
}

// Shared by the picker row click handler and routeTotpCandidate's zero/one-match
// branches so a proposeSaveTotp round-trip's actual outcome is reported instead of
// always claiming success.
function describeSaveProposalOutcome(response) {
  if (response && response.type === 'saveProposalAccepted') return 'Opened in MyPass';
  if (response && response.type === 'locked') return 'Unlock MyPass first';
  if (response && response.type === 'pairingRequired') return 'Connect to MyPass first';
  if (response && response.message) return response.message;
  return "Couldn't reach MyPass";
}

// Assigned once the DOM is wired up below: renderTotpPicker needs `listEl` and
// `buildLetterAvatarElement`, routeTotpCandidate needs renderTotpPicker, and
// requestPendingPrompt needs `showPanel` and the prompt panel. Declared here (rather
// than only inside the DOM block) so module.exports at the bottom of this file can see them.
let renderCredentials = null;
let renderTotpPicker = null;
let routeTotpCandidate = null;
let requestPendingPrompt = null;
let handleAddTotpClick = null;
let requestPageTotpCandidate = null;
let requestPageNewPasswordField = null;
let requestPageIdentifier = null;
let handleGeneratePasswordClick = null;
let rehydratePendingPasskeyCeremony = null;
let renderPasskeyCeremonyPanel = null;
let renderPasskeyPicker = null;
let sendPasskeyCreateConfirm = null;
let sendPasskeyGetApprove = null;
let sendPasskeyCancel = null;
let initPasskeySettingsToggle = null;
let refreshPasskeyProxy = null;
let initPasskeyPopupUI = null;

const PENDING_PASSKEY_KEY = 'pendingPasskeyCeremony';
const LAST_PASSKEY_FAILURE_KEY = 'lastPasskeyFailure';

/// Both values are shown on every confirm panel: the relying party the credential will
/// be bound to, and the origin the ceremony actually came from. A mismatch between them
/// is what a phishing attempt looks like, so the user has to be able to see both.
function passkeyCeremonySummary(pending) {
  if (!pending) return '';
  const origin = pending.originUrl || 'unknown origin';
  return `${pending.rpId} — ${origin}`;
}

/// Which panel a pending ceremony maps to. Pure so the routing can be tested without a
/// DOM: more than one match always means the picker, a create onto an entry that already
/// carries a passkey means the Replace confirm, and a get always requires an explicit
/// Approve — there is no silent auto-sign path even with exactly one match.
function choosePasskeyPanel(pending) {
  if (!pending || pending.phase === 'checking' || (pending.kind !== 'create' && pending.kind !== 'get')) return null;
  const matches = pending.matches || [];
  if (matches.length > 1) return 'passkey-picker';
  if (pending.kind === 'get') {
    return matches.length === 1 ? 'passkey-use' : null;
  }
  return matches.length === 1 && matches[0].hasPasskey ? 'passkey-replace' : 'passkey-save';
}

/// `{ headline, detail }` for the Save / Replace panels, so the user can see *which*
/// entry is about to be written to (or, with zero matches, that a new entry will be
/// created) rather than only the generic rpId/origin pair. `detail` is always the same
/// `passkeyCeremonySummary` line — kept as a second, secondary line so the phishing-
/// relevant rpId/origin comparison stays visible no matter which headline is shown.
///
/// `overrideMatch` lets the picker ask for the headline of a specific row the user just
/// clicked, rather than `pending.matches[0]` (which is only meaningful when there is
/// exactly one match — the picker path always has more than one).
function passkeyTargetSummary(pending, overrideMatch) {
  const detail = passkeyCeremonySummary(pending);
  if (!pending) return { headline: '', detail };
  const matches = pending.matches || [];
  const match = overrideMatch || (matches.length === 1 ? matches[0] : null);
  if (!match) {
    return { headline: `New entry for ${pending.rpId}`, detail };
  }
  const who = match.username ? `"${match.title}" (${match.username})` : `"${match.title}"`;
  const headline = pending.kind === 'get'
    ? `Sign in as ${who}?`
    : (match.hasPasskey ? `Replace passkey in ${who}?` : `Save passkey to ${who}?`);
  return { headline, detail };
}

if (typeof document !== 'undefined') {
  const lockedPanel = document.getElementById('locked-panel');
  const lockedTitleEl = document.getElementById('locked-title');
  const lockedHelperEl = document.getElementById('locked-helper');
  const lockedUnlockBtn = document.getElementById('locked-unlock-btn');

  const appUnavailablePanel = document.getElementById('app-unavailable-panel');
  const appUnavailableStatusEl = document.getElementById('app-unavailable-status');
  const appUnavailableActions = document.getElementById('app-unavailable-actions');
  const retryBtn = document.getElementById('retry-btn');
  const openMypassBtn = document.getElementById('open-mypass-btn');
  const openMypassHelper = document.getElementById('open-mypass-helper');
  openMypassHelper.textContent = APP_UNAVAILABLE_OPEN_HELPER_TEXT;

  const emptyPanel = document.getElementById('empty-panel');
  const emptyTitleEl = document.getElementById('empty-title');
  emptyTitleEl.textContent = EMPTY_LIST_TITLE_TEXT;

  const listPanel = document.getElementById('list-panel');
  const listHeaderEl = document.getElementById('list-header');
  listHeaderEl.textContent = LIST_HEADER_TEXT;
  const listEl = document.getElementById('list');
  const identifierOfferEl = document.getElementById('identifier-offer');
  const identifierOfferTextEl = document.getElementById('identifier-offer-text');
  const identifierOfferActionsEl = document.getElementById('identifier-offer-actions');
  if (identifierOfferEl) identifierOfferEl.hidden = true;

  const pairingPanel = document.getElementById('pairing-panel');
  const pairingTitleEl = document.getElementById('pairing-title');
  pairingTitleEl.textContent = PAIRING_PANEL_TITLE_TEXT;
  const connectBtn = document.getElementById('connect-btn');
  const codeEntry = document.getElementById('code-entry');
  const codeInput = document.getElementById('code-input');
  const submitCodeBtn = document.getElementById('submit-code-btn');
  const pairingErrorEl = document.getElementById('pairing-error');

  const passkeyCheckingPanel = document.getElementById('passkey-checking-panel');
  // A save-login / save-2FA / settings prompt raised by the page for this tab. Like a passkey
  // ceremony it owns the popup while it is shown; a ceremony still outranks it.
  const promptPanel = document.getElementById('prompt-panel');
  const promptCardRoot = document.getElementById('prompt-card-root');
  let promptOwnsPopup = false;
  let promptController = null;
  let promptRevision = 0;
  let pendingPasskeyOwnsPopup = false;
  let visiblePasskeyFailure = null;
  let activePasskeyRequestId = null;
  let pendingPasskeyRevision = 0;
  let ordinaryPopupInitialized = false;
  let passkeySettingsInitialized = false;
  const passkeySavePanel = document.getElementById('passkey-save-panel');
  const passkeyReplacePanel = document.getElementById('passkey-replace-panel');
  const passkeyUsePanel = document.getElementById('passkey-use-panel');
  const passkeyPickerPanel = document.getElementById('passkey-picker-panel');
  const passkeyPickerListEl = document.getElementById('passkey-picker-list');
  const browserChrome = document.getElementById('browser-chrome');
  const popupMenu = document.getElementById('popup-menu');
  const popupMenuBtn = document.getElementById('popup-menu-btn');
  const popupMenuViews = {
    root: document.getElementById('popup-menu-root'),
    help: document.getElementById('popup-menu-help')
  };
  const vaultSearch = document.getElementById('vault-search');
  const categoryBtn = document.getElementById('category-btn');
  const categoryWrap = document.getElementById('category-wrap');
  const categoryMenu = document.getElementById('category-menu');
  const addEntryBtn = document.getElementById('add-entry-btn');
  const addEntryMenu = document.getElementById('add-entry-menu');
  const openGeneratorBtn = document.getElementById('open-generator-btn');

  const addTotpSection = document.getElementById('add-totp-section');
  const addTotpBtn = document.getElementById('add-totp-btn');
  const totpPasteInput = document.getElementById('totp-paste-input');
  const addTotpErrorEl = document.getElementById('add-totp-error');
  // Belt-and-suspenders alongside popup.html's static `hidden` attribute (same idiom as
  // PANEL_SECTIONS/showPanel always setting `.hidden = true` before selectively revealing):
  // the section must only appear once a real 2FA candidate is in context.
  addTotpSection.hidden = true;

  const generatePasswordSection = document.getElementById('generate-password-section');
  const generatePasswordBtn = document.getElementById('generate-password-btn');
  const generatePasswordErrorEl = document.getElementById('generate-password-error');
  generatePasswordSection.hidden = true;

  // Set by queryCredentials for every credential lookup (initial load, retries, and
  // post-pairing/post-unlock re-checks) so the Add-2FA button can reuse the active tab's
  // URL without re-querying chrome.tabs itself.
  let currentTabUrl = null;
  // 'site' looks up logins for currentTabUrl. 'favorites' is the chrome:// and
  // empty-tab list. Retries, unlock polling, and the post-pairing refresh follow it.
  let popupContext = { kind: 'category', category: 'favorites' };
  let latestPasskeyProxyStatus = null;
  let matchedSiteCredentials = [];
  let currentPageIdentifier = null;
  let identifierRequestId = 0;
  let listOwnedByTotpPicker = false;
  let selectedCategory = 'favorites';
  let browseRequestId = 0;
  lockedTitleEl.textContent = LOCKED_TITLE_TEXT;
  lockedHelperEl.textContent = LOCKED_HELPER_TEXT;

  // Status and site lookup arrive independently. Recompute from current context
  // at either callback, including when switching into search or global browsing.
  function renderPasskeyProxyStatus() {
    const el = document.getElementById('passkey-proxy-status');
    el.textContent = passkeyProxyStatusText(latestPasskeyProxyStatus, popupContext, matchedSiteCredentials);
    el.hidden = !el.textContent;
  }

  const PANEL_SECTIONS = [
    lockedPanel,
    appUnavailablePanel,
    emptyPanel,
    listPanel,
    pairingPanel,
    passkeyCheckingPanel,
    document.getElementById('passkey-failure-panel'),
    passkeySavePanel,
    passkeyReplacePanel,
    passkeyUsePanel,
    passkeyPickerPanel,
    promptPanel
  ];

  const PASSKEY_PANELS = ['passkey-failure', 'passkey-checking', 'passkey-save', 'passkey-replace', 'passkey-use', 'passkey-picker'];

  function showMenuView(name) {
    Object.keys(popupMenuViews).forEach((key) => {
      popupMenuViews[key].hidden = key !== name;
    });
  }

  function closePopupMenu() {
    popupMenu.hidden = true;
    popupMenuBtn.setAttribute('aria-expanded', 'false');
    const lockError = document.getElementById('lock-vault-error');
    if (lockError) lockError.hidden = true;
    showMenuView('root');
  }

  function showPanel(name) {
    if (pendingPasskeyOwnsPopup && !PASSKEY_PANELS.includes(name)) return;
    if (promptOwnsPopup && name !== 'prompt' && !PASSKEY_PANELS.includes(name)) return;
    for (const section of PANEL_SECTIONS) {
      section.hidden = true;
    }
    openMypassHelper.hidden = true;
    // The locked view is only the key, the title, the subtitle and Unlock. With no
    // Retry button the popup has to notice an unlock by itself, so the locked panel
    // polls; every other panel stops that single timer.
    if (name === 'locked') {
      addTotpSection.hidden = true;
      // A tick must not run while the Unlock wait is polling: sendBrowse() cancels that
      // wait and would leave the button disabled with the waiting text.
      startLockedPolling(() => { if (unlockPollTimer === null) reloadCurrentContext(); });
    } else {
      stopLockedPolling();
      // Leaving the locked panel drops any error text, so the next present starts from the default.
      lockedHelperEl.textContent = LOCKED_HELPER_TEXT;
    }
    // Approval owns the popup; unrelated actions must not interrupt a ceremony.
    if (PASSKEY_PANELS.includes(name) || name === 'prompt') {
      addTotpSection.hidden = true;
      generatePasswordSection.hidden = true;
    }
    document.getElementById('passkey-diagnostics').hidden = PASSKEY_PANELS.includes(name) || name === 'prompt';
    document.getElementById('site-passkey-hint').hidden = true;
    // Search, categories, and new-item stay on the browse panels. The "More" menu (Help)
    // also stays on pairing and "app not found". The locked panel shows no menu button at
    // all: it is only the key, title, subtitle and Unlock, and the 2 s poll would close an
    // open menu anyway.
    const browse = name === 'credential-list' || name === 'empty';
    const menuOnly = name === 'app-unavailable' || name === 'pairing';
    browserChrome.hidden = !browse && !menuOnly;
    vaultSearch.hidden = !browse;
    openGeneratorBtn.hidden = !browse;
    addEntryBtn.hidden = !browse;
    categoryWrap.hidden = !browse;
    if (!browse) {
      closePopupMenu();
      categoryMenu.hidden = true;
      addEntryMenu.hidden = true;
    }
    switch (name) {
      case 'prompt':
        promptPanel.hidden = false;
        break;
      case 'passkey-failure':
        document.getElementById('passkey-failure-panel').hidden = false;
        break;
      case 'passkey-checking':
        passkeyCheckingPanel.hidden = false;
        break;
      case 'passkey-save':
        passkeySavePanel.hidden = false;
        break;
      case 'passkey-replace':
        passkeyReplacePanel.hidden = false;
        break;
      case 'passkey-use':
        passkeyUsePanel.hidden = false;
        break;
      case 'passkey-picker':
        passkeyPickerPanel.hidden = false;
        break;
      case 'locked':
        lockedPanel.hidden = false;
        break;
      case 'app-unavailable':
        appUnavailablePanel.hidden = false;
        break;
      case 'empty':
        emptyPanel.hidden = false;
        break;
      case 'credential-list':
        listPanel.hidden = false;
        break;
      case 'pairing':
        pairingPanel.hidden = false;
        break;
      default:
        break;
    }
  }

  function setPairingError(text) {
    if (text) {
      pairingErrorEl.textContent = text;
      pairingErrorEl.hidden = false;
    } else {
      pairingErrorEl.textContent = '';
      pairingErrorEl.hidden = true;
    }
  }

  function showPairingPanel() {
    showPanel('pairing');
  }

  function hidePairingPanel() {
    codeEntry.hidden = true;
    codeInput.value = '';
    setPairingError('');
  }

  function resetPairingFlow() {
    codeEntry.hidden = true;
    codeInput.value = '';
    setPairingError('');
  }

  function revealCodeEntry() {
    codeEntry.hidden = false;
  }

  function buildLetterAvatarElement(title) {
    const avatar = document.createElement('div');
    avatar.className = 'letter-avatar';
    avatar.style.backgroundColor = entryVisuals.colorForTitle(title);
    avatar.setAttribute('role', 'img');
    avatar.setAttribute(
      'aria-label',
      title.trim() ? `Icon for ${title}` : 'Icon for untitled entry'
    );
    const letter = document.createElement('span');
    letter.className = 'letter-avatar-letter';
    letter.textContent = entryVisuals.letterFromTitle(title);
    avatar.appendChild(letter);
    return avatar;
  }

  function openSavedEntry(credential, passkeyOnly) {
    if (!isSiteUrl(credential.url)) return;
    chrome.tabs.create({ url: credential.url, active: true }, (tab) => {
      if (passkeyOnly || !tab || typeof tab.id !== 'number') {
        window.close();
        return;
      }
      chrome.runtime.sendMessage({
        type: 'armOpenAndFill',
        tabId: tab.id,
        id: credential.id,
        url: credential.url
      }, () => window.close());
    });
  }

  function addressFillError(response) {
    if (response && response.type === 'locked') return 'Unlock MyPass, then select the address again.';
    if (response && response.type === 'pairingRequired') return 'Connect this browser to MyPass, then try again.';
    const messages = {
      address_no_form: 'No supported address form on this page. Open a form with standard address autocomplete fields. Address filling is available in the main page only.',
      address_page_changed: 'The page or selected tab changed. Return to the address form and select the address again.',
      address_form_changed: 'The address form changed. Review any fields already filled, then select the address again.',
      address_no_values: 'No saved address values match the fields in this form. Edit this address in MyPass.',
    };
    return messages[response && response.code] || 'Could not fill the address. Check that MyPass is unlocked, then select the address again.';
  }

  function beginAddressFill(credential) {
    const requestId = ++browseRequestId;
    listEl.innerHTML = '';
    listHeaderEl.textContent = 'Checking address forms…';
    const chooser = document.createElement('li');
    chooser.className = 'address-fill-chooser';
    listEl.appendChild(chooser);
    const messageEl = document.createElement('p');
    messageEl.setAttribute('role', 'status');
    chooser.appendChild(messageEl);
    const fillFromPage = () => {
      chrome.runtime.sendMessage({ type: 'prepareAddressFill' }, (response) => {
        const runtimeError = chrome.runtime.lastError;
        if (requestId !== browseRequestId) return;
        const back = document.createElement('button');
        back.type = 'button';
        back.className = 'btn-secondary address-fill-back';
        back.textContent = 'Back to addresses';
        back.addEventListener('click', () => selectCategory('addresses'));
        if (runtimeError || !response || response.type !== 'addressForms') {
          listHeaderEl.textContent = 'Address not filled';
          messageEl.textContent = addressFillError(response);
          chooser.appendChild(back);
          return;
        }
        listHeaderEl.textContent = 'Fill address';
        messageEl.textContent = 'Send “' + credential.title + '” to ' + new URL(response.url).host + '. Choose the form to fill:';
        const focused = response.forms.filter((form) => form.focused);
        const forms = focused.length === 1 ? focused : response.forms;
        const buttons = [];
        forms.forEach((form) => {
          const button = document.createElement('button');
          button.type = 'button';
          button.className = 'btn-secondary address-fill-option';
          button.textContent = 'Fill ' + form.label.toLowerCase() + ' (' + form.fields + ' fields)';
          button.addEventListener('click', (event) => {
            if (!event.isTrusted) return;
            buttons.forEach((item) => { item.disabled = true; });
            messageEl.textContent = 'Filling address…';
            chrome.runtime.sendMessage({ type: 'fillAddress', token: response.token,
              groupId: form.id, id: credential.id }, (result) => {
              const failed = chrome.runtime.lastError;
              if (requestId !== browseRequestId) return;
              if (!failed && result && result.type === 'addressFilled') {
                messageEl.textContent = 'Filled ' + result.filled + ' fields. Review the form before submitting.';
              } else {
                messageEl.textContent = addressFillError(result);
              }
            });
          });
          buttons.push(button);
          chooser.appendChild(button);
        });
        chooser.appendChild(back);
      });
    };
    const area = chrome.storage && chrome.storage.local;
    const settingsKey = autofillSettings.AUTOFILL_SETTINGS_KEY;
    if (!area || typeof area.get !== 'function') {
      fillFromPage();
      return;
    }
    area.get([settingsKey], (stored) => {
      if (requestId !== browseRequestId) return;
      if (chrome.runtime && chrome.runtime.lastError) {
        fillFromPage();
        return;
      }
      const settings = autofillSettings.normalizeAutofillSettings(stored && stored[settingsKey]);
      if (settings.showAddresses) {
        fillFromPage();
        return;
      }
      listHeaderEl.textContent = 'Address suggestions are off';
      messageEl.textContent = 'Turn on Show addresses in Autofill and save, then select the address again.';
      const back = document.createElement('button');
      back.type = 'button';
      back.className = 'btn-secondary address-fill-back';
      back.textContent = 'Back to addresses';
      back.addEventListener('click', () => selectCategory('addresses'));
      chooser.appendChild(back);
    });
  }

  renderCredentials = function (credentials, url, options) {
    listOwnedByTotpPicker = false;
    const openSavedUrl = !!(options && options.openSavedUrl);
    listEl.innerHTML = '';
    credentials.forEach((credential) => {
      const item = document.createElement('li');
      item.className = 'credential-row' + (options && options.identifierOffer ? ' credential-row-unsaved' : '');

      item.appendChild(buildLetterAvatarElement(credential.title));

      const textCol = document.createElement('div');
      textCol.className = 'credential-text';

      const title = document.createElement('div');
      title.className = 'credential-title';
      title.textContent = credential.title;
      textCol.appendChild(title);

      if (credential.username) {
        const username = document.createElement('div');
        username.className = 'credential-username';
        username.textContent = credential.username;
        textCol.appendChild(username);
      }

      item.appendChild(textCol);
      if (credential.entryType === 'Address') {
        item.tabIndex = 0;
        item.setAttribute('role', 'button');
        item.setAttribute('aria-label', 'Fill address: ' + credential.title);
        item.addEventListener('keydown', (event) => {
          if (event.isTrusted && (event.key === 'Enter' || event.key === ' ')) {
            event.preventDefault();
            beginAddressFill(credential);
          }
        });
      }

      const totpUrl = openSavedUrl ? credential.url : url;
      if (shouldShowTotpCircle(credential.hasTotp) && isSiteUrl(totpUrl)) {
        item.appendChild(buildTotpCircleElement(credential, totpUrl));
      }

      if (credential.hasPasskey) {
        item.appendChild(buildPasskeyBadgeElement());
      }

      item.addEventListener('click', (event) => {
        if (credential.entryType === 'Address') {
          if (event.isTrusted) beginAddressFill(credential);
          return;
        }
        if (openSavedUrl) {
          openSavedEntry(credential, !!(options && options.passkeyOnly));
          return;
        }
        chrome.runtime.sendMessage(
          { type: 'fillCredential', id: credential.id, url },
          () => {
            window.close();
          }
        );
      });
      listEl.appendChild(item);
    });
  };

  function startPairingFlow() {
    resetPairingFlow();
    chrome.runtime.sendMessage({ type: 'startPairing' }, (startResp) => {
      if (startResp && startResp.type === 'pairingStarted') {
        writePairingStartedAt(() => {
          revealCodeEntry();
        });
        return;
      }
      if (startResp && startResp.type === 'locked') {
        setPairingError('MyPass is locked.');
        return;
      }
      setPairingError('Could not start pairing. Is MyPass running and unlocked?');
    });
  }

  function submitPairingCode() {
    setPairingError('');
    const code = normalizePairingCode(codeInput.value);
    chrome.runtime.sendMessage({ type: 'completePairing', code }, (completeResp) => {
      if (completeResp && completeResp.type === 'paired') {
        clearPairingStartedAt(() => {
          hidePairingPanel();
          reloadCurrentContext();
        });
        return;
      }
      if (completeResp && completeResp.type === 'error' && completeResp.message === 'invalid_code') {
        clearPairingStartedAt(() => {
          setPairingError('Invalid code. Tap Connect to try again.');
        });
        return;
      }
      setPairingError('Could not complete pairing. Is MyPass running and unlocked?');
    });
  }

  function hideAppUnavailableActions() {
    openMypassHelper.hidden = true;
  }

  function loadForActiveTab(tab) {
    currentTabUrl = tab && isSiteUrl(tab.url) ? tab.url : null;
    buildCategoryMenu();
    if (currentTabUrl) {
      requestPageTotpCandidate(tab.id);
      requestPageNewPasswordField(tab.id);
      requestPageIdentifier(tab.id);
      selectCategory('thisSite');
      return;
    }
    selectCategory('favorites');
  }

  function presentLockedPanel() {
    showPanel('locked');
    lockedUnlockBtn.textContent = UNLOCK_BUTTON_LABEL;
    lockedUnlockBtn.onclick = () => requestUnlock();
  }

  function reloadCurrentContext() {
    if (popupContext.kind === 'search') querySearch(popupContext.query);
    else if (popupContext.kind === 'site') queryCredentials(popupContext.url);
    else queryCategory(popupContext.category || 'favorites');
  }

  function retryCredentialsForActiveTab() {
    listEl.innerHTML = '';
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      loadForActiveTab(tabs[0]);
    });
  }

  function showAppUnavailableActions(url, response) {
    hidePairingPanel();
    listEl.innerHTML = '';
    appUnavailableStatusEl.textContent = bridgeStatus.classifyBridgeConnectionError(response) === 'native_host_not_found'
      ? HOST_UNAVAILABLE_STATUS_TEXT : APP_UNAVAILABLE_STATUS_TEXT;
    showPanel('app-unavailable');
    openMypassHelper.hidden = true;
    retryBtn.onclick = () => {
      if (url) {
        listEl.innerHTML = '';
        queryCredentials(url);
      } else {
        retryCredentialsForActiveTab();
      }
    };
    openMypassBtn.onclick = () => {
      openMypassHelper.hidden = false;
    };
  }

  function handleCredentialsResponse(response, url) {
    if (pendingPasskeyOwnsPopup || promptOwnsPopup) return;
    const kind = bridgeStatus.classifyGetCredentialsResponse(response);
    matchedSiteCredentials = kind === 'credentialList' && popupContext.kind === 'site'
      ? response.credentials : [];
    renderPasskeyProxyStatus();
    if (kind === 'error') {
      if (shouldShowAppUnavailableActions(kind)) {
        showAppUnavailableActions(url, response);
      }
      return;
    }
    hideAppUnavailableActions();
    if (kind === 'locked') {
      presentLockedPanel();
      return;
    }
    if (kind === 'pairingRequired') {
      showPairingPanel();
      connectBtn.onclick = startPairingFlow;
      submitCodeBtn.onclick = () => submitPairingCode();
      readPairingStartedAt((startedAt) => {
        if (isPairingChallengeFresh(startedAt, Date.now())) {
          revealCodeEntry();
        }
      });
      return;
    }
    if (kind === 'credentialList') {
      const browsing = popupContext.kind !== 'site';
      listHeaderEl.textContent = browsing ? categoryTitle(popupContext.category) : LIST_HEADER_TEXT;
      if (popupContext.kind === 'search') listHeaderEl.textContent = 'Search';
      emptyTitleEl.textContent = emptyTitleForContext();
      if (response.credentials.length === 0) {
        showPanel('empty');
        hideIdentifierOffer();
        return;
      }
      showPanel('credential-list');
      const passkeyOnly = browsing && popupContext.kind === 'category' && popupContext.category === 'passkeys';
      const identifierOffer = browsing ? null : pageIdentifier.unsavedPageIdentifier(currentPageIdentifier, response.credentials);
      renderCredentials(response.credentials, url, browsing ? { openSavedUrl: true, passkeyOnly } : { identifierOffer: identifierOffer });
      if (identifierOffer) paintIdentifierOffer(identifierOffer);
      else hideIdentifierOffer();
      const hint = document.getElementById('site-passkey-hint');
      if (hint) {
        hint.hidden = !passkeyOnly && (browsing || !response.credentials.some((entry) => entry.hasPasskey));
        hint.textContent = passkeyOnly ? 'Select an entry to open its website.' : '';
        if (!passkeyOnly) hint.hidden = true;
      }
    }
  }

  let unlockPollTimer = null;

  function stopUnlockPolling() {
    if (unlockPollTimer !== null) {
      clearTimeout(unlockPollTimer);
      unlockPollTimer = null;
    }
  }

  function endUnlockWait(response, url) {
    stopUnlockPolling();
    lockedHelperEl.textContent = LOCKED_HELPER_TEXT;
    lockedUnlockBtn.disabled = false;
    handleCredentialsResponse(response, url);
  }

  function pollForUnlock(attempt) {
    const message = browseMessage();
    chrome.runtime.sendMessage(message, (response) => {
      if (!shouldContinueUnlockPolling(bridgeStatus.classifyGetCredentialsResponse(response), attempt)) {
        endUnlockWait(response, popupContext.kind === 'site' ? popupContext.url : null);
        return;
      }
      unlockPollTimer = setTimeout(() => pollForUnlock(attempt + 1), UNLOCK_POLL_INTERVAL_MS);
    });
  }

  function requestUnlock() {
    chrome.runtime.sendMessage({ type: 'requestUnlock' }, (response) => {
      const kind = classifyRequestUnlockResponse(response);
      if (kind === 'notNeeded') {
        reloadCurrentContext();
        return;
      }
      if (kind !== 'presented') {
        // Unreachable app or a failed request: say so instead of leaving the click silent.
        // The next present of the locked panel restores LOCKED_HELPER_TEXT.
        lockedHelperEl.textContent = UNLOCK_UNREACHABLE_TEXT;
        return;
      }
      // MyPass is now showing its unlock screen. Wait for it rather than leaving the
      // user on a stale panel whose only action is Retry.
      lockedHelperEl.textContent = WAITING_FOR_UNLOCK_TEXT;
      lockedUnlockBtn.disabled = true;
      pollForUnlock(0);
    });
  }

  function emptyTitleForContext() {
    if (popupContext.kind === 'search') return NO_SEARCH_RESULTS_TEXT;
    if (popupContext.kind === 'site') return EMPTY_LIST_TITLE_TEXT;
    if (popupContext.category === 'favorites') return NO_FAVORITES_TEXT;
    if (popupContext.category === 'addresses') return 'No addresses. Add one in MyPass using Add > Address.';
    return NO_ITEMS_TEXT;
  }

  function browseMessage() {
    if (popupContext.kind === 'search') return { type: 'searchEntries', query: popupContext.query };
    if (popupContext.kind === 'site') return { type: 'getCredentials', url: popupContext.url };
    return { type: 'listEntries', category: popupContext.category || 'favorites' };
  }

  function sendBrowse(message, url) {
    const requestId = ++browseRequestId;
    stopUnlockPolling();
    matchedSiteCredentials = [];
    renderPasskeyProxyStatus();
    chrome.runtime.sendMessage(message, (response) => {
      if (requestId !== browseRequestId) return;
      handleCredentialsResponse(response, url);
    });
  }

  function queryCredentials(url) {
    popupContext = { kind: 'site', url };
    currentTabUrl = url;
    sendBrowse({ type: 'getCredentials', url }, url);
  }

  function queryCategory(category) {
    popupContext = { kind: 'category', category };
    sendBrowse({ type: 'listEntries', category }, null);
  }

  function querySearch(query) {
    popupContext = { kind: 'search', query, category: selectedCategory };
    sendBrowse({ type: 'searchEntries', query }, null);
  }

  function buildCategoryMenu() {
    categoryMenu.innerHTML = '';
    BROWSER_CATEGORIES.forEach((category) => {
      if (category.id === 'thisSite' && !isSiteUrl(currentTabUrl)) return;
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'category-option';
      button.textContent = (category.id === selectedCategory ? '✓ ' : '') + category.title;
      button.addEventListener('click', () => selectCategory(category.id));
      categoryMenu.appendChild(button);
    });
  }

  function selectCategory(categoryId, options) {
    selectedCategory = categoryId;
    categoryBtn.textContent = categoryTitle(categoryId) + ' ▾';
    categoryMenu.hidden = true;
    if (!options || options.keepSearch !== true) vaultSearch.value = '';
    buildCategoryMenu();
    if (categoryId === 'thisSite' && isSiteUrl(currentTabUrl)) queryCredentials(currentTabUrl);
    else queryCategory(categoryId === 'thisSite' ? 'favorites' : categoryId);
  }

  function openCenteredExtensionWindow(relativeUrl, width, height) {
    if (!chrome.windows || typeof chrome.windows.create !== 'function' || !chrome.runtime || typeof chrome.runtime.getURL !== 'function') {
      return;
    }
    const screen = window.screen || {};
    const availWidth = screen.availWidth || width;
    const availHeight = screen.availHeight || height;
    chrome.windows.create({
      url: chrome.runtime.getURL(relativeUrl),
      type: 'popup',
      width: width,
      height: height,
      left: Math.round((screen.availLeft || 0) + Math.max(0, (availWidth - width) / 2)),
      top: Math.round((screen.availTop || 0) + Math.max(0, (availHeight - height) / 2)),
      focused: true
    }, () => {
      if (!chrome.runtime.lastError) window.close();
    });
  }

  function buildAddMenu() {
    addEntryMenu.innerHTML = '';
    const icons = {
      Login: '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M14.2 3.2a6.3 6.3 0 0 0-5.9 8.3L3.2 16.6V21h4.2l1.3-1.3v-2.2h2.2l1.2-1.2a6.3 6.3 0 1 0 2.1-13.1zm1.5 5.1a1.35 1.35 0 1 1 0 2.7 1.35 1.35 0 0 1 0-2.7z"/></svg>',
      Card: '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><rect x="2.2" y="5" width="19.6" height="14" rx="2.4" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M2.2 9.6h19.6" stroke="currentColor" stroke-width="1.8"/></svg>',
      Notes: '<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M6 3.2h8.2L19.8 9v11.2a.8.8 0 0 1-.8.8H6a.8.8 0 0 1-.8-.8V4a.8.8 0 0 1 .8-.8z"/><path d="M14 3.6V9h5.2" fill="none" stroke="#fff" stroke-width="1.4"/><path d="M8 13.2h8M8 16.6h5.5" fill="none" stroke="#fff" stroke-width="1.4" stroke-linecap="round"/></svg>'
    };
    [
      { type: 'Login', title: 'New login', iconClass: 'add-entry-icon-login' },
      { type: 'Card', title: 'New card', iconClass: 'add-entry-icon-card' },
      { type: 'Notes', title: 'New note', iconClass: 'add-entry-icon-note' }
    ].forEach((choice) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'add-entry-option';
      const icon = document.createElement('span');
      icon.className = 'add-entry-icon ' + choice.iconClass;
      icon.setAttribute('aria-hidden', 'true');
      icon.innerHTML = icons[choice.type];
      const label = document.createElement('span');
      label.className = 'add-entry-label';
      label.textContent = choice.title;
      button.appendChild(icon);
      button.appendChild(label);
      button.addEventListener('click', () => openAddEntry(choice.type));
      addEntryMenu.appendChild(button);
    });
  }

  function openAddEntry(entryType) {
    addEntryMenu.hidden = true;
    const params = new URLSearchParams({ type: entryType });
    if (entryType === 'Login' && isSiteUrl(currentTabUrl)) params.set('url', currentTabUrl);
    const height = entryType === 'Notes' ? 640 : entryType === 'Card' ? 680 : 760;
    openCenteredExtensionWindow('create.html?' + params.toString(), 520, height);
  }

  function openGenerator() {
    categoryMenu.hidden = true;
    addEntryMenu.hidden = true;
    const params = new URLSearchParams({ purpose: 'autofill' });
    if (isSiteUrl(currentTabUrl)) params.set('url', currentTabUrl);
    openCenteredExtensionWindow('generator.html?' + params.toString(), 420, 640);
  }

  renderTotpPicker = function (credentials, url, otpauthOrSecret) {
    listOwnedByTotpPicker = true;
    hideIdentifierOffer();
    showPanel('credential-list');
    listEl.innerHTML = '';
    credentials.forEach((credential) => {
      const item = document.createElement('li');
      item.className = 'credential-row';
      item.appendChild(buildLetterAvatarElement(credential.title));

      const textCol = document.createElement('div');
      textCol.className = 'credential-text';
      const title = document.createElement('div');
      title.className = 'credential-title';
      title.textContent = credential.title;
      textCol.appendChild(title);
      item.appendChild(textCol);

      if (credential.hasTotp) {
        const badge = document.createElement('span');
        badge.className = 'totp-badge';
        badge.textContent = '2FA';
        item.appendChild(badge);
      }

      item.addEventListener('click', () => {
        chrome.runtime.sendMessage(
          { type: 'proposeSaveTotp', url, otpauthOrSecret, entryId: credential.id },
          (response) => {
            addTotpErrorEl.hidden = false;
            addTotpErrorEl.textContent = describeSaveProposalOutcome(response);
          }
        );
      });
      listEl.appendChild(item);
    });
  };

  routeTotpCandidate = function (url, otpauthOrSecret, credentials) {
    const reportOutcome = (response) => {
      addTotpErrorEl.hidden = false;
      addTotpErrorEl.textContent = describeSaveProposalOutcome(response);
    };
    if (credentials.length === 0) {
      chrome.runtime.sendMessage({ type: 'proposeSaveTotp', url, otpauthOrSecret }, reportOutcome);
    } else if (credentials.length === 1) {
      chrome.runtime.sendMessage({ type: 'proposeSaveTotp', url, otpauthOrSecret, entryId: credentials[0].id }, reportOutcome);
    } else {
      renderTotpPicker(credentials, url, otpauthOrSecret);
    }
  };

  handleAddTotpClick = function (activeTabUrl) {
    const raw = totpPasteInput.value;
    const validation = validateTotpPasteInput(raw);
    if (!validation.valid) {
      addTotpErrorEl.textContent = 'Enter a valid authenticator URL or secret key';
      addTotpErrorEl.hidden = false;
      return;
    }
    addTotpErrorEl.hidden = true;
    chrome.runtime.sendMessage({ type: 'getCredentials', url: activeTabUrl }, (response) => {
      // Same trust model as the password-save flow: never fire a save/picker proposal
      // unless the vault is actually unlocked and paired. A locked/pairingRequired/error
      // response must surface a status here, not silently fall through to a blind
      // proposeSaveTotp (that would happen if we treated "not credentialList" as "zero
      // matches", since routeTotpCandidate's zero-match branch sends the proposal anyway).
      const kind = bridgeStatus.classifyGetCredentialsResponse(response);
      if (kind === 'locked') {
        addTotpErrorEl.textContent = 'Unlock MyPass first';
        addTotpErrorEl.hidden = false;
        return;
      }
      if (kind === 'pairingRequired') {
        addTotpErrorEl.textContent = 'Connect to MyPass first';
        addTotpErrorEl.hidden = false;
        return;
      }
      if (kind !== 'credentialList') {
        addTotpErrorEl.textContent = "Couldn't reach MyPass";
        addTotpErrorEl.hidden = false;
        return;
      }
      routeTotpCandidate(activeTabUrl, validation.value, response.credentials);
    });
  };

  // --- Passkey ceremony panels ---------------------------------------------------
  // The pending record lives in chrome.storage.session (written by background.js), not
  // in this popup's memory: the popup is opened fresh by chrome.action.openPopup() after
  // the ceremony has already started, and may be closed and reopened mid-ceremony.

  // The background worker exclusively owns the pending record. A delayed callback
  // for this popup may arrive after another ceremony has already reserved that record.
  // Closing the UI must never remove the next request's state.
  function showPasskeyFailure(failure) {
    visiblePasskeyFailure = failure;
    pendingPasskeyOwnsPopup = true;
    const message = failure.kind === 'get'
      ? 'The browser could not finish signing in. Cancel the website request and try again.'
      : 'The browser could not finish saving this passkey. Cancel the website request and try again. A local passkey may already exist; check the website before replacing it.';
    document.getElementById('passkey-failure-message').textContent = message;
    document.getElementById('passkey-failure-close-btn').onclick = () => window.close();
    showPanel('passkey-failure');
  }

  function submitPasskeyApproval(message, kind) {
    activePasskeyRequestId = message.requestId;
    renderPasskeyCeremonyPanel({ phase: 'completing', requestId: message.requestId });
    chrome.runtime.sendMessage(message, (response) => {
      const error = chrome.runtime.lastError;
      if (activePasskeyRequestId !== message.requestId) return;
      if (!error && response && !response.error && response.type === (kind === 'create' ? 'passkeyCreateComplete' : 'passkeyGetComplete')) {
        window.close();
      } else {
        showPasskeyFailure({ kind, requestId: message.requestId });
      }
    });
  }

  sendPasskeyCreateConfirm = function (requestId, entryId, replace) {
    submitPasskeyApproval({ type: 'passkeyCreateConfirm', requestId, entryId, replace: !!replace }, 'create');
  };

  sendPasskeyGetApprove = function (requestId, entryId) {
    submitPasskeyApproval({ type: 'passkeyGetApprove', requestId, entryId }, 'get');
  };

  sendPasskeyCancel = function (requestId) {
    chrome.runtime.sendMessage(
      { type: 'passkeyCancelUI', requestId },
      () => window.close()
    );
  };

  renderPasskeyPicker = function (pending) {
    passkeyPickerListEl.innerHTML = '';
    (pending.matches || []).forEach((credential) => {
      const item = document.createElement('li');
      item.className = 'credential-row';
      item.appendChild(buildLetterAvatarElement(credential.title));

      const textCol = document.createElement('div');
      textCol.className = 'credential-text';
      const title = document.createElement('div');
      title.className = 'credential-title';
      title.textContent = credential.title;
      textCol.appendChild(title);
      if (credential.username) {
        const username = document.createElement('div');
        username.className = 'credential-username';
        username.textContent = credential.username;
        textCol.appendChild(username);
      }
      item.appendChild(textCol);

      if (credential.hasPasskey) {
        item.appendChild(buildPasskeyBadgeElement());
      }

      item.addEventListener('click', () => {
        if (pending.kind === 'create') {
          // An entry that already carries a passkey must go through the same "Replace
          // passkey?" confirm every other route to that panel uses — never straight to
          // `sendPasskeyCreateConfirm`, which would silently overwrite it.
          if (credential.hasPasskey) {
            showPasskeySaveOrReplacePanel(pending, 'passkey-replace', credential.id, credential);
          } else {
            showPasskeySaveOrReplacePanel(pending, 'passkey-save', credential.id, credential);
          }
        } else {
          sendPasskeyGetApprove(pending.requestId, credential.id);
        }
      });
      passkeyPickerListEl.appendChild(item);
    });
  };

  function renderPasskeyDetails(prefix, pending, overrideMatch) {
    const matches = pending.matches || [];
    const match = overrideMatch || (matches.length === 1 ? matches[0] : null);
    document.getElementById(prefix + '-summary').textContent = match ? '' : 'A new entry will be created.';
    const details = document.getElementById(prefix + '-detail');
    details.innerHTML = '';
    const rows = [
      ['Entry', match ? match.title : pending.rpId],
      ['Account', match && match.username ? match.username : (pending.userName || 'Not provided')],
      ['Website', pending.rpId || 'Unknown website'],
      ['Origin', pending.originUrl || 'Unknown origin']
    ];
    for (const [label, value] of rows) {
      const term = document.createElement('dt');
      term.textContent = label;
      const description = document.createElement('dd');
      description.textContent = value;
      details.appendChild(term);
      details.appendChild(description);
    }
  }

  /// Shared by the direct 0/1-match routing in `renderPasskeyCeremonyPanel` and by the
  /// picker's row click handler (which needs to show Save/Replace for the entry the user
  /// just picked, rather than confirming immediately).
  function showPasskeySaveOrReplacePanel(pending, panel, entryId, overrideMatch) {
    const isReplace = panel === 'passkey-replace';
    renderPasskeyDetails(isReplace ? 'passkey-replace' : 'passkey-save', pending, overrideMatch);
    document.getElementById(isReplace ? 'passkey-replace-btn' : 'passkey-save-btn').onclick =
      () => sendPasskeyCreateConfirm(pending.requestId, entryId, isReplace);
    document.getElementById(isReplace ? 'passkey-replace-cancel-btn' : 'passkey-cancel-btn').onclick =
      () => sendPasskeyCancel(pending.requestId);
    showPanel(panel);
  }

  renderPasskeyCeremonyPanel = function (pending) {
    if (pending) activePasskeyRequestId = pending.requestId;
    if (pending && (pending.phase === 'checking' || pending.phase === 'completing')) {
      document.getElementById('passkey-checking-title').textContent = pending.phase === 'completing' ? 'Completing passkey request' : 'Preparing passkey request';
      document.getElementById('passkey-checking-message').textContent = pending.phase === 'completing' ? 'Waiting for the browser to finish. Keep the website open.' : 'Waiting for MyPass. You will be asked to approve before a passkey is saved or used.';
      document.getElementById('passkey-checking-cancel-btn').onclick = () => sendPasskeyCancel(pending.requestId);
      showPanel('passkey-checking');
      return true;
    }
    const panel = choosePasskeyPanel(pending);
    if (!panel) return false;

    const summaryText = passkeyCeremonySummary(pending);
    const matches = pending.matches || [];
    const entryId = matches.length === 1 ? matches[0].id : undefined;

    if (panel === 'passkey-picker') {
      document.getElementById('passkey-picker-summary').textContent = summaryText;
      renderPasskeyPicker(pending);
      document.getElementById('passkey-picker-cancel-btn').onclick = () => sendPasskeyCancel(pending.requestId);
      showPanel('passkey-picker');
      return true;
    }

    if (panel === 'passkey-use') {
      renderPasskeyDetails('passkey-use', pending);
      document.getElementById('passkey-approve-btn').onclick = () => sendPasskeyGetApprove(pending.requestId, entryId);
      document.getElementById('passkey-use-cancel-btn').onclick = () => sendPasskeyCancel(pending.requestId);
      showPanel('passkey-use');
      return true;
    }

    showPasskeySaveOrReplacePanel(pending, panel, entryId);
    return true;
  };

  // Returns whether a ceremony took over the popup, so the caller can skip the ordinary
  // credential-list flow entirely rather than racing it.
  rehydratePendingPasskeyCeremony = function (cb) {
    const area = getPairingStorageArea();
    if (!area || !area.get) {
      cb(false);
      return;
    }
    const revision = pendingPasskeyRevision;
    area.get([PENDING_PASSKEY_KEY, LAST_PASSKEY_FAILURE_KEY], (result) => {
      if (revision !== pendingPasskeyRevision) { cb(pendingPasskeyOwnsPopup); return; }
      const pending = result && result[PENDING_PASSKEY_KEY];
      const failure = result && result[LAST_PASSKEY_FAILURE_KEY];
      if (failure && (!pending || pending.requestId === failure.requestId)) { showPasskeyFailure(failure); cb(true); return; }
      visiblePasskeyFailure = null;
      if (!pending) {
        pendingPasskeyOwnsPopup = false;
        cb(false);
        return;
      }
      pendingPasskeyOwnsPopup = !!(pending.phase === 'checking' || pending.phase === 'completing' || choosePasskeyPanel(pending));
      cb(renderPasskeyCeremonyPanel(pending));
    });
  };

  // Only the passkey proxy status diagnostics stay in the popup; the settings that used to
  // sit beside them moved to the options page (app/app.js).
  initPasskeySettingsToggle = function () {
    if (passkeySettingsInitialized) return;
    passkeySettingsInitialized = true;
    const refreshStatus = () => {
      chrome.runtime.sendMessage({ type: 'getPasskeyProxyStatus' }, (status) => {
        void chrome.runtime.lastError;
        latestPasskeyProxyStatus = status;
        renderPasskeyProxyStatus();
      });
      chrome.runtime.sendMessage({ type: 'getBrowserManagerDiagnostics' }, (result) => {
        void chrome.runtime.lastError;
        const el = document.getElementById('browser-manager-status');
        el.textContent = browserManagerDiagnosticsText(result);
        el.hidden = !el.textContent;
      });
    };
    refreshStatus();
  };

  // Fire-and-forget: lets the background service worker resync its cache and
  // attach/detach against the app's current status on every popup open, rather than
  // only at service-worker startup. The no-op callback reads chrome.runtime.lastError
  // so Chrome doesn't log an "Unchecked runtime.lastError" warning if the message port
  // closes before a response (the handler never sends one for this message).
  //
  // Deliberately unconditional and separate from `initPasskeySettingsToggle`: it has no
  // UI side effects, so it must run even when a ceremony panel takes over the popup
  // (`rehydratePendingPasskeyCeremony` returning `tookOver === true`) — that was
  // previously the one case where this send was skipped, which meant the proxy almost
  // never detached during real passkey use (2026-08-23 fix).
  refreshPasskeyProxy = function () {
    chrome.runtime.sendMessage({ type: 'refreshPasskeyProxy' }, () => { void chrome.runtime.lastError; });
  };

  addTotpBtn.onclick = () => handleAddTotpClick(currentTabUrl);

  // The Add-2FA section starts hidden (popup.html) and only appears when there is an
  // actual 2FA candidate in context: detected on the current page (this function). It must
  // stay hidden on ordinary pages, chrome:// pages, and tabs with no content script.
  function hideIdentifierOffer() {
    if (identifierOfferEl) identifierOfferEl.hidden = true;
    if (identifierOfferActionsEl) identifierOfferActionsEl.innerHTML = '';
  }

  function paintIdentifierOffer(offer) {
    if (!identifierOfferEl || !offer) { hideIdentifierOffer(); return; }
    identifierOfferEl.hidden = false;
    identifierOfferTextEl.textContent = offer.value
      + ' is not saved for this site. Add it to an existing item, or create a new one.';
    identifierOfferActionsEl.innerHTML = '';
    matchedSiteCredentials.forEach((credential) => {
      const add = document.createElement('button');
      add.type = 'button';
      add.className = 'btn-secondary';
      add.textContent = 'Add to ' + (credential.title || 'item');
      add.addEventListener('click', (event) => {
        if (!event.isTrusted) return;
        addEntryIdentifier(credential, offer);
      });
      identifierOfferActionsEl.appendChild(add);
    });
    const create = document.createElement('button');
    create.type = 'button';
    create.className = 'btn-primary';
    create.textContent = 'New item';
    create.addEventListener('click', (event) => {
      if (!event.isTrusted) return;
      const params = new URLSearchParams({ type: 'Login', username: offer.value });
      if (isSiteUrl(currentTabUrl)) params.set('url', currentTabUrl);
      openCenteredExtensionWindow('create.html?' + params.toString(), 520, 760);
    });
    identifierOfferActionsEl.appendChild(create);
  }

  function refreshSiteIdentifierOffer() {
    if (listOwnedByTotpPicker || popupContext.kind !== 'site' || matchedSiteCredentials.length === 0) {
      if (popupContext.kind !== 'site') hideIdentifierOffer();
      return;
    }
    const offer = pageIdentifier.unsavedPageIdentifier(currentPageIdentifier, matchedSiteCredentials);
    renderCredentials(matchedSiteCredentials, popupContext.url, { identifierOffer: offer });
    if (offer) paintIdentifierOffer(offer);
    else hideIdentifierOffer();
  }

  function addEntryIdentifier(credential, offer) {
    const buttons = Array.from(identifierOfferActionsEl.children || []);
    buttons.forEach((button) => { button.disabled = true; });
    chrome.runtime.sendMessage({
      type: 'addEntryIdentifier',
      id: credential.id,
      url: popupContext.url,
      value: offer.value,
      fieldKey: offer.fieldKey
    }, (result) => {
      void chrome.runtime.lastError;
      if (result && result.type === 'ok') {
        if (!credential.username) credential.username = offer.value;
        credential.identifiers = (credential.identifiers || []).concat([offer.value]);
        refreshSiteIdentifierOffer();
        return;
      }
      identifierOfferTextEl.textContent = (result && result.message)
        ? result.message
        : 'Could not add this to the item. Open MyPass and try again.';
      buttons.forEach((button) => { button.disabled = false; });
    });
  }

  requestPageIdentifier = function (tabId) {
    if (typeof tabId !== 'number' || !chrome.tabs || typeof chrome.tabs.sendMessage !== 'function') return;
    const requestId = ++identifierRequestId;
    chrome.tabs.sendMessage(tabId, { type: 'queryPageIdentifier' }, (resp) => {
      const lastError = chrome.runtime.lastError;
      if (requestId !== identifierRequestId) return;
      currentPageIdentifier = !lastError && resp && resp.value ? { value: resp.value, kind: resp.kind } : null;
      refreshSiteIdentifierOffer();
    });
  };

  requestPageTotpCandidate = function (tabId) {
    if (typeof tabId !== 'number' || typeof chrome.tabs.sendMessage !== 'function') return;
    chrome.tabs.sendMessage(tabId, { type: 'queryTotpCandidate' }, (resp) => {
      // Reading lastError marks it "checked" so Chrome doesn't log an "Unchecked
      // runtime.lastError" warning when the tab has no content script listening
      // (chrome:// pages, the Web Store, etc.).
      const lastError = chrome.runtime.lastError;
      if (lastError || !resp || !resp.candidate) return;
      totpPasteInput.value = resp.candidate;
      addTotpSection.hidden = false;
    });
  };

  // Mirrors requestPageTotpCandidate: asks the active tab's content script whether this
  // page currently has a new-password field, and shows/hides the Generate section
  // accordingly. Restricted to "there is a field to fill" for a concrete reason (Decision
  // 7 of the design spec): without a target field, the only place to put a freshly
  // generated secret would be the system clipboard, and the extension has no bridge call
  // equivalent to the app's own ClipboardAutoClear — so this popup action must never
  // create that secret-in-clipboard state in the first place.
  requestPageNewPasswordField = function (tabId) {
    if (typeof tabId !== 'number' || typeof chrome.tabs.sendMessage !== 'function') return;
    chrome.tabs.sendMessage(tabId, { type: 'queryNewPasswordField' }, (resp) => {
      const lastError = chrome.runtime.lastError;
      if (lastError || !resp || !resp.hasNewPasswordField) return;
      generatePasswordSection.hidden = false;
    });
  };

  // Generates a password via the bridge. The actual fill into the active tab is NOT done
  // here: background.js's handleRuntimeMessage relays a successful generatedPassword
  // response through the exact same relayFillToActiveTab/sendApplyFill path
  // fillCredential already uses (query-the-active-tab, hostname pre-check, scope-to-
  // frameId-0 all included) — this function used to hand-roll that same sequence itself,
  // which is how it ended up as a second, less careful copy missing the hostname check.
  // The response is stripped the same way fillCredential's is, so the generated password
  // never enters this popup's JS runtime at all; `generatedPasswordFilled` is the whole
  // of the success signal, and branching on it rather than on a password's presence is
  // what makes that stripping real instead of merely intended.
  // `url` travels alongside `domain` in the request purely so that relay has a page URL
  // to compare against the active tab's URL; the app only ever reads `domain` from this
  // message (PasswordRulesCatalog.rules(forHost:)), so the extra field is inert on the
  // wire.
  handleGeneratePasswordClick = function (activeTabUrl) {
    generatePasswordErrorEl.hidden = true;
    let domain;
    try {
      domain = new URL(activeTabUrl).hostname;
    } catch (e) {
      generatePasswordErrorEl.textContent = "Couldn't determine this page's domain";
      generatePasswordErrorEl.hidden = false;
      return;
    }
    chrome.runtime.sendMessage({ type: 'generatePassword', domain, url: activeTabUrl }, (response) => {
      if (!response || response.type !== 'generatedPasswordFilled') {
        generatePasswordErrorEl.textContent = describeSaveProposalOutcome(response);
        generatePasswordErrorEl.hidden = false;
        return;
      }
      window.close();
    });
  };

  generatePasswordBtn.onclick = () => handleGeneratePasswordClick(currentTabUrl);

  buildAddMenu();
  categoryBtn.addEventListener('click', () => {
    addEntryMenu.hidden = true;
    categoryMenu.hidden = !categoryMenu.hidden;
  });
  addEntryBtn.addEventListener('click', () => {
    categoryMenu.hidden = true;
    addEntryMenu.hidden = !addEntryMenu.hidden;
  });
  vaultSearch.addEventListener('input', () => {
    const query = vaultSearch.value.trim();
    if (!query) {
      selectCategory(selectedCategory, { keepSearch: true });
      return;
    }
    querySearch(query);
  });
  openGeneratorBtn.addEventListener('click', openGenerator);
  popupMenuBtn.addEventListener('click', () => {
    if (!popupMenu.hidden) {
      closePopupMenu();
      return;
    }
    categoryMenu.hidden = true;
    addEntryMenu.hidden = true;
    showMenuView('root');
    popupMenu.hidden = false;
    popupMenuBtn.setAttribute('aria-expanded', 'true');
  });
  document.getElementById('popup-menu-help-btn').addEventListener('click', () => showMenuView('help'));
  document.getElementById('popup-menu-help-back').addEventListener('click', () => showMenuView('root'));
  document.getElementById('lock-vault-btn').addEventListener('click', () => {
    const button = document.getElementById('lock-vault-btn');
    const el = document.getElementById('lock-vault-error');
    button.disabled = true;
    el.hidden = true;
    chrome.runtime.sendMessage({ type: 'lockVault' }, (result) => {
      void chrome.runtime.lastError;
      button.disabled = false;
      if (result && (result.type === 'ok' || result.type === 'locked')) {
        presentLockedPanel();
        return;
      }
      el.hidden = false;
      el.textContent = result && result.type === 'pairingRequired'
        ? 'Connect this browser to MyPass first.'
        : 'Could not lock MyPass. Open the app and try again.';
    });
  });

  // --- Page prompts (save login / save 2FA / settings) ---------------------------------
  // The background owns the record and its secret; this popup only draws the redacted view it is
  // given and answers with { promptId, action }. Asked for the tab this popup was opened for
  // (currentWindow is correct here: unlike prompt.html, this page IS the toolbar popup of that
  // window), so a prompt raised in another tab is never shown against this one.
  const promptApi = {
    send: (message, cb) => chrome.runtime.sendMessage(message, (response) => {
      void chrome.runtime.lastError;
      cb(response);
    }),
    settings: promptCardApi.chromeSettingsStore(chrome, autofillSettings),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (id) => clearTimeout(id),
    now: () => Date.now(),
    done: () => requestPendingPrompt(true),
    reload: () => requestPendingPrompt(false)
  };

  function releasePrompt() {
    promptOwnsPopup = false;
    if (promptController) {
      promptController.dispose();
      promptController = null;
    }
  }

  function showPromptCard(view) {
    if (pendingPasskeyOwnsPopup) return;
    if (promptController) promptController.dispose();
    promptOwnsPopup = true;
    browseRequestId += 1;
    showPanel('prompt');
    promptController = promptCardApi.render(document, promptCardRoot, view, promptApi);
  }

  // closeWhenEmpty: after the user answered, an empty queue means the job is done and the popup
  // closes (like a finished ceremony). Everywhere else an empty answer just hands the popup back
  // to the ordinary list.
  requestPendingPrompt = function (closeWhenEmpty, knownTab) {
    const revision = ++promptRevision;
    const withTab = (tab) => {
      if (!tab || typeof tab.id !== 'number') {
        if (closeWhenEmpty) window.close();
        return;
      }
      chrome.runtime.sendMessage({ type: 'getPrompt', tabId: tab.id }, (response) => {
        void chrome.runtime.lastError;
        if (revision !== promptRevision) return;
        if (response && response.type === 'prompt') {
          showPromptCard(response.prompt);
          return;
        }
        const hadPrompt = promptOwnsPopup;
        releasePrompt();
        if (closeWhenEmpty) window.close();
        else if (hadPrompt) resumeOrdinaryPopup();
      });
    };
    if (knownTab) { withTab(knownTab); return; }
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      void chrome.runtime.lastError;
      withTab(tabs && tabs[0]);
    });
  };

  // A prompt arriving, expiring or being dropped while this popup is open. A save in flight is
  // never interrupted, and the card already showing a still-pending prompt is left as it is.
  if (chrome.storage.onChanged) chrome.storage.onChanged.addListener((changes, areaName) => {
    if (areaName !== 'session' || !changes.pendingPrompts) return;
    if (promptController && promptController.busy()) return;
    const list = changes.pendingPrompts.newValue;
    if (promptController && Array.isArray(list) && list.some((r) => r && r.id === promptController.id)) return;
    requestPendingPrompt(false);
  });

  // A pending ceremony is checked first and, when present, owns the popup outright: the
  // popup was opened *by* that ceremony, and the user is being asked to make a security
  // decision. Running the ordinary credential lookup alongside it would let whichever
  // callback resolved last win the panel.
  //
  // Wrapped in a named, exported function (rather than run inline) so tests can re-invoke
  // this exact composition after seeding a pending ceremony, instead of hand-rolling the
  // rehydrate/refresh/tookOver sequence themselves — a hand-rolled copy in a test would not
  // notice a regression at this call site (e.g. refreshPasskeyProxy() moving back below the
  // `if (tookOver) return;` line).
  function resumeOrdinaryPopup() {
    releasePrompt();
    emptyTitleEl.textContent = 'Loading…';
    showPanel('empty');
    initPasskeySettingsToggle();
    if (ordinaryPopupInitialized) {
      reloadCurrentContext();
      requestPendingPrompt(false);
      return;
    }
    ordinaryPopupInitialized = true;
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      if (!pendingPasskeyOwnsPopup) loadForActiveTab(tabs[0]);
      // Not awaited: the ordinary list shows at once and a pending prompt takes over when the
      // background answers, so a slow app lookup cannot delay the popup.
      requestPendingPrompt(false, tabs[0]);
    });
  }

  // The popup can already be open while native code prepares the ceremony. Observe
  // both the checking reservation and its ready replacement; openPopup() does not
  // reload an existing popup. Ignore local storage changes and invalidate any old
  // browse response before the security confirmation takes ownership of the UI.
  if (chrome.storage.onChanged) chrome.storage.onChanged.addListener((changes, areaName) => {
    if (areaName !== 'session') return;
    if (changes.vaultLockedNotice) presentLockedPanel();
    if (changes[LAST_PASSKEY_FAILURE_KEY]) {
      pendingPasskeyRevision += 1;
      const failure = changes[LAST_PASSKEY_FAILURE_KEY].newValue;
      // A late failure from a cancelled request must not cover the next approval.
      if (failure && pendingPasskeyOwnsPopup && activePasskeyRequestId !== null
        && failure.requestId !== activePasskeyRequestId) return;
      visiblePasskeyFailure = failure || null;
      if (failure) { showPasskeyFailure(failure); return; }
    }
    if (!changes[PENDING_PASSKEY_KEY]) return;
    pendingPasskeyRevision += 1;
    browseRequestId += 1;
    const pending = changes[PENDING_PASSKEY_KEY].newValue;
    if (!pending && visiblePasskeyFailure) { showPasskeyFailure(visiblePasskeyFailure); return; }
    if (pending && visiblePasskeyFailure && pending.requestId === visiblePasskeyFailure.requestId) { showPasskeyFailure(visiblePasskeyFailure); return; }
    visiblePasskeyFailure = null;
    const previouslyOwned = pendingPasskeyOwnsPopup;
    pendingPasskeyOwnsPopup = !!(pending && (pending.phase === 'checking' || pending.phase === 'completing' || choosePasskeyPanel(pending)));
    if (pendingPasskeyOwnsPopup) renderPasskeyCeremonyPanel(pending);
    else if (previouslyOwned) resumeOrdinaryPopup();
  });

  initPasskeyPopupUI = function () {
    rehydratePendingPasskeyCeremony((tookOver) => {
      refreshPasskeyProxy();
      if (!tookOver) resumeOrdinaryPopup();
    });
  };
  initPasskeyPopupUI();
  // Test harnesses run this block without a window.
  if (typeof window !== 'undefined' && window.addEventListener) window.addEventListener('pagehide', stopLockedPolling);
}

if (typeof module !== 'undefined') {
  module.exports = {
    classifyGetCredentialsResponse: bridgeStatus.classifyGetCredentialsResponse,
    isPairingChallengeFresh,
    normalizePairingCode,
    PAIRING_CHALLENGE_TTL_MS,
    APP_UNAVAILABLE_STATUS_TEXT,
    HOST_UNAVAILABLE_STATUS_TEXT,
    APP_UNAVAILABLE_OPEN_HELPER_TEXT,
    shouldShowAppUnavailableActions,
    LOCKED_TITLE_TEXT,
    LOCKED_HELPER_TEXT,
    EMPTY_LIST_TITLE_TEXT,
    LIST_HEADER_TEXT,
    FAVORITES_HEADER_TEXT,
    NO_FAVORITES_TEXT,
    isSiteUrl,
    PAIRING_PANEL_TITLE_TEXT,
    shouldShowUnlockButton,
    LOCKED_POLL_INTERVAL_MS,
    startLockedPolling,
    stopLockedPolling,
    classifyRequestUnlockResponse,
    UNLOCK_BUTTON_LABEL,
    POPUP_PAIRING_STORAGE_KEY,
    shouldContinueUnlockPolling,
    UNLOCK_POLL_INTERVAL_MS,
    UNLOCK_POLL_MAX_ATTEMPTS,
    WAITING_FOR_UNLOCK_TEXT,
    shouldShowTotpCircle,
    createTotpCircleController,
    buildTotpCircleElement,
    buildPasskeyBadgeElement,
    renderCredentials,
    totpRemainingFraction,
    totpRingDashOffset,
    tickTotpRings,
    registerTotpRing,
    TOTP_RING_PERIOD_SECONDS,
    TOTP_RING_CIRCUMFERENCE,
    validateTotpPasteInput,
    isBase32PasteSecret,
    describeSaveProposalOutcome,
    renderTotpPicker,
    routeTotpCandidate,
    requestPendingPrompt,
    handleAddTotpClick,
    requestPageTotpCandidate,
    requestPageNewPasswordField,
    handleGeneratePasswordClick,
    PENDING_PASSKEY_KEY,
    passkeyCeremonySummary,
    passkeyTargetSummary,
    choosePasskeyPanel,
    passkeyProxyStatusText,
    browserManagerDiagnosticsText,
    rehydratePendingPasskeyCeremony,
    renderPasskeyCeremonyPanel,
    renderPasskeyPicker,
    sendPasskeyCreateConfirm,
    sendPasskeyGetApprove,
    sendPasskeyCancel,
    initPasskeySettingsToggle,
    refreshPasskeyProxy,
    initPasskeyPopupUI
  };
}
