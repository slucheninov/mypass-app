(function () {
  // letterFromTitle/colorForTitle decide an entry's avatar letter and colour — single-sourced
  // in shared/entry-visuals.js (see its header comment) so the popup and this panel can never
  // disagree about the same entry's colour. manifest.json loads entry-visuals.js as a
  // preceding <script>, so in a page these are already globals by the time this IIFE runs;
  // node has no such script tag (tests `require('../content.js')` directly), so require them
  // there instead — the same three-world seam entry-visuals.js itself is built for.
  const entryVisuals = typeof require === 'function'
    ? require('./shared/entry-visuals.js')
    : { letterFromTitle: window.letterFromTitle, colorForTitle: window.colorForTitle };
  const letterFromTitle = entryVisuals.letterFromTitle;
  const colorForTitle = entryVisuals.colorForTitle;

  // Pure helpers shared with the popup and the service worker live in shared/.
  // manifest.json loads those files as classic scripts before this one, so the
  // names are already globals here. Node tests require this file directly and
  // have no script tags, so the require() branch loads the same modules.
  // Bindings stay inside this function. A top-level const in this file with
  // the same name as one of those scripts is a load-time SyntaxError in the
  // page (see tests/scriptScopes.test.js).
  //
  // Read the `mypass*` bags by bare name. They are `var`s in the shared
  // scripts above, so they live in this extension's content-script scope.
  // Do not switch this to the page global object: that object is the page,
  // it has no findLoginForm, and the call throws "is not a function"
  // (seen on linkedin.com/feed).
  const shared = typeof require === 'function'
    ? {
        urlMatch: require('./shared/url-match.js'),
        bridgeStatus: require('./shared/bridge-status.js'),
        base32: require('./shared/base32.js'),
        formDetection: require('./shared/form-detection.js'),
        addressForm: require('./shared/address-form.js'),
        totpDetection: require('./shared/totp-detection.js'),
        pageIdentifier: require('./shared/page-identifier.js'),
        securityWatch: require('./shared/security-watch.js'),
        autofillSettings: require('./shared/autofill-settings.js'),
        placement: require('./shared/placement.js'),
      }
    : {
        urlMatch: mypassUrlMatch,
        bridgeStatus: mypassBridgeStatus,
        base32: mypassBase32,
        formDetection: mypassFormDetection,
        addressForm: mypassAddressForm,
        totpDetection: mypassTotpDetection,
        pageIdentifier: mypassPageIdentifier,
        securityWatch: mypassSecurityWatch,
        autofillSettings: mypassAutofillSettings,
        placement: mypassPlacement,
      };
  const hostnameMatches = shared.urlMatch.hostnameMatches;
  const securityWatch = shared.securityWatch;
  const classifyGetCredentialsResponse = shared.bridgeStatus.classifyGetCredentialsResponse;
  const isBase32Secret = shared.base32.isBase32Secret;
  const {
    scoreUsernameLikeness,
    findBestUsernameInput,
    scoreOtpLikeness,
    findOtpField,
    isFieldVisible,
    isNewPasswordCandidate,
    findFillableFields,
    findLoginForm,
  } = shared.formDetection;
  const {
    detectOtpauthLink,
    detectOtpauthText,
    detectManualSetupKey,
    decodeQrFromImage,
    detectQrCandidate,
    detectTotpCandidateWithPriority,
    shouldShowTotpBanner,
    forgetDecodedQr,
  } = shared.totpDetection;

  // Which behaviours may run outside the top frame. Stated once, as data, rather than as
  // `window.top === window` scattered through the file: the policy is then testable without
  // a DOM, and adding a behaviour forces a decision about frames instead of inheriting one.
  // Unknown names are NOT enabled — a banner added later without a policy entry must not
  // quietly start rendering inside every ad frame on the web.
  const FRAME_FEATURE_POLICY = {
    inlineIcon: 'allFrames',
    saveBanner: 'topOnly',
    totpBanner: 'topOnly',
    totpScan: 'topOnly',
    totpCandidateQuery: 'topOnly',
    newPasswordFieldQuery: 'topOnly',
    pageIdentifierQuery: 'topOnly',
    pendingOpenFill: 'topOnly',
  };

  function isFeatureEnabledInFrame(feature, isTopFrame) {
    const policy = FRAME_FEATURE_POLICY[feature];
    if (!policy) return false;
    return policy === 'allFrames' || isTopFrame === true;
  }

  // Which fields an applyFill message writes into. Extracted from the message listener
  // below — which is unreachable from the node tests, living inside the browser-only
  // wiring guard — precisely because the destination of a fill is the thing that must be
  // testable. The existing test for the popup's Generate button asserted the relayed
  // message payload and never which input received it, which is how the bug this
  // function's `newPassword` branch fixes shipped.
  //
  // Three cases, in priority order:
  //
  //  1. A clicked field (pendingFillTarget) wins outright, so the fill lands in the group
  //     the user actually clicked rather than the first password field in document order.
  //  2. `fillTarget: 'newPassword'` — a popup-originated generated password. The popup
  //     has no clicked field, and falling back to findLoginForm() here wrote the new
  //     secret over the CURRENT-password box on a change-password form (current / new /
  //     confirm), since findLoginForm returns the first visible password input in
  //     document order. That is exactly the outcome the inline panel's own
  //     new-password-only gating exists to prevent, arriving through the other surface.
  //     Signup forms only appeared to work because there the first password field happens
  //     to be the new-password field. When no new-password field is found this fills
  //     NOTHING — never the first password field — because the safe failure for a
  //     generated password is to go nowhere.
  //  3. Everything else: the popup's ordinary credential fill. A page with a password
  //     still goes through findLoginForm. A phone- or email-only first step has no
  //     password, so the username field from findFillableFields is filled instead.
  function resolveApplyFillTargets(message, target, root) {
    if (target) return resolveFillTargets(target);

    const scope = root || document;
    if (message && message.fillTarget === 'newPassword') {
      const field = findFillableFields(scope).find(
        (candidate) => candidate.kind === 'password' && candidate.isNewPassword
      );
      return { usernameInput: null, passwordInput: field ? field.input : null, otpInput: null };
    }

    // findLoginForm requires a password field, so a phone-only or email-only first
    // step (no password yet) used to resolve to nothing and the popup wrote nowhere.
    // The username field findFillableFields already recognized is the fill target.
    const loginForm = findLoginForm(scope);
    const usernameField = findFillableFields(scope).find((field) => field.kind === 'username');
    return {
      usernameInput: (loginForm && loginForm.usernameInput) || (usernameField ? usernameField.input : null),
      passwordInput: loginForm ? loginForm.passwordInput : null,
      otpInput: findOtpField(scope),
    };
  }

  // Fills only the field whose icon was clicked — a deliberate product decision. Clicking the
  // password icon must not touch the username, and vice versa. The popup's fill path is
  // unaffected: it has no clicked field and still fills username and password together via
  // findLoginForm.
  function resolveFillTargets(target) {
    if (!target || !target.input) {
      return { usernameInput: null, passwordInput: null, otpInput: null };
    }
    return {
      usernameInput: target.kind === 'username' ? target.input : null,
      passwordInput: target.kind === 'password' ? target.input : null,
      otpInput: target.kind === 'otp' ? target.input : null,
    };
  }

  // Exported for tests: the wiring that consumes this lives in the browser-only block and is
  // unreachable from node, which is exactly where the stale-target bug hid.
  function isFillTargetUsable(target, now, ttlMs) {
    if (!target || !target.input) return false;
    if (target.input.isConnected !== true) return false;
    return (now - target.at) <= ttlMs;
  }

  // Panel state is a function of the connection state and the field kind, never of which
  // element was clicked. Extracted as a pure function for the same reason Stage 6 extracted
  // choosePasskeyPanel: routing is the part worth testing, and a DOM makes it untestable.
  function chooseInlinePanel(response, fieldKind, settings) {
    const kind = classifyGetCredentialsResponse(response);
    if (kind === 'locked') return { panel: 'locked', entries: [] };
    if (kind === 'pairingRequired') return { panel: 'pairingRequired', entries: [] };
    if (kind !== 'credentialList') return { panel: 'unavailable', entries: [] };

    const all = response.credentials || [];
    const matched = fieldKind === 'otp' ? all.filter((entry) => entry.hasTotp) : all;
    const entries = shared.autofillSettings.filterSuggestionEntries(matched, settings);
    return entries.length === 0 ? { panel: 'empty', entries: [] } : { panel: 'entries', entries };
  }

  // The inline icon is a control the PAGE can see and dispatch events at, unlike the popup.
  // Every activation path therefore requires a user-generated event. This is not defence in
  // depth against a spoofed tab: relayFillToActiveTab in background.js targets
  // sender.tab.id/sender.frameId/sender.documentId, i.e. exactly the document that sent the
  // request, so it needs no help from this gate to find the right tab. The gate exists
  // because the inline panel is injected into a page the attacker may control: without it,
  // page script could synthesize a click and trigger a fill/reveal the user never asked for.
  function isTrustedActivation(event) {
    return !!(event && event.isTrusted === true);
  }

  // Same trust gate as handleIconActivate/handleEntrySelect, generalized for the Save/Dismiss
  // buttons on the page-injected security warning (and any other control this script puts in a document the page can see). Those
  // buttons live in a document the page script can see and dispatch synthetic click events
  // at, unlike the popup, so every handler that sends a message or mutates state must first
  // confirm the event was user-generated. Returns a listener suitable for addEventListener,
  // rather than checking inline in each handler, so every banner button site gets the same
  // gate the same way.
  // `event.isTrusted` alone does NOT prove a person acted: form.requestSubmit() and
  // form.submit-by-script run from page code still dispatch a submit event whose isTrusted is
  // true. Opening browser UI (the popup or a window) needs a real gesture, so a login submit
  // must also happen inside a live user activation (navigator.userActivation.isActive: a click,
  // a tap or a non-modifier keydown within the last few seconds, set by the browser and not
  // forgeable from page script). That covers both a mouse click on a submit button and Enter in
  // a field, trusted or JS-handled, and fails closed where the API is missing. Limits, accepted:
  // a page that waits longer than the activation window before submitting gets no prompt, and a
  // page can still call requestSubmit() from inside a click the user really made.
  function hasLiveUserActivation(nav) {
    const target = nav === undefined ? (typeof navigator !== 'undefined' ? navigator : null) : nav;
    return !!(target && target.userActivation && target.userActivation.isActive === true);
  }

  function isUserSubmit(event, nav) {
    return isTrustedActivation(event) && hasLiveUserActivation(nav);
  }

  function trustedClickHandler(handler) {
    return function (event) {
      if (!isTrustedActivation(event)) return;
      handler(event);
    };
  }

  function handleIconActivate(event, requestPanel) {
    if (!isTrustedActivation(event)) return false;
    requestPanel();
    return true;
  }

  // Sends the identical message popup.js:617 sends. The content script never learns the
  // secret: background.js resolves it natively and delivers it straight back as applyFill.
  function handleEntrySelect(event, entryId, url, sendMessage) {
    if (!isTrustedActivation(event)) return false;
    sendMessage({ type: 'fillCredential', id: entryId, url });
    return true;
  }

  // The save-login and save-2FA offers used to be banners injected at the bottom of the page.
  // They are now prompts shown in the MyPass popup (or its window fallback): this script only
  // ASKS, and the background, which owns the pending record, validates the sender (tab, frame,
  // document, url), re-reads the offerSave* switches and decides how to surface it. The secret
  // travels once, to the background; the popup only ever receives a redacted view, and no
  // banner here means nothing in the page's DOM for page script to click or read.
  const PROMPT_DEDUPE_MS = 120000;

  // A login submit raises at most one prompt per distinct credential pair per 120 s (the
  // prompt's own lifetime): a re-submitted identical form must not keep re-opening the popup
  // after the user dismissed it. Returns whether a request was sent.
  //
  // The dedupe state must not keep the password itself (it would sit in this page's memory long
  // after the prompt was saved, dismissed or expired). It keeps a fingerprint of the password
  // under a random per-document salt, so a CHANGED password still counts as a new credential, and
  // the entry is dropped when its 120 s are up. The salt lives only in memory, is never sent
  // anywhere and is not shared between documents, so the fingerprint is not a reusable password
  // hash; it is also not a security primitive, only a way not to hold the plaintext.
  function passwordFingerprint(state, password) {
    if (!state.dedupeSalt) {
      const seed = new Uint32Array(2);
      if (typeof crypto !== 'undefined' && crypto && typeof crypto.getRandomValues === 'function') {
        crypto.getRandomValues(seed);
      } else {
        seed[0] = Math.random() * 0x100000000;
        seed[1] = Math.random() * 0x100000000;
      }
      state.dedupeSalt = [seed[0], seed[1]];
    }
    // Two salted 32-bit multiply-xor passes (FNV-1a / murmur style): 64 bits of state.
    let h1 = state.dedupeSalt[0] ^ 0x811c9dc5;
    let h2 = state.dedupeSalt[1] ^ 0x1b873593;
    for (let i = 0; i < password.length; i += 1) {
      const c = password.charCodeAt(i);
      h1 = Math.imul(h1 ^ c, 0x01000193);
      h2 = Math.imul(h2 ^ c, 0x85ebca6b);
      h2 ^= h2 >>> 13;
    }
    return (h1 >>> 0).toString(16) + '.' + (h2 >>> 0).toString(16) + '.' + password.length;
  }

  function requestSavePrompt(url, username, password, state, sendMessage, now) {
    const at = typeof now === 'number' ? now : Date.now();
    const key = url + '\u0000' + username + '\u0000' + passwordFingerprint(state, password);
    const last = state.lastLoginPrompt;
    if (last && last.key === key && at - last.at < PROMPT_DEDUPE_MS) return false;
    const entry = { key, at };
    state.lastLoginPrompt = entry;
    // Drop the entry once the prompt's lifetime is over. The identity check keeps a newer
    // submit's entry; unref lets a test runner exit without waiting for the timer.
    const timer = setTimeout(() => {
      if (state.lastLoginPrompt === entry) state.lastLoginPrompt = null;
    }, PROMPT_DEDUPE_MS);
    if (timer && typeof timer.unref === 'function') timer.unref();
    sendMessage({ type: 'requestPrompt', kind: 'saveLogin', url, username, password }, (response) => {
      // A reply that never came (the service worker restarted, the extension reloaded) sets
      // runtime.lastError; reading it is what stops Chrome logging "Unchecked runtime.lastError".
      // The missing response itself is handled below exactly as before: the entry is dropped so
      // the next submit retries.
      if (typeof chrome !== 'undefined' && chrome && chrome.runtime) void chrome.runtime.lastError;
      if (!response || response.type === 'error' || response.created === false) {
        if (state.lastLoginPrompt === entry) state.lastLoginPrompt = null;
      }
    });
    return true;
  }

  // `priority` is the detected candidate's own priority (1=link, 2=text, 3=QR, 4=manual key).
  // Offering counts as the dismissal: with no in-page banner there is nothing to re-show, so
  // `shouldShowTotpBanner`'s "an offered candidate suppresses only equal-or-weaker-priority
  // ones" rule stores the ACTUAL priority, not a hardcoded weakest value.
  function requestTotpPrompt(url, otpauthOrSecret, state, priority, sendMessage) {
    state.totpBannerDismissed = { priority };
    sendMessage({ type: 'requestPrompt', kind: 'saveTotp', url, otpauthOrSecret });
    return true;
  }

  // What the content script answers a popup's `{ type: 'queryTotpCandidate' }` request
  // with — the current page's strongest TOTP candidate (or null), so the popup's Add-2FA
  // section can prefill itself and reveal only when there is something to act on. Extracted
  // as its own (unguarded, exported) function — like requestTotpPrompt above —
  // since the chrome.runtime.onMessage listener that calls it lives inside the browser-only
  // wiring guard below and isn't reachable from tests without a full DOM/chrome stub.
  function buildTotpCandidateQueryResponse(root) {
    return detectTotpCandidateWithPriority(root);
  }

  // What the content script answers a popup's `{ type: 'queryNewPasswordField' }`
  // request with — whether this document currently has a field the popup could offer
  // to fill with a generated password. Exported (like buildTotpCandidateQueryResponse
  // above) so the classification is testable without a DOM.
  function buildNewPasswordFieldQueryResponse(root) {
    const hasNewPasswordField = findFillableFields(root).some((field) => field.kind === 'password' && field.isNewPassword);
    return { hasNewPasswordField };
  }

  // The mutation sweep serves two consumers with OPPOSITE frame rules: attaching icons must
  // happen in every frame — that is the whole feature — while the TOTP banner is top-frame
  // work. Built as an injectable factory so a test can prove which half is gated; wiring
  // these backwards is silent and a DOM-level test cannot see it.
  function buildMutationSweep(options) {
    const onAttachIcons = typeof options.onAttachIcons === 'function' ? options.onAttachIcons : () => {};
    const onScanForBanner = typeof options.onScanForBanner === 'function' ? options.onScanForBanner : () => {};
    const isTopFrame = options.isTopFrame;
    return function sweep() {
      try {
        onAttachIcons();
      } catch (error) {
        console.warn('MyPass: inline icon attach failed — ' + error.message);
      }
      if (isFeatureEnabledInFrame('totpBanner', isTopFrame)) {
        onScanForBanner();
      }
    };
  }

  // Guards the document-level 'load' listener added below against being attached more than
  // once. Real wiring calls observeTotpCandidates exactly once per frame, but nothing stops a
  // test — or a future caller — from calling it again, and a duplicate capture-phase listener
  // on `document` would silently double-schedule every image load forever. The listener bound
  // here closes over the FIRST observeTotpCandidates call's own `scheduleSweep` — a second
  // call's sweep/debounceTimer would simply never be reachable, hence the guard rather than
  // "just let it re-attach".
  let totpImageLoadListenerAttached = false;
  // The listener function itself, so teardownExtensionContext (below, in the browser-only
  // wiring) can remove exactly this listener and reset the flag when the extension context
  // dies — otherwise it would keep firing after an unpacked-extension reload orphans this
  // content script, re-running the sweep and re-injecting icons/banners into a dead page.
  let totpImageLoadListener = null;

  // onMutation defaults to a no-op so this stays callable from tests without a DOM;
  // the browser-only wiring below passes attachIcons so newly-inserted fields get an
  // icon on the same debounce that re-checks for TOTP candidates. The frame gating itself
  // lives in buildMutationSweep above, where it can be exercised with spies instead of a DOM.
  function observeTotpCandidates(state, onMutation, isTopFrame, offerSave) {
    const notifyMutation = typeof onMutation === 'function' ? onMutation : () => {};
    const mayOfferSave = typeof offerSave === 'function' ? offerSave : () => true;
    const sweep = buildMutationSweep({
      onAttachIcons: notifyMutation,
      onScanForBanner: () => {
        if (!mayOfferSave()) return;
        const detected = detectTotpCandidateWithPriority(document);
        if (shouldShowTotpBanner(detected, state)) {
          requestTotpPrompt(location.href, detected.candidate, state, detected.priority, sendRuntimeMessage);
        }
      },
      isTopFrame,
    });
    // With all_frames: true this now runs in EVERY frame, and not every frame holds an
    // HTML document. An SVG or XML document is an ordinary iframe target and has no
    // document.body at all, so MutationObserver.observe(undefined) would throw an
    // uncaught TypeError there — precisely the "errors on a page full of ad iframes"
    // failure this feature must not produce. Such a document has no fillable fields and
    // no banner to show, so there is nothing to watch for: return null rather than throw.
    // Callers treat the observer as optional.
    const body = (typeof document !== 'undefined' && document) ? document.body : null;
    if (!body) return null;
    let debounceTimer = null;
    const scheduleSweep = () => {
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(sweep, 400);
    };
    const observer = new MutationObserver(scheduleSweep);
    observer.observe(body, { childList: true, subtree: true });

    // A QR image that finishes loading after this sweep already ran — e.g. a slow network
    // image whose <img> node was already in the DOM at mutation time — is otherwise never
    // re-examined: nothing mutates again, so the MutationObserver above never fires for it.
    // 'load' does not bubble, so it must be caught in the capture phase on `document` rather
    // than on the image itself (which this code never has a direct reference to). Gated on
    // the same 'totpBanner' frame policy buildMutationSweep applies to onScanForBanner above:
    // QR rescans are top-frame-only work, same as the rest of the banner feature. Attached at
    // most once via the module-level flag above.
    if (!totpImageLoadListenerAttached && isFeatureEnabledInFrame('totpBanner', isTopFrame)) {
      totpImageLoadListenerAttached = true;
      totpImageLoadListener = (event) => {
        // A page-synthesized 'load' event must not evict the QR cache or force a sweep.
        if (!isTrustedActivation(event)) return;
        // A dead extension context (unpacked reload) must not re-run the sweep: with no
        // background service worker to answer it, attachIcons()/requestTotpPrompt() would
        // act on into a page nothing can ever service again. teardownExtensionContext
        // removes this listener once sendRuntimeMessage first observes the context is gone,
        // but this early-return also protects the gap before that first detection.
        if (!isExtensionContextAlive()) return;
        if (!event || !event.target || event.target.tagName !== 'IMG') return;
        // The just-finished load may be a same-URL, same-size refresh (a re-fetched
        // endpoint, an animated/refreshing QR) — decodeQrFromImage's cache would otherwise
        // hand back the stale verdict from before this load, defeating the whole point of
        // rescanning. Drop this image's cache entry so the upcoming sweep decodes fresh.
        forgetDecodedQr(event.target);
        scheduleSweep();
      };
      document.addEventListener('load', totpImageLoadListener, true);
    }
    return observer;
  }

  function setNativeValue(input, value) {
    // Write through the platform setter. Assigning `input.value` updates the DOM node
    // but a controlled field (React and similar) keeps its own tracker and paints the
    // empty string back, so the phone the user just saved never appears.
    const view = input.ownerDocument && input.ownerDocument.defaultView;
    const prototypes = view && { INPUT: view.HTMLInputElement, TEXTAREA: view.HTMLTextAreaElement };
    const constructor = prototypes && prototypes[input.tagName];
    const setter = constructor && Object.getOwnPropertyDescriptor(constructor.prototype, 'value');
    if (setter && setter.set) setter.set.call(input, value);
    else input.value = value;
    const EventCtor = (view && view.Event) || Event;
    input.dispatchEvent(new EventCtor('input', { bubbles: true }));
    input.dispatchEvent(new EventCtor('change', { bubbles: true }));
  }

  // Built from DOM nodes rather than an innerHTML string: sites that enforce the
  // require-trusted-types-for 'script' CSP directive throw on any innerHTML assignment,
  // and those are exactly the sites this feature most needs to work on.
  const SVG_NS = 'http://www.w3.org/2000/svg';
  // Upright green key silhouette from icons/icon128.png, retaining the round head,
  // high-set hole, and right-facing teeth at small sizes. A DOM-built path works with
  // Trusted Types and does not expose a chrome-extension asset URL to the page.
  const KEY_ICON_PATH_D =
    'M20 60.5a30 30 0 1 1 24 0V66l6 7-7 7 7 8-7 7 7 9-15 17q-3 3-6 0l-7-7q-2-2-2-6Z' +
    'M32 13a8 8 0 1 0 0 16a8 8 0 1 0 0-16Z';

  function buildKeyGlyph(doc) {
    const svg = doc.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('viewBox', '0 0 64 128');
    svg.setAttribute('width', '12');
    svg.setAttribute('height', '24');
    svg.setAttribute('aria-hidden', 'true');
    const path = doc.createElementNS(SVG_NS, 'path');
    path.setAttribute('fill', 'currentColor');
    path.setAttribute('fill-rule', 'evenodd');
    path.setAttribute('d', KEY_ICON_PATH_D);
    svg.appendChild(path);
    return svg;
  }

  const ICON_HOST_STYLE =
    ':host{all:initial}' +
    'button{all:unset;box-sizing:border-box;cursor:pointer;display:flex;align-items:center;' +
    'justify-content:center;width:100%;height:100%;border-radius:6px;color:#73b359;}' +
    'svg{display:block;flex:none;max-height:calc(100% - 4px);pointer-events:none;}' +
    'button:hover{background:rgba(115,179,89,0.14);}' +
    'button:focus-visible{outline:2px solid #007aff;outline-offset:1px;background:rgba(115,179,89,0.14);}' +
    '@media(prefers-color-scheme:dark){button:hover{background:rgba(115,179,89,0.24);}}' +
    '@media(forced-colors:active){button{color:ButtonText;}' +
    'button:hover{color:HighlightText;background:Highlight;}button:focus-visible{outline-color:Highlight;}}';

  // The panel is a list surface, not a 20x20 glyph button, so it needs its own stylesheet:
  // handing it ICON_HOST_STYLE rendered every row as a 20x20 box with overflowing text on a
  // fully transparent host. Deliberately the same restrained language as popup.html — system
  // font, #fff on a #d2d2d7 hairline, 8px radius, #f5f5f7 hover, #007aff for the one action.
  const PANEL_HOST_STYLE =
    ':host{all:initial}' +
    '.panel{box-sizing:border-box;min-width:220px;max-width:320px;padding:4px;' +
    'background:#ffffff;color:#1d1d1f;border:1px solid #d2d2d7;border-radius:8px;' +
    'box-shadow:0 4px 16px rgba(0,0,0,0.18);' +
    "font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-size:13px;" +
    'line-height:1.35;max-height:var(--mypass-panel-max-height,70vh);overflow-y:auto;}' +
    '.row{all:unset;box-sizing:border-box;display:flex;align-items:center;gap:8px;' +
    'width:100%;padding:7px 10px;border-radius:6px;cursor:pointer;text-align:left;' +
    'color:#1d1d1f;overflow:hidden;}' +
    '.row:hover,.row:focus-visible{background:#f5f5f7;}' +
    '.avatar{flex-shrink:0;width:28px;height:28px;border-radius:7px;' +
    'display:flex;align-items:center;justify-content:center;}' +
    '.avatar span{color:#ffffff;font-size:13px;font-weight:600;line-height:1;}' +
    '.entry-text{min-width:0;flex:1 1 auto;overflow:hidden;}' +
    '.entry-title{font-size:13px;font-weight:600;line-height:1.3;' +
    'white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}' +
    '.entry-username{font-size:11px;color:#6e6e73;line-height:1.25;margin-top:1px;' +
    'white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}' +
    '.ring{flex-shrink:0;width:18px;height:18px;}' +
    '.ring svg{display:block;width:100%;height:100%;}' +
    '.message{padding:7px 10px;color:#6e6e73;}' +
    '.settings-row{margin-top:2px;border-top:1px solid #e5e5ea;border-radius:0 0 6px 6px;' +
    'color:#6e6e73;font-size:12px;}' +
    '.unlock{all:unset;box-sizing:border-box;display:block;width:100%;margin-top:2px;' +
    'padding:7px 10px;border-radius:6px;cursor:pointer;text-align:center;' +
    'background:#007aff;color:#ffffff;}' +
    '.unlock:hover,.unlock:focus-visible{background:#0069db;}' +
    // Dark surface for dark-themed sites. Light values above stay the default — this block
    // only redefines colours, never layout, so the avatar, ring, row layout, and ellipsis
    // behaviour are all untouched. `.avatar span` and `.unlock` keep their fixed white text
    // since both sit on their own coloured backgrounds (the avatar's per-entry colour, the
    // blue unlock button) that do not change between themes.
    '@media (prefers-color-scheme: dark){' +
    '.panel{background:#1e1e20;color:#f5f5f7;border-color:#3a3a3c;' +
    'box-shadow:0 4px 16px rgba(0,0,0,0.5);}' +
    '.row{color:#f5f5f7;}' +
    '.row:hover,.row:focus-visible{background:#2c2c2e;}' +
    '.entry-username{color:#98989d;}' +
    '.message{color:#98989d;}' +
    '.settings-row{border-top-color:#3a3a3c;color:#98989d;}' +
    // Slightly brighter green so the 2FA ring stays legible against the dark panel surface.
    '.ring circle{stroke:#30d158;}' +
    '}';

  // The shared scaffolding behind both surfaces: a light-DOM host carrying our marker,
  // an isolating closed shadow root, and the layout styles marked important so a page
  // rule cannot hide or move the control. The icon adds a button on top of this; the
  // panel adds its rows. Neither should carry the other's contents — hence styleText is a
  // parameter, not a shared constant: each surface installs its own stylesheet.
  function buildShadowHost(documentRef, markerAttribute, styleText) {
    const doc = documentRef || document;
    const host = doc.createElement('div');
    host.setAttribute(markerAttribute, '');
    // setProperty(..., 'important') because a page's own `!important` CSS rule (e.g.
    // [data-mypass-inline-icon]{display:none!important}) would otherwise beat a plain
    // inline style and could hide or mislocate the control.
    // `fixed`, not `absolute`: coordinates are the viewport numbers getBoundingClientRect
    // returns, so a <body> margin, `position:relative`, or `transform` cannot shift the control
    // (it used to land 8-40 px away on such pages). Hosts are mounted by mountParentFor.
    host.style.setProperty('position', 'fixed', 'important');
    // z-index one below the in-page security warning's 2147483647 so that warning still wins.
    host.style.setProperty('z-index', '2147483646', 'important');
    host.style.setProperty('display', 'block', 'important');
    host.style.setProperty('pointer-events', 'auto', 'important');

    const shadow = host.attachShadow({ mode: 'closed' });
    const style = doc.createElement('style');
    style.textContent = typeof styleText === 'string' ? styleText : '';
    shadow.appendChild(style);

    return { host, shadow };
  }

  function buildIconHost(documentRef) {
    const doc = documentRef || document;
    const { host, shadow } = buildShadowHost(doc, 'data-mypass-inline-icon', ICON_HOST_STYLE);
    const button = doc.createElement('button');
    button.setAttribute('type', 'button');
    button.setAttribute('aria-label', 'Fill from MyPass');
    button.setAttribute('title', 'Fill from MyPass');
    // The accessible name stays "Fill from MyPass"; these only say a list opens and whether it
    // currently is (togglePanel/closePanel keep aria-expanded in step).
    button.setAttribute('aria-haspopup', 'true');
    button.setAttribute('aria-expanded', 'false');
    button.appendChild(buildKeyGlyph(doc));
    shadow.appendChild(button);
    return { host, shadow, button };
  }

  // The panel's counterpart to buildIconHost. It exists so the marker/stylesheet pairing is
  // decided in one exported place rather than at the call site inside the browser-only
  // block, which no test can reach — that is exactly how the panel ended up wearing the
  // icon's 20x20 button stylesheet.
  function buildPanelHost(documentRef) {
    const doc = documentRef || document;
    return buildShadowHost(doc, 'data-mypass-inline-panel', PANEL_HOST_STYLE);
  }

  // Writes to the overlay host only. The page's own input is never given padding, a
  // wrapper, or any attribute — that is the whole reason for the overlay approach.
  //
  // Viewport coordinates on a `position:fixed` host: `viewport` is kept in the signature for
  // callers but scroll offsets are deliberately ignored. `offset` is what settlePlacementOffset
  // measured for this host: {dx, dy} in the host's local pixels, plus {sx, sy}, the scale its
  // containing block applies on screen (an `html` transform; both default to 1). `left`, `top`,
  // `width` and `height` are LOCAL pixels, but placeIcon works in viewport pixels, so each is
  // divided by the scale: a 28 px icon under `html{transform:scale(.5)}` has to be written as
  // 56 local px to render as 28 on screen, and its position moves with the page by 1/scale.
  function positionHostToField(host, rect, viewport, obstacles = [], clipRect = null, offset = null, box = null) {
    const placed = shared.placement.placeIcon(rect, obstacles, clipRect, box);
    const dx = offset && typeof offset.dx === 'number' ? offset.dx : 0;
    const dy = offset && typeof offset.dy === 'number' ? offset.dy : 0;
    const sx = shared.placement.usableScale(offset && offset.sx);
    const sy = shared.placement.usableScale(offset && offset.sy);
    const left = placed.left / sx + dx;
    const top = placed.top / sy + dy;
    // Tiny fields have no room for both typing and a useful button. The toolbar popup
    // remains available; never grow a button beyond the input or cover most of its text.
    host.style.setProperty('display', placed.fits ? 'block' : 'none', 'important');
    host.style.setProperty('position', 'fixed', 'important');
    host.style.setProperty('width', String(placed.size / sx) + 'px', 'important');
    host.style.setProperty('height', String(placed.size / sy) + 'px', 'important');
    host.style.setProperty('top', String(top) + 'px', 'important');
    host.style.setProperty('left', String(left) + 'px', 'important');
    return {
      fits: placed.fits,
      intended: { left: placed.left, top: placed.top, width: placed.size, height: placed.size }
    };
  }

  // Where a host for `input` is mounted. A field inside an open modal <dialog> lives in the
  // browser top layer, which paints above every z-index; a host under <html> would be covered
  // and not clickable, so it goes inside that dialog instead. Everything else goes under
  // <html>, not <body>: a `transform`/`filter` on <body> makes it the containing block for
  // fixed descendants and shifts them. `:modal` is Chrome 105+ (the manifest floor is 127),
  // but an unsupported selector throws, so guard it rather than lose the icon.
  function mountParentFor(input) {
    const doc = (input && input.ownerDocument) || document;
    try {
      const dialog = input && typeof input.closest === 'function' ? input.closest('dialog:modal') : null;
      if (dialog) return dialog;
    } catch (error) {
      // Unsupported pseudo-class: fall through to the document mount.
    }
    return doc.documentElement;
  }

  // The host's own layout size in local pixels: what positionHostToField wrote, which unlike
  // offsetWidth is not rounded to an integer.
  function localExtent(host, name, fallback) {
    const written = host.style && parseFloat(host.style[name]);
    return written > 0 ? written : fallback;
  }

  // Compares where a host actually landed (and, when `intended` carries a width and height, how
  // big it rendered) with where it was told to go. Returns the fix, or null when it is right:
  // {dx, dy}, the shift in the host's own (local) CSS pixels that closes the position gap, or
  // {dx: 0, dy: 0, sx, sy} when the rendered SIZE is off, because a scaled containing block
  // (`html{transform:scale(.5)}`) renders the local 28 px that was written as 14. The scale is
  // read off the host as rendered size over written size. A size fix never carries a position
  // fix: the position was computed for the old scale and is measured again once the new size
  // is applied. Applies nothing itself.
  function measurePlacementOffset(host, intended) {
    if (!host || typeof host.getBoundingClientRect !== 'function') return null;
    const measured = host.getBoundingClientRect();
    if (!measured || !(measured.width > 0)) return null;
    const scale = shared.placement.scaleFrom(
      measured, localExtent(host, 'width', host.offsetWidth), localExtent(host, 'height', host.offsetHeight)
    );
    if (typeof intended.width === 'number' && typeof intended.height === 'number'
      && (Math.abs(measured.width - intended.width) > 1 || Math.abs(measured.height - intended.height) > 1)) {
      return { dx: 0, dy: 0, sx: scale.x, sy: scale.y };
    }
    return shared.placement.correction(intended, measured, 1, scale);
  }

  // A scale is only an estimate, and a size change moves what the position gap means, so one
  // fix can still leave a gap: apply it, measure again, and repeat a few times. `target` is where
  // the host should be in viewport coordinates (plus its width and height when the caller sizes
  // it); `apply(offset)` writes the host's styles for a given {dx, dy, sx, sy}. Returns the
  // offset and whether the host ended within tolerance, so callers cache the offset as verified
  // only when it really is.
  const PLACEMENT_SETTLE_PASSES = 3;
  function settlePlacementOffset(host, target, offset, apply) {
    let current = offset || { dx: 0, dy: 0 };
    for (let pass = 0; pass < PLACEMENT_SETTLE_PASSES; pass += 1) {
      const fix = measurePlacementOffset(host, target);
      if (!fix) return { offset: current, settled: true };
      current = {
        dx: current.dx + fix.dx,
        dy: current.dy + fix.dy,
        sx: typeof fix.sx === 'number' ? fix.sx : current.sx,
        sy: typeof fix.sy === 'number' ? fix.sy : current.sy
      };
      apply(current);
    }
    return { offset: current, settled: !measurePlacementOffset(host, target) };
  }

  // Syncs that may re-run the settle loop for one host before it is accepted as it is; keeps a
  // page whose host never converges from costing a layout read per scroll frame forever.
  const PLACEMENT_VERIFY_TRIES = 3;

  // A host that needed no correction at all (the usual page) is verified once and never measured
  // again. One that did need a shift or a scale sits in a containing block that is not the
  // viewport (a transformed `html`), and that block scrolls with the page, so the shift that was
  // right a moment ago is wrong after the next scroll: such a host is measured again on every
  // sync, which costs one layout read per frame on those pages only.
  function isIdentityOffset(offset) {
    return !offset || (Math.abs(offset.dx || 0) < 0.01 && Math.abs(offset.dy || 0) < 0.01 && !offset.sx && !offset.sy);
  }

  // An ancestor that is the containing block of `position:fixed` descendants (a transform,
  // perspective, filter or backdrop-filter, `contain: layout|paint|strict|content`, or a
  // will-change naming one of those). Any other ancestor is bypassed by a fixed child, so its
  // `overflow` does not clip it.
  function isFixedContainingBlock(style) {
    if (!style) return false;
    const set = (value) => typeof value === 'string' && value !== '' && value !== 'none' && value !== 'auto';
    return set(style.transform) || set(style.perspective) || set(style.filter) || set(style.backdropFilter)
      || set(style.translate) || set(style.rotate) || set(style.scale)
      || /layout|paint|strict|content/.test(style.contain || '')
      || /transform|perspective|filter|backdrop-filter|translate|rotate|scale|contain/.test(style.willChange || '');
  }

  function fieldClipRect(input, viewport) {
    const clip = { left: 0, top: 0, right: viewport.innerWidth, bottom: viewport.innerHeight };
    const doc = input.ownerDocument;
    // A `position:fixed` field is laid out against the viewport: an `overflow:hidden` wrapper
    // that is not its containing block does not clip it (confirmed in Chrome: the field stays
    // fully visible and clickable while the wrapper is 100 x 40 px). Ignoring that hid the icon
    // on such pages. Once the walk meets a real containing block the field is clipped by it and
    // by every ancestor above it, so the normal rule resumes from there.
    const inputStyle = viewport.getComputedStyle(input) || {};
    let escapesClipping = inputStyle.position === 'fixed';
    // Hosts live under <html> (or a modal dialog), so they do not inherit a field's overflow clipping. Intersect
    // every ancestor scrollport per axis and require the whole button to remain inside it:
    // a partially clipped field must not leave a clickable logo over neighbouring content.
    // Root/body overflow is propagated to the viewport, whose bounds are already included.
    for (let ancestor = input.parentElement;
      ancestor && ancestor !== doc.body && ancestor !== doc.documentElement;
      ancestor = ancestor.parentElement) {
      const style = viewport.getComputedStyle(ancestor);
      if (escapesClipping && isFixedContainingBlock(style)) escapesClipping = false;
      if (escapesClipping) continue;
      const clipsX = /^(auto|scroll|hidden|clip)$/.test(style.overflowX);
      const clipsY = /^(auto|scroll|hidden|clip)$/.test(style.overflowY);
      if (!clipsX && !clipsY) continue;
      const rect = ancestor.getBoundingClientRect();
      // Client bounds exclude borders and scrollbars. Scale those offsets with the box
      // when a site transforms it, keeping all comparisons in viewport coordinates.
      const scaleX = ancestor.offsetWidth ? rect.width / ancestor.offsetWidth : 1;
      const scaleY = ancestor.offsetHeight ? rect.height / ancestor.offsetHeight : 1;
      if (clipsX) {
        clip.left = Math.max(clip.left, rect.left + ancestor.clientLeft * scaleX);
        clip.right = Math.min(clip.right, rect.left + (ancestor.clientLeft + ancestor.clientWidth) * scaleX);
      }
      if (clipsY) {
        clip.top = Math.max(clip.top, rect.top + ancestor.clientTop * scaleY);
        clip.bottom = Math.min(clip.bottom, rect.top + (ancestor.clientTop + ancestor.clientHeight) * scaleY);
      }
    }
    return clip;
  }

  // The visible field box around a slim input: a Material-style field is a 48 px box whose
  // <input> is only 22 px tall, and an icon sized from the input alone is too small to show.
  // Walks a few ancestors and keeps the outermost one that hugs the input (within BOX_SLACK px
  // before, above and below it, no taller than BOX_MAX_HEIGHT), so a row or card around the field
  // is never mistaken for it. Zero-size ancestors (display:contents) are stepped over. Reads
  // geometry only; nothing on the page is touched. Returns {element, rect: {left, top, width,
  // height}} or null.
  //
  // The end side tolerates BOX_END_SLACK instead: sites make room for their own controls (a
  // reveal and a clear button, a suffix) by narrowing the input or by padding the field. On the
  // pz.gov.pl password field the site's CSS shrinks the input by 3.375rem (54 px) inside its
  // container, 70 px short of the visible box with the field's own padding; a 24 px limit saw no
  // box there and the icon stayed hidden. 72 px is room for three 24 px control slots, still far
  // below a row or card, and only the end side gets it. An input tall enough for the icon never
  // reaches the walk (placement ignores the box for it), which keeps the per-frame sync cheap.
  const BOX_SLACK = 24;
  const BOX_END_SLACK = 72;
  const BOX_MAX_HEIGHT = 80;
  const BOX_MAX_LEVELS = 4;
  function findFieldBox(input) {
    const inner = input.getBoundingClientRect();
    if (!shared.placement.needsBox(inner)) return null;
    let best = null;
    let ancestor = input.parentElement;
    for (let level = 0; ancestor && level < BOX_MAX_LEVELS; level += 1, ancestor = ancestor.parentElement) {
      const rect = ancestor.getBoundingClientRect();
      if (!(rect.width > 0) || !(rect.height > 0)) continue;
      const hugs = rect.left <= inner.left + 1 && rect.top <= inner.top + 1
        && rect.left + rect.width >= inner.left + inner.width - 1
        && rect.top + rect.height >= inner.top + inner.height - 1
        && inner.left - rect.left <= BOX_SLACK && inner.top - rect.top <= BOX_SLACK
        && rect.left + rect.width - (inner.left + inner.width) <= BOX_END_SLACK
        && rect.top + rect.height - (inner.top + inner.height) <= BOX_SLACK
        && rect.height <= BOX_MAX_HEIGHT;
      if (!hugs) break;
      if (rect.height > inner.height + 1) {
        best = { element: ancestor, rect: { left: rect.left, top: rect.top, width: rect.width, height: rect.height } };
      }
    }
    return best;
  }

  function fieldVisualBox(input) {
    const found = findFieldBox(input);
    return found ? found.rect : null;
  }

  // Visible site controls (reveal, clear, suffix buttons) the icon must stay clear of. `scope`
  // is the field box when there is one: its controls are not all inside the input's own
  // container (a suffix button is a sibling of it). Without a box the scan is limited to the
  // input's container, so controls elsewhere in a form never change an icon's alignment. SVGs
  // inside a button inherit the button's measured bounds.
  function fieldControlRects(input, scope) {
    const wrapper = scope || input.parentElement;
    if (!wrapper) return [];
    return Array.from(wrapper.querySelectorAll('button, [role="button"], a[href]'))
      .filter((control) => {
        const style = input.ownerDocument.defaultView.getComputedStyle(control);
        return style.visibility !== 'hidden' && style.display !== 'none';
      })
      .map((control) => control.getBoundingClientRect());
  }

  const PANEL_MESSAGES = {
    empty: 'No entries for this site',
    locked: 'MyPass is locked',
    pairingRequired: 'Pair MyPass to use it here',
    unavailable: 'MyPass is not running',
  };

  // Countdown ring geometry for the row's TOTP indicator. Deliberately smaller than the
  // popup's own 32px ring (this is a compact list row), but the same visual language: a green
  // stroke that empties as the current 30s TOTP period runs out.
  const PANEL_RING_PERIOD_SECONDS = 30;
  const PANEL_RING_RADIUS = 8;
  const PANEL_RING_CIRCUMFERENCE = 2 * Math.PI * PANEL_RING_RADIUS;

  // Wall-clock only — this NEVER reads the entry's secret. It is derived purely from
  // Date.now(), the same clock every TOTP authenticator already relies on, so it can convey
  // "the code is about to rotate" without ever knowing what the code is.
  function panelRingFraction(nowMs) {
    return 1 - ((nowMs / 1000) % PANEL_RING_PERIOD_SECONDS) / PANEL_RING_PERIOD_SECONDS;
  }

  // Built entirely from createElementNS + setAttribute, never innerHTML — the same
  // Trusted-Types reason as buildKeyGlyph above; this feature was already bitten by exactly
  // that on a require-trusted-types-for 'script' site. The ring is an INDICATOR ONLY: it
  // carries no text, no title/code, and — critically — no click listener of its own. It must
  // never call getTotp or otherwise reveal/copy the code; the panel's whole trust model is
  // that no secret is ever held here. Clicks still reach the row because the ring is nested
  // inside the row <button> and clicks bubble — the ring itself just never listens.
  function buildPanelRing(doc) {
    const wrapper = doc.createElement('div');
    wrapper.setAttribute('class', 'ring');

    const svg = doc.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('viewBox', '0 0 20 20');
    svg.setAttribute('aria-hidden', 'true');
    svg.setAttribute('focusable', 'false');

    const circle = doc.createElementNS(SVG_NS, 'circle');
    circle.setAttribute('cx', '10');
    circle.setAttribute('cy', '10');
    circle.setAttribute('r', String(PANEL_RING_RADIUS));
    circle.setAttribute('fill', 'none');
    circle.setAttribute('stroke', '#34c759');
    circle.setAttribute('stroke-width', '2');
    circle.setAttribute('stroke-linecap', 'round');
    circle.setAttribute('transform', 'rotate(-90 10 10)');
    circle.setAttribute('stroke-dasharray', String(PANEL_RING_CIRCUMFERENCE));
    const fraction = panelRingFraction(Date.now());
    circle.style.strokeDashoffset = String(PANEL_RING_CIRCUMFERENCE * (1 - fraction));

    svg.appendChild(circle);
    wrapper.appendChild(svg);
    return { wrapper, circle };
  }

  function buildPanelContent(documentRef, routed, callbacks) {
    const doc = documentRef || document;
    const root = doc.createElement('div');
    // Class names, not inline styles: the rules live in PANEL_HOST_STYLE inside the panel's
    // own closed shadow root, where no page rule can reach them.
    root.setAttribute('class', 'panel');
    const rows = [];
    // Every ring's <circle> element, so the caller (togglePanel) can animate them on a single
    // shared ticker while the panel is open — buildPanelContent itself stays a pure, timer-free
    // function so it remains directly callable from node tests.
    const ringCircles = [];
    let unlockButton = null;

    if (routed.panel === 'entries') {
      for (const entry of routed.entries) {
        const row = doc.createElement('button');
        row.setAttribute('type', 'button');
        row.setAttribute('class', 'row');
        row.setAttribute('data-entry-id', entry.id);

        const avatar = doc.createElement('div');
        avatar.setAttribute('class', 'avatar');
        avatar.style.backgroundColor = colorForTitle(entry.title);
        const avatarLetter = doc.createElement('span');
        avatarLetter.textContent = letterFromTitle(entry.title);
        avatar.appendChild(avatarLetter);
        row.appendChild(avatar);

        const textCol = doc.createElement('div');
        textCol.setAttribute('class', 'entry-text');
        const titleEl = doc.createElement('div');
        titleEl.setAttribute('class', 'entry-title');
        titleEl.textContent = entry.title;
        textCol.appendChild(titleEl);
        if (entry.username) {
          const usernameEl = doc.createElement('div');
          usernameEl.setAttribute('class', 'entry-username');
          usernameEl.textContent = entry.username;
          textCol.appendChild(usernameEl);
        }
        row.appendChild(textCol);

        if (entry.hasTotp) {
          const ring = buildPanelRing(doc);
          row.appendChild(ring.wrapper);
          ringCircles.push(ring.circle);
        }

        row.addEventListener('click', (event) => callbacks.onSelect(event, entry.id));
        root.appendChild(row);
        rows.push(row);
      }
    } else {
      const message = doc.createElement('div');
      message.setAttribute('class', 'message');
      message.textContent = PANEL_MESSAGES[routed.panel] || PANEL_MESSAGES.unavailable;
      root.appendChild(message);

      if (routed.panel === 'locked') {
        unlockButton = doc.createElement('button');
        unlockButton.setAttribute('type', 'button');
        unlockButton.setAttribute('class', 'unlock');
        unlockButton.textContent = 'Unlock MyPass';
        unlockButton.addEventListener('click', (event) => callbacks.onUnlock(event));
        root.appendChild(unlockButton);
      }
    }

    return { root, rows, unlockButton, ringCircles };
  }

  // The inline panel's "Generate password" action — a plain row, not one of
  // buildPanelContent's credential rows, since it has no entry behind it. Appended
  // by togglePanel only for a field kind === 'password' with isNewPassword === true.
  function buildGeneratePasswordRow(doc, onClick) {
    const row = doc.createElement('button');
    row.setAttribute('type', 'button');
    row.setAttribute('class', 'row generate-password-row');
    row.textContent = 'Generate password';
    row.addEventListener('click', onClick);
    return row;
  }

  // "MyPass settings", the last row of the picker in every state. It carries no page data and
  // the caller sends one fixed message, so there is nothing for a page to steer.
  function buildSettingsRow(doc, onClick) {
    const row = doc.createElement('button');
    row.setAttribute('type', 'button');
    row.setAttribute('class', 'row settings-row');
    row.textContent = 'MyPass settings';
    row.addEventListener('click', (event) => onClick(event));
    return row;
  }

  // Decides what the generate-password row's click handler does with the bridge's
  // response — pure, so the branch a user actually sees is unit-testable without a DOM.
  // A successful generatedPassword response fills; every other shape surfaces a message
  // instead of silently closing the panel. Task 11 deliberately split "your own generator
  // has no character types enabled" from "this site's requirements could not be
  // satisfied" into two different `error` messages, specifically so each points the user
  // at the right place to fix it — collapsing them back into one generic string here would
  // undo that, so an `error` response's own `message` is always used verbatim. Falls back
  // to PANEL_MESSAGES.unavailable only when there is no usable response at all (extension
  // context lost, native host unreachable with no message attached).
  function classifyGeneratePasswordResponse(response) {
    if (response && response.type === 'generatedPassword' && response.password) {
      return { action: 'fill', password: response.password };
    }
    if (response && response.type === 'locked') {
      return { action: 'showMessage', message: PANEL_MESSAGES.locked };
    }
    if (response && response.type === 'pairingRequired') {
      return { action: 'showMessage', message: PANEL_MESSAGES.pairingRequired };
    }
    if (response && response.type === 'error' && response.message) {
      return { action: 'showMessage', message: response.message };
    }
    return { action: 'showMessage', message: PANEL_MESSAGES.unavailable };
  }

  // Renders a generatePassword failure using the panel's own vocabulary — the same
  // `.message` element buildPanelContent already renders for locked/pairingRequired/
  // unavailable panels — rather than inventing a second presentation for this one action.
  // Replaces any message already shown so retrying (e.g. after unlocking) doesn't stack
  // duplicates.
  function showGeneratePasswordMessage(doc, root, text) {
    const existing = root.querySelector('.generate-password-message');
    if (existing) existing.remove();
    const message = doc.createElement('div');
    message.setAttribute('class', 'message generate-password-message');
    message.textContent = text;
    root.appendChild(message);
  }

  // Set by the browser-only wiring below, once icons/observers/panel actually exist to tear
  // down. Stays a no-op under node and in the sliver of time before that wiring runs — there
  // is nothing to release yet in either case.
  let handleExtensionContextLost = () => {};

  // An unpacked extension reload destroys the context of content scripts already injected
  // into open tabs, while leaving their DOM on the page. `chrome.runtime.id` is the cheapest
  // reliable signal: it is undefined once the context is gone, and reading it never throws.
  // Every message this script sends must be guarded by it, or a stale icon throws
  // "Extension context invalidated" on click instead of quietly doing nothing.
  function isExtensionContextAlive(chromeRef) {
    const ref = chromeRef || (typeof chrome !== 'undefined' ? chrome : null);
    return !!(ref && ref.runtime && ref.runtime.id);
  }

  // The single choke point every chrome.runtime.sendMessage call in this file goes through.
  // When the context is dead the send is skipped — never attempted, never thrown — and
  // handleExtensionContextLost runs so any injected UI (icons, an open panel, observers, the
  // ring ticker) is torn down instead of being left on the page looking alive. `chromeRef` is
  // test-only plumbing, mirroring isExtensionContextAlive above; real call sites omit it and
  // get the global `chrome`.
  function sendRuntimeMessage(message, callback, chromeRef) {
    const ref = chromeRef || (typeof chrome !== 'undefined' ? chrome : null);
    if (!isExtensionContextAlive(ref)) {
      handleExtensionContextLost();
      return;
    }
    ref.runtime.sendMessage(message, callback);
  }

  if (typeof chrome !== 'undefined' && typeof document !== 'undefined') {
    // Reference comparison only — reading properties of a cross-origin window.top throws,
    // comparing the reference does not.
    const IS_TOP_FRAME = window.top === window;
    const runsHere = (feature) => isFeatureEnabledInFrame(feature, IS_TOP_FRAME);

    const state = {
      lastCredentialList: [],
      lastLoginPrompt: null,
      vaultLocked: false,
      totpBannerDismissed: null,
    };
    // Defaults match today's page until chrome.storage answers. Icons wait for
    // that answer so a saved "off" does not flash an icon first.
    let pageAutofillSettings = shared.autofillSettings.defaultAutofillSettings();
    let autofillSettingsReady = false;

    function requestCredentials() {
      sendRuntimeMessage({ type: 'getCredentials', url: location.href }, (response) => {
        // Track lock-state separately from "empty list": a `locked` response
        // also carries no credentials, but is NOT the same as "genuinely no
        // matches for this site" — conflating the two lets the save-banner
        // logic below offer to save a duplicate when the vault was simply
        // locked at page-load time.
        state.vaultLocked = !!(response && (response.type === 'locked' || response.type === 'pairingRequired'));
        state.lastCredentialList = (response && response.type === 'credentialList') ? response.credentials : [];
      });
    }

    const attachedIcons = new Map();
    let openPanel = null;
    let syncScheduled = false;
    let panelRequestId = 0;
    // The MutationObserver returned by observeTotpCandidates() below, captured so
    // teardownExtensionContext can disconnect it once the extension context dies.
    let mutationObserver = null;
    // The field whose panel row was just clicked, so applyFill knows which group to fill
    // instead of guessing the first password field in document order (the bug this fixes).
    // Set unconditionally in onSelect, before handleEntrySelect runs — a rejected untrusted
    // activation simply never produces an applyFill message, so a stale value here is inert.
    //
    // A fill request that is never answered (locked vault, native error, origin mismatch)
    // would otherwise leave the target set forever, and the next unrelated fill — including
    // one started from the popup, which has no clicked field — would inherit it. The target
    // is therefore consumed on first use and expires on its own. A native round trip is well
    // under a second; 5s is generous.
    //
    // Nothing ties a target to its own response, so two fills issued back-to-back have their
    // first response consume whatever target is current at that moment. That is deliberate,
    // not overlooked: consume-once plus the TTL narrows the failure mode to "the most
    // recently clicked field wins," which is the right behaviour for a user who just clicked
    // something newer.
    const FILL_TARGET_TTL_MS = 5000;
    let pendingFillTarget = null;

    function setPendingFillTarget(input, kind, now) {
      pendingFillTarget = { input, kind, at: now };
    }

    function takePendingFillTarget(now) {
      const target = pendingFillTarget;
      pendingFillTarget = null;
      return isFillTargetUsable(target, now, FILL_TARGET_TTL_MS) ? target : null;
    }

    function releaseIcon(input, record) {
      if (record.resizeObserver) record.resizeObserver.disconnect();
      record.host.remove();
      attachedIcons.delete(input);
    }

    // Drops hosts and ResizeObservers whose input has left the document. Called from both
    // syncIconPositions and attachIcons: an SPA that removes fields without adding any and
    // without a scroll fires no scroll/resize, and a disconnected input fires no
    // ResizeObserver either, so syncIconPositions alone would never run and the stale hosts
    // would persist. attachIcons is the MutationObserver's debounced callback, which does
    // run on exactly that DOM change.
    function pruneDetachedIcons() {
      attachedIcons.forEach((record, input) => {
        if (input.isConnected) return;
        if (openPanel && openPanel.input === input) closePanel();
        releaseIcon(input, record);
      });
    }

    // Places one icon host and, the first time it lands in a given mount parent, measures where
    // it really ended up. A mismatch (an `html` transform, CSS zoom, a transformed dialog) is
    // stored on the record and reused, so scrolling does not force a layout read per frame.
    function placeIconHost(record, input, rect) {
      const parent = mountParentFor(input);
      if (parent && record.host.parentNode !== parent) {
        parent.appendChild(record.host);
        record.verified = false;
        record.verifyTries = 0;
        record.offset = null;
      }
      const found = findFieldBox(input);
      const box = found ? found.rect : null;
      const controls = fieldControlRects(input, found ? found.element : null);
      const result = positionHostToField(
        record.host, rect, window, controls, fieldClipRect(input, window), record.offset, box
      );
      if (result.fits && !record.verified) {
        const settled = settlePlacementOffset(record.host, result.intended, record.offset, (offset) => {
          positionHostToField(
            record.host, rect, window, controls, fieldClipRect(input, window), offset, box
          );
        });
        record.offset = settled.offset;
        if (!settled.settled) record.verifyTries = (record.verifyTries || 0) + 1;
        record.verified = settled.settled
          ? isIdentityOffset(settled.offset)
          : record.verifyTries >= PLACEMENT_VERIFY_TRIES;
      }
    }

    function syncIconPositions() {
      syncScheduled = false;
      pruneDetachedIcons();
      attachedIcons.forEach((record, input) => {
        const rect = input.getBoundingClientRect();
        const offscreen = rect.bottom < 0 || rect.top > window.innerHeight || rect.width === 0;
        // important for the same reason as buildIconHost/positionHostToField: a plain
        // assignment here would silently drop the !important flag those set initially.
        record.host.style.setProperty('display', offscreen ? 'none' : 'block', 'important');
        if (!offscreen) placeIconHost(record, input, rect);
      });
      positionOpenPanel();
    }

    function scheduleSync() {
      if (syncScheduled) return;
      syncScheduled = true;
      window.requestAnimationFrame(syncIconPositions);
    }

    function attachIconTo(input, kind, isNewPassword) {
      if (attachedIcons.has(input)) return;
      // Same reason as observeTotpCandidates: a bodyless document (SVG/XML in an iframe)
      // has nowhere to mount the host. Such a document cannot contain an <input> either,
      // so this should be unreachable — but "should be" is not a guarantee worth an
      // uncaught TypeError in a stranger's ad frame.
      if (!document.body || !document.documentElement) return;
      const { host, button } = buildIconHost(document);
      button.addEventListener('click', (event) => {
        handleIconActivate(event, () => togglePanel(input, kind, isNewPassword));
      });
      const record = { host, button, verified: false, offset: null };
      attachedIcons.set(input, record);
      // Position synchronously before the first paint so the icon never flashes at the
      // viewport origin and then jumps into place on the next rAF. The host must be in the
      // document before it can be measured, so mount first (placeIconHost does that too).
      const parent = mountParentFor(input);
      parent.appendChild(host);
      placeIconHost(record, input, input.getBoundingClientRect());

      const resizeObserver = new ResizeObserver(scheduleSync);
      resizeObserver.observe(input);
      record.resizeObserver = resizeObserver;
      scheduleSync();
    }

    function attachIcons() {
      if (!autofillSettingsReady) return;
      pruneDetachedIcons();
      if (!pageAutofillSettings.showFieldIcon) {
        attachedIcons.forEach((record, input) => {
          if (openPanel && openPanel.input === input) closePanel();
          releaseIcon(input, record);
        });
        return;
      }
      for (const field of findFillableFields(document)) {
        attachIconTo(field.input, field.kind, field.isNewPassword);
      }
      scheduleSync();
    }

    // The single ticker driving every ring in the currently-open panel. Kept outside
    // openPanel so closePanel can always reach it and clear it unconditionally — panels open
    // and close constantly, and an interval left running after close is a leak.
    let panelRingTimer = null;

    function stopPanelRingAnimation() {
      if (panelRingTimer === null) return;
      clearInterval(panelRingTimer);
      panelRingTimer = null;
    }

    function closePanel() {
      // Bumping here is what makes Escape / outside-click cancel an in-flight request,
      // not just remove an already-built panel.
      panelRequestId += 1;
      // Wired in unconditionally (not inside the `if (!openPanel)` branch below) so the ring
      // ticker can never outlive the panel it belonged to, no matter which path led here.
      // stopPanelRingAnimation is itself a no-op once already stopped, so this is idempotent.
      stopPanelRingAnimation();
      if (!openPanel) return;
      setIconExpanded(openPanel.input, false);
      openPanel.host.remove();
      openPanel = null;
    }

    // aria-expanded mirrors whether this field's picker is open. Set on open, and cleared by
    // closePanel for every way a panel goes away (Escape, outside click, selection, the field
    // leaving the document).
    function setIconExpanded(input, expanded) {
      const record = attachedIcons.get(input);
      if (record && record.button) record.button.setAttribute('aria-expanded', expanded ? 'true' : 'false');
    }

    // Opens below the field, flips above when there is not enough room, clamps into the
    // viewport, and caps the height so a long list scrolls inside the panel. Re-run on every
    // scroll/resize sync because the host is fixed. A field scrolled out of view closes the
    // panel instead of leaving it floating over unrelated content.
    function positionOpenPanel() {
      if (!openPanel) return;
      const rect = openPanel.input.getBoundingClientRect();
      if (rect.bottom < 0 || rect.top > window.innerHeight || rect.width === 0) {
        closePanel();
        return;
      }
      const host = openPanel.host;
      const box = host.getBoundingClientRect();
      // placeFloating works on the picker's RENDERED size and answers in viewport pixels; the
      // host's styles are LOCAL pixels of its containing block, so every length it receives is
      // divided by that block's scale (rendered over written size, as for the icon). The
      // position stays viewport / scale + a cached offset, which keeps it right while scrolling.
      // Taken once. The picker is mounted under the same parent as the field's icon, so it shares
      // that icon's containing block, and the icon's scale is exact (the size it wrote over the
      // size it rendered). Only without one is the scale read off the picker itself, from
      // offsetWidth/offsetHeight, which are rounded; re-reading them on every scroll frame
      // would let the scale jitter.
      if (!openPanel.scale && box.width > 0 && box.height > 0) {
        const iconRecord = attachedIcons.get(openPanel.input);
        const iconScale = iconRecord && iconRecord.offset;
        openPanel.scale = iconScale && (iconScale.sx || iconScale.sy)
          ? { x: shared.placement.usableScale(iconScale.sx), y: shared.placement.usableScale(iconScale.sy) }
          : shared.placement.scaleFrom(
            box, localExtent(host, 'width', host.offsetWidth), localExtent(host, 'height', host.offsetHeight)
          );
      }
      const scale = openPanel.scale || { x: 1, y: 1 };
      const placed = shared.placement.placeFloating(
        rect, { width: Math.max(box.width, 1), height: Math.max(box.height, 1) }, window
      );
      const write = (offset) => {
        host.style.setProperty('top', String(placed.top / scale.y + offset.dy) + 'px', 'important');
        host.style.setProperty('left', String(placed.left / scale.x + offset.dx) + 'px', 'important');
      };
      host.style.setProperty('--mypass-panel-max-height', String(placed.maxHeight / scale.y) + 'px', 'important');
      write(openPanel);
      host.style.setProperty('min-width', String(Math.min(rect.width, 320) / scale.x) + 'px', 'important');
      if (!openPanel.verified) {
        const settled = settlePlacementOffset(
          host, { left: placed.left, top: placed.top }, { dx: openPanel.dx, dy: openPanel.dy }, write
        );
        openPanel.dx = settled.offset.dx;
        openPanel.dy = settled.offset.dy;
        if (!settled.settled) openPanel.verifyTries = (openPanel.verifyTries || 0) + 1;
        openPanel.verified = settled.settled
          ? isIdentityOffset(settled.offset)
          : openPanel.verifyTries >= PLACEMENT_VERIFY_TRIES;
      }
    }

    // Runs once sendRuntimeMessage detects the extension context is gone (an unpacked
    // reload orphaned this content script while its DOM stayed on the page). Reuses the
    // existing release machinery rather than a parallel teardown path: closePanel() already
    // clears the ring ticker unconditionally, and releaseIcon() is the same function
    // pruneDetachedIcons uses per-icon. Idempotent — closePanel/releaseIcon/disconnect are
    // all safe to call again, and a second pass here simply finds nothing left to do.
    function teardownExtensionContext() {
      closePanel();
      Array.from(attachedIcons.entries()).forEach(([input, record]) => releaseIcon(input, record));
      if (mutationObserver) mutationObserver.disconnect();
      if (totpImageLoadListener) {
        document.removeEventListener('load', totpImageLoadListener, true);
        totpImageLoadListener = null;
        totpImageLoadListenerAttached = false;
      }
    }
    handleExtensionContextLost = teardownExtensionContext;

    function togglePanel(input, kind, isNewPassword) {
      const wasOpenForThisInput = !!(openPanel && openPanel.input === input);
      closePanel();
      if (wasOpenForThisInput) return;

      panelRequestId += 1;
      const requestId = panelRequestId;
      // Always re-ask rather than trusting state.lastCredentialList: the vault may have
      // been unlocked since page load, and a stale locked panel is the exact failure
      // Stage 3 shipped and had to fix.
      sendRuntimeMessage({ type: 'getCredentials', url: location.href }, (response) => {
        // Superseded by a later open, or cancelled by a close, while we were waiting.
        if (requestId !== panelRequestId) return;
        const routed = chooseInlinePanel(response, kind, pageAutofillSettings);
        // Handle for manual verification only (RUN.md) — never used as logic. Identity
        // against `host` is what closePanel/isOwnHost actually rely on. The panel is a
        // list, not a button, so it gets the bare scaffolding — no icon glyph inside it.
        const { host, shadow } = buildPanelHost(document);
        const content = buildPanelContent(document, routed, {
          onSelect: (event, entryId) => {
            setPendingFillTarget(input, kind, Date.now());
            if (handleEntrySelect(event, entryId, location.href, (m) => sendRuntimeMessage(m))) {
              closePanel();
            }
          },
          onUnlock: (event) => {
            if (isTrustedActivation(event)) {
              sendRuntimeMessage({ type: 'requestUnlock' });
              closePanel();
            }
          },
        });
        if (kind === 'password' && isNewPassword) {
          const generateRow = buildGeneratePasswordRow(document, trustedClickHandler(() => {
            sendRuntimeMessage({ type: 'generatePassword', domain: location.hostname, url: location.href }, (genResponse) => {
              // Superseded by a later open, or cancelled by a close, while the request
              // was in flight — same staleness guard the credential fetch above uses, so
              // a delayed response never mutates a panel that is no longer the open one.
              if (requestId !== panelRequestId) return;
              const outcome = classifyGeneratePasswordResponse(genResponse);
              if (outcome.action === 'fill') {
                setNativeValue(input, outcome.password);
                closePanel();
                return;
              }
              // A locked vault, a missing pairing, or a typed error (including the two
              // Task 11 split apart) must reach the user here — this is the surface they
              // just clicked on — rather than the panel silently closing with nothing to
              // show for it.
              showGeneratePasswordMessage(document, content.root, outcome.message);
            });
          }));
          content.root.appendChild(generateRow);
        }
        // Last row in every state, so settings stay reachable from the page even in a window
        // with no toolbar button to click.
        content.root.appendChild(buildSettingsRow(document, (event) => {
          if (!isTrustedActivation(event)) return;
          sendRuntimeMessage({ type: 'openMyPassSettings' });
          closePanel();
        }));
        shadow.appendChild(content.root);
        // Mounted invisibly first so it can be measured before the flip/clamp decision.
        host.style.setProperty('visibility', 'hidden', 'important');
        host.style.setProperty('top', '0px', 'important');
        host.style.setProperty('left', '0px', 'important');
        mountParentFor(input).appendChild(host);
        openPanel = { host, input, dx: 0, dy: 0, verified: false };
        setIconExpanded(input, true);
        positionOpenPanel();
        if (openPanel && openPanel.host === host) host.style.setProperty('visibility', 'visible', 'important');

        // Animate the countdown ring(s) on a single shared 1Hz ticker while the panel stays
        // open — purely cosmetic, driven by the wall clock (panelRingFraction), never by the
        // entry's secret. No-op when this panel has no TOTP entries.
        if (content.ringCircles.length > 0) {
          panelRingTimer = setInterval(() => {
            const offset = PANEL_RING_CIRCUMFERENCE * (1 - panelRingFraction(Date.now()));
            for (const circle of content.ringCircles) {
              circle.style.strokeDashoffset = String(offset);
            }
          }, 1000);
        }
      });
    }

    function attachSubmitListener(loginForm) {
      if (!loginForm.form) return;
      loginForm.form.addEventListener('submit', (event) => {
        // A save prompt opens browser UI (the popup or a window), so only a submit the user
        // really caused may raise one. A synthetic `submit` is refused by isTrusted, but
        // form.requestSubmit() is "trusted" too, so a live user activation is required as well
        // (see isUserSubmit); the background additionally throttles opening per tab.
        if (!isUserSubmit(event)) return;
        if (
          runsHere('saveBanner') &&
          pageAutofillSettings.offerSaveLogin &&
          !state.vaultLocked && state.lastCredentialList.length === 0
        ) {
          const username = loginForm.usernameInput ? loginForm.usernameInput.value : '';
          const password = loginForm.passwordInput.value;
          if (password) {
            requestSavePrompt(location.href, username, password, state, sendRuntimeMessage);
          }
        }
      });
    }

    let addressSnapshot = null;
    chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
      // The background asks the exact document that raised a prompt (tab, frame, documentId)
      // whether it is still the one on screen before it shows or saves anything. A navigated
      // document has a new documentId, so this never even arrives; the url lets the background
      // also catch a same-document history.pushState to another host. Only the extension's own
      // background can reach this listener, and nothing secret is returned.
      if (message.type === 'promptStillValid') {
        if (_sender && _sender.id !== chrome.runtime.id) return;
        sendResponse({ type: 'ok', url: location.href });
        return;
      }
      if (message.type === 'queryAddressForm') {
        if (window.top !== window || message.url !== location.href) return;
        const groups = pageAutofillSettings.showAddresses ? shared.addressForm.collect(document) : [];
        addressSnapshot = { challenge: message.challenge, url: location.href, groups, at: Date.now() };
        // The background obtains documentId from Chrome's authenticated sender,
        // not from a value supplied by the page or the popup.
        chrome.runtime.sendMessage({ type: 'addressFormReport', challenge: message.challenge,
          forms: shared.addressForm.describe(groups, document) }, () => { void chrome.runtime.lastError; });
        sendResponse({ type: 'ok' });
        return;
      }
      if (message.type === 'applyAddress') {
        const snapshot = addressSnapshot;
        addressSnapshot = null; // one explicit selection can fill only once
        if (window.top !== window || !snapshot || snapshot.challenge !== message.challenge
          || snapshot.url !== location.href || message.url !== location.href
          || Date.now() - snapshot.at > 120000 || !message.address) {
          sendResponse({ type: 'error', code: 'address_form_changed' });
          return;
        }
        sendResponse(shared.addressForm.apply(document, snapshot.groups, message.groupId,
          message.address, shared.addressForm.setNativeAddressValue, () => snapshot.url === location.href));
        return;
      }

      if (message.type === 'queryTotpCandidate') {
        // Only the top frame replies, or the popup sees multiple frames' responses race —
        // no sendResponse at all here, not a "false" response, so the popup keeps waiting
        // on whichever frame is actually allowed to answer.
        if (!runsHere('totpCandidateQuery')) return;
        sendResponse(buildTotpCandidateQueryResponse(document));
        return;
      }
      if (message.type === 'queryNewPasswordField') {
        if (!runsHere('newPasswordFieldQuery')) return;
        sendResponse(buildNewPasswordFieldQueryResponse(document));
        return;
      }
      if (message.type === 'queryPageIdentifier') {
        if (!runsHere('pageIdentifierQuery')) return;
        const found = shared.pageIdentifier.readPageIdentifier(document, {
          isFieldVisible, findOtpField, scoreUsernameLikeness
        });
        sendResponse(found || { value: '', kind: '' });
        return;
      }
      if (message.type === 'applyFill') {
        // Consumed before the origin guard, not after: a rejected message must still clear
        // the target, or a hostname-mismatched applyFill would leave the clicked field armed
        // for whatever fill arrives next — including one started from the popup, which has
        // no clicked field of its own. Every path through this branch consumes it, so no
        // applyFill — accepted or rejected — can ever leave a target behind.
        const target = takePendingFillTarget(Date.now());

        if (!hostnameMatches(message.url, location.href)) {
          console.warn('MyPass: applyFill rejected — message URL does not match the current page hostname');
          return;
        }
        // See resolveApplyFillTargets for the priority order and for why a
        // `fillTarget: 'newPassword'` message must never fall back to the first password
        // field on the page.
        const { usernameInput, passwordInput, otpInput } = resolveApplyFillTargets(message, target, document);

        if (usernameInput && message.username) {
          setNativeValue(usernameInput, message.username);
        }
        let wrotePassword = false;
        if (passwordInput && message.password) {
          setNativeValue(passwordInput, message.password);
          wrotePassword = true;
        }
        if (message.totp && otpInput) {
          setNativeValue(otpInput, message.totp);
        }
        // Opt-in only. click() is not a trusted user click; some sites ignore it.
        // A generated password (fillTarget newPassword) and a code-only fill do not submit.
        if (wrotePassword) {
          shared.autofillSettings.maybeSubmitFilledLogin(
            passwordInput, pageAutofillSettings, true, message.fillTarget
          );
        }
      }
    });

    // Detect the form and request the current domain's credential list on
    // load, but NEVER auto-fill here — filling only happens in response to
    // an explicit "applyFill" message triggered by a popup click (Task 12).
    const loginForm = findLoginForm();
    if (loginForm) {
      requestCredentials();
      attachSubmitListener(loginForm);
    }
    function scanTotpBanner() {
      if (!runsHere('totpScan') || !pageAutofillSettings.offerSaveTotp) return;
      const detected = detectTotpCandidateWithPriority(document);
      if (shouldShowTotpBanner(detected, state)) {
        requestTotpPrompt(location.href, detected.candidate, state, detected.priority, sendRuntimeMessage);
      }
    }

    function loadPageAutofillSettings() {
      const key = shared.autofillSettings.AUTOFILL_SETTINGS_KEY;
      const apply = (value) => {
        pageAutofillSettings = shared.autofillSettings.normalizeAutofillSettings(value);
        autofillSettingsReady = true;
        attachIcons();
        scanTotpBanner();
      };
      const storage = chrome.storage && chrome.storage.local;
      if (!storage || typeof storage.get !== 'function') {
        apply(null);
        return;
      }
      storage.get([key], (stored) => {
        if (chrome.runtime && chrome.runtime.lastError) {
          apply(null);
          return;
        }
        apply(stored && stored[key]);
      });
      if (chrome.storage.onChanged && typeof chrome.storage.onChanged.addListener === 'function') {
        chrome.storage.onChanged.addListener((changes, area) => {
          if (area !== 'local' || !changes[key]) return;
          pageAutofillSettings = shared.autofillSettings.normalizeAutofillSettings(changes[key].newValue);
          if (!autofillSettingsReady) return;
          attachIcons();
        });
      }
    }
    loadPageAutofillSettings();

    // A vault row click opens this tab, then asks the app for the login. The popup
    // cannot message a tab it just created (activeTab covers only the tab that was
    // current when the icon was clicked), so this page claims the fill itself.
    // One retry covers the race where document_idle beats the popup's arm message.
    function claimPendingOpenFill(attempt) {
      if (!runsHere('pendingOpenFill')) return;
      sendRuntimeMessage({ type: 'claimPendingFill', url: location.href }, (response) => {
        if (response && response.type === 'none' && attempt < 1) {
          setTimeout(() => claimPendingOpenFill(attempt + 1), 700);
        }
      });
    }
    claimPendingOpenFill(0);

    window.addEventListener('scroll', scheduleSync, { capture: true, passive: true });
    window.addEventListener('resize', scheduleSync, { passive: true });
    // A focus change often follows a layout change the observers cannot see (an accordion, a
    // sticky header, a field revealed by a transition). One cheap re-measure covers it.
    document.addEventListener('focusin', scheduleSync, true);
    // A site's clear button usually appears on the first keystroke by dropping its `hidden`
    // attribute, which no observer here sees (childList only), so the icon would sit on top of
    // it until some other event re-measured. Typing is that event; scheduleSync is rAF-coalesced.
    document.addEventListener('input', scheduleSync, true);

    function isOwnHost(node) {
      if (!node) return false;
      if (openPanel && node === openPanel.host) return true;
      for (const record of attachedIcons.values()) {
        if (node === record.host) return true;
      }
      return false;
    }

    document.addEventListener('click', (event) => {
      if (!openPanel) return;
      // Our own control: let its own handler decide (the icon toggles, a row fills).
      // Identity, not the marker attribute — a page can forge the attribute on its
      // own elements, but it cannot forge a reference to a node we created. This runs
      // in the capture phase (before the icon/panel's own bubble-phase handlers), so
      // returning early here — rather than closing — is what keeps toggle-to-close
      // working: the icon's own click handler still gets to run and close the panel.
      if (isOwnHost(event.target)) return;
      closePanel();
    }, true);
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') closePanel();
    }, true);

    // A trusted paste or copy of a login MyPass just filled, on a site that is
    // not saved with that item. The text goes to the service worker for the
    // comparison. The page only sees the warning, never the stored secret.
    let securityWarningEl = null;
    function showSecurityWarning(action) {
      const message = action === 'paste'
        ? 'This site is not saved with this MyPass password.'
        : 'This site is not saved with this MyPass login.';
      if (securityWarningEl && securityWarningEl.isConnected) {
        securityWarningEl.firstChild.textContent = message;
        return;
      }
      const banner = document.createElement('div');
      banner.setAttribute('role', 'status');
      banner.style.cssText =
        'position:fixed;bottom:16px;left:16px;right:16px;z-index:2147483647;background:#fff7ed;' +
        'color:#9a3412;padding:12px 14px;border-radius:8px;border:1px solid #fdba74;' +
        'font-family:sans-serif;font-size:13px;line-height:1.35;box-shadow:0 2px 8px rgba(0,0,0,0.18);' +
        'display:flex;align-items:center;gap:8px;';
      const text = document.createElement('span');
      text.textContent = message;
      text.style.cssText = 'flex:1;';
      const closeButton = document.createElement('button');
      closeButton.type = 'button';
      closeButton.textContent = 'Dismiss';
      closeButton.style.cssText = 'cursor:pointer;border:0;background:#9a3412;color:#fff;border-radius:6px;padding:4px 8px;';
      closeButton.addEventListener('click', trustedClickHandler(() => {
        banner.remove();
        securityWarningEl = null;
      }));
      banner.appendChild(text);
      banner.appendChild(closeButton);
      (document.body || document.documentElement).appendChild(banner);
      securityWarningEl = banner;
    }
    function reportSecurityUse(action, text, event) {
      if (!isTrustedActivation(event)) return;
      if (typeof text !== 'string' || text.length < securityWatch.SECURITY_MATCH_MIN_LENGTH) return;
      sendRuntimeMessage({
        type: 'checkSecurityUse',
        action: action,
        text: text.slice(0, 500),
        url: location.href
      }, (result) => {
        if (result && result.warn === true) showSecurityWarning(action);
      });
    }
    document.addEventListener('paste', (event) => {
      const data = event.clipboardData && event.clipboardData.getData('text/plain');
      reportSecurityUse('paste', data || '', event);
    }, true);
    document.addEventListener('copy', (event) => {
      const target = event.target;
      const selection = window.getSelection ? String(window.getSelection()) : '';
      const text = securityWatch.textFromCopyTarget(
        target && target.value,
        target && target.selectionStart,
        target && target.selectionEnd,
        selection
      );
      reportSecurityUse('copy', text, event);
    }, true);

    mutationObserver = observeTotpCandidates(
      state, attachIcons, IS_TOP_FRAME, () => autofillSettingsReady && pageAutofillSettings.offerSaveTotp
    );
  }

  if (typeof module !== 'undefined') {
    module.exports = {
      scoreUsernameLikeness, findBestUsernameInput, hostnameMatches, scoreOtpLikeness, findOtpField,
      isFieldVisible, findFillableFields, findLoginForm, resolveFillTargets, resolveApplyFillTargets,
      isNewPasswordCandidate,
      isFillTargetUsable,
      detectOtpauthLink, detectOtpauthText, isBase32Secret, detectManualSetupKey,
      decodeQrFromImage, detectQrCandidate,
      detectTotpCandidateWithPriority, shouldShowTotpBanner,
      classifyGetCredentialsResponse, chooseInlinePanel, requestSavePrompt, requestTotpPrompt,
      isTrustedActivation, hasLiveUserActivation, isUserSubmit, handleIconActivate, handleEntrySelect, trustedClickHandler,
      buildTotpCandidateQueryResponse, buildNewPasswordFieldQueryResponse, buildGeneratePasswordRow,
      classifyGeneratePasswordResponse, showGeneratePasswordMessage,
      KEY_ICON_PATH_D, buildKeyGlyph, buildShadowHost, buildIconHost, positionHostToField, fieldControlRects, fieldClipRect, fieldVisualBox, findFieldBox,
      mountParentFor, measurePlacementOffset, settlePlacementOffset, buildSettingsRow,
      buildPanelHost, buildPanelContent, ICON_HOST_STYLE, PANEL_HOST_STYLE,
      panelRingFraction,
      FRAME_FEATURE_POLICY, isFeatureEnabledInFrame, buildMutationSweep, observeTotpCandidates,
      isExtensionContextAlive, sendRuntimeMessage,
    };
  }
})();
