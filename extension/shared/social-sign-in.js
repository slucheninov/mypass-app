// Social sign-in: which "Sign in with ..." button (Google, Facebook, Apple, Microsoft, GitHub) the
// person clicked on a site, so MyPass can remember it and point at the same button next time.
// This file only decides and draws; content.js wires the page events and background.js talks to
// the app. Three signals name a provider, strongest first:
//   1. an SDK marker (`.g_id_signin`, `.fb-login-button`, `#appleid-signin`, `data-provider`);
//   2. a link or form target on the provider's OAuth host (accounts.google.com,
//      facebook.com/dialog/oauth, appleid.apple.com, login.microsoftonline.com, login.live.com,
//      github.com/login/oauth) or a site path such as /auth/google;
//   3. the button text, aria-label or title ("Sign in with Google", "Увійти через Google",
//      "Войти через Google", "Mit Google anmelden").
//
// Classic script. The only top-level name is the `mypassSocialSignIn` bag.

var mypassSocialSignIn = (function () {
  var PROVIDERS = ['google', 'facebook', 'apple', 'microsoft', 'github'];
  var PROVIDER_NAMES = {
    google: 'Google', facebook: 'Facebook', apple: 'Apple', microsoft: 'Microsoft', github: 'GitHub'
  };
  // A pending record waits this long for the vault to be unlocked, then it is dropped.
  var PENDING_TTL_MS = 10 * 60 * 1000;
  var RING_COLOR = '#73b359';
  // The frame heuristic: the last trusted pointer event of the page must have been over the
  // provider frame (its box grown by the slop: pointer events over a cross-origin frame go to the
  // frame, so the page's last one lands on or just outside its edge) at most this long before the
  // page lost focus to it.
  var FRAME_POINTER_WINDOW_MS = 1500;
  var FRAME_POINTER_SLOP_PX = 8;
  // The hosts whose own button frames the heuristic trusts (never a site's /auth/<provider> path).
  var FRAME_HOSTS = {
    'accounts.google.com': 'google',
    'appleid.apple.com': 'apple',
    'www.facebook.com': 'facebook',
    'web.facebook.com': 'facebook',
    'facebook.com': 'facebook',
    'login.microsoftonline.com': 'microsoft',
    'login.live.com': 'microsoft',
    'github.com': 'github'
  };
  var MAX_TEXT = 120;
  // What a click can land on: the clickable element itself, or an SDK container around it.
  var CLICKABLE = 'a[href],button,[role="button"],input[type="submit"],input[type="button"],'
    + '[data-provider],.g_id_signin,.fb-login-button,#appleid-signin';
  var CANDIDATES = CLICKABLE + ',iframe[src]';

  var PROVIDER_WORD = /\b(google|facebook|apple|microsoft|github)\b/;
  // Lowercased, single-spaced phrases that put a provider name after them.
  var PHRASES = [
    // en
    'sign in with', 'sign in using', 'sign in via', 'sign in through', 'signin with', 'log in with',
    'login with', 'log in using', 'log in via', 'continue with', 'sign up with', 'connect with',
    'register with',
    // de
    'anmelden mit', 'einloggen mit', 'weiter mit', 'fortfahren mit', 'registrieren mit',
    // uk
    'увійти через', 'увійти за допомогою', 'увійти з', 'вхід через', 'продовжити з',
    'продовжити через', 'зареєструватися через',
    // ru
    'войти через', 'войти с помощью', 'войти с', 'вход через', 'продолжить с', 'продолжить через',
    'зарегистрироваться через'
  ];
  // German also puts the verb last: "Mit Google anmelden".
  var VERB_LAST = /\bmit (google|facebook|apple|microsoft|github) (anmelden|einloggen|fortfahren|registrieren)\b/;

  function isProvider(value) {
    return PROVIDERS.indexOf(value) !== -1;
  }

  function normalize(text) {
    return String(text || '').replace(/\s+/g, ' ').trim().toLowerCase().slice(0, MAX_TEXT * 2);
  }

  // A phrase plus a provider name, in one short label. Long text (a paragraph about Google) is
  // never a button label.
  function providerFromText(text) {
    var value = normalize(text);
    if (!value || value.length > MAX_TEXT) return null;
    var verbLast = value.match(VERB_LAST);
    if (verbLast) return verbLast[1];
    for (var i = 0; i < PHRASES.length; i += 1) {
      var at = value.indexOf(PHRASES[i]);
      if (at === -1) continue;
      var after = value.slice(at + PHRASES[i].length).match(PROVIDER_WORD);
      if (after) return after[1];
    }
    return null;
  }

  // A bare provider name ("Google") in an aria-label or title: icon-only buttons say no more.
  function providerFromName(text) {
    var value = normalize(text);
    return isProvider(value) ? value : null;
  }

  // The OAuth hosts and paths of each provider, and a site's own /auth/<provider> path.
  function providerFromHref(href, base) {
    if (typeof href !== 'string' || !href || href.length > 4096) return null;
    var url;
    try {
      url = new URL(href, base || undefined);
    } catch (error) {
      return null;
    }
    var host = url.hostname.toLowerCase().replace(/\.$/, '');
    var path = url.pathname.toLowerCase();
    if (url.protocol === 'https:') {
      if (host === 'accounts.google.com') return 'google';
      if ((host === 'facebook.com' || /\.facebook\.com$/.test(host)) && /^\/(v[\d.]+\/)?dialog\/oauth/.test(path)) {
        return 'facebook';
      }
      if (host === 'appleid.apple.com') return 'apple';
      if (host === 'login.microsoftonline.com' || host === 'login.live.com') return 'microsoft';
      if (host === 'github.com' && /^\/login\/oauth(\/|$)/.test(path)) return 'github';
    }
    if (url.protocol !== 'https:' && url.protocol !== 'http:') return null;
    var own = path.match(/\/(?:auth|oauth2?|login|signin|sign-in|sso|connect|social|accounts)\/(google|facebook|apple|microsoft|github)(?:[\/._-]|$)/);
    return own ? own[1] : null;
  }

  // The provider of a button frame served by the provider's own sign-in host, or null.
  function providerFromFrameSrc(src, base) {
    if (typeof src !== 'string' || !src || src.length > 4096) return null;
    try {
      var url = new URL(src, base || undefined);
      if (url.protocol !== 'https:') return null;
      var host = url.hostname.toLowerCase().replace(/\.$/, '');
      return Object.prototype.hasOwnProperty.call(FRAME_HOSTS, host) ? FRAME_HOSTS[host] : null;
    } catch (error) {
      return null;
    }
  }

  function attr(el, name) {
    return el && typeof el.getAttribute === 'function' ? el.getAttribute(name) : null;
  }

  function hasClass(el, name) {
    var value = attr(el, 'class');
    return !!value && (' ' + value + ' ').indexOf(' ' + name + ' ') !== -1;
  }

  function providerFromMarkers(el) {
    if (!el) return null;
    if (hasClass(el, 'g_id_signin') || hasClass(el, 'abcRioButton')) return 'google';
    if (hasClass(el, 'fb-login-button')) return 'facebook';
    if (attr(el, 'id') === 'appleid-signin' || hasClass(el, 'appleid-signin')) return 'apple';
    var declared = normalize(attr(el, 'data-provider'));
    if (declared) {
      for (var i = 0; i < PROVIDERS.length; i += 1) {
        if (declared.indexOf(PROVIDERS[i]) !== -1) return PROVIDERS[i];
      }
    }
    return null;
  }

  function targetOf(el, base) {
    var tag = String(el.tagName || '').toLowerCase();
    if (tag === 'a') return providerFromHref(attr(el, 'href'), base);
    if (tag === 'iframe') return providerFromHref(attr(el, 'src'), base);
    var direct = providerFromHref(attr(el, 'formaction') || attr(el, 'data-href'), base);
    if (direct) return direct;
    var form = typeof el.closest === 'function' ? el.closest('form') : null;
    var type = normalize(attr(el, 'type'));
    if (form && (tag === 'button' ? type === '' || type === 'submit' : type === 'submit')) {
      return providerFromHref(attr(form, 'action'), base);
    }
    return null;
  }

  // The provider an element names and how strongly (3 marker, 2 link, 1 text), or null.
  function detect(el, base) {
    if (!el || typeof el.getAttribute !== 'function') return null;
    var marker = providerFromMarkers(el);
    if (marker) return { provider: marker, score: 3 };
    var link = targetOf(el, base);
    if (link) return { provider: link, score: 2 };
    var label = attr(el, 'aria-label');
    var title = attr(el, 'title');
    var value = String(el.tagName || '').toLowerCase() === 'input' ? attr(el, 'value') : null;
    var text = String(el.tagName || '').toLowerCase() === 'iframe' ? '' : el.textContent;
    var fromText = providerFromText(label) || providerFromText(title) || providerFromText(value)
      || providerFromText(text) || providerFromName(label) || providerFromName(title);
    return fromText ? { provider: fromText, score: 1 } : null;
  }

  // The provider of the sign-in button a click landed on, or null. Walks up from the target to
  // the nearest clickable element (and SDK containers around it, a few levels at most).
  function providerForClickTarget(target, base) {
    var el = target && target.nodeType === 3 ? target.parentElement : target;
    var hops = 0;
    while (el && hops < 6) {
      var clickable = typeof el.closest === 'function' ? el.closest(CLICKABLE) : null;
      if (!clickable) return null;
      var found = detect(clickable, base);
      if (found) return found.provider;
      el = clickable.parentElement;
      hops += 1;
    }
    return null;
  }

  function isHttpsUrl(value) {
    try {
      var url = new URL(String(value || ''));
      return url.protocol === 'https:' && !!url.hostname;
    } catch (error) {
      return false;
    }
  }

  // The message a click sends, or null: a trusted click (never a script's `click()`) on an
  // https page with the setting on, that landed on a recognised sign-in button.
  function recordMessageForClick(event, context) {
    if (!event || event.isTrusted !== true) return null;
    if (!context || context.enabled === false || !isHttpsUrl(context.pageUrl)) return null;
    var provider = providerForClickTarget(event.target, context.pageUrl);
    return provider ? { type: 'recordSocialSignIn', pageUrl: context.pageUrl, provider: provider, source: 'click' } : null;
  }

  // Whether the trusted pointer `{x, y, at}` (client coordinates, ms) was over `rect` at most
  // FRAME_POINTER_WINDOW_MS before `now`.
  function pointerWasOver(pointer, rect, now) {
    if (!pointer || !rect || typeof now !== 'number') return false;
    var x = pointer.x;
    var y = pointer.y;
    var at = pointer.at;
    if (![x, y, at, rect.left, rect.top, rect.width, rect.height].every(function (n) {
      return typeof n === 'number' && isFinite(n);
    })) return false;
    if (rect.width <= 0 || rect.height <= 0) return false;
    var age = now - at;
    if (age < 0 || age > FRAME_POINTER_WINDOW_MS) return false;
    var slop = FRAME_POINTER_SLOP_PX;
    return x >= rect.left - slop && x <= rect.left + rect.width + slop
      && y >= rect.top - slop && y <= rect.top + rect.height + slop;
  }

  // A sign-in button that is a cross-origin iframe (Google's rendered button) never shows the
  // click to this page: the window loses focus to the frame instead. That counts only for a frame
  // served by the provider's own sign-in host, with a live user activation, and when the page's
  // last trusted pointer event (`context.pointer`, which page script cannot synthesize) was over
  // that frame within FRAME_POINTER_WINDOW_MS of the blur (`context.now`). Focus moved by script
  // alone never records. The bridge treats the result as weaker than a click (`source: frame`):
  // it never creates an entry and never overrides the once-a-day limit.
  function recordMessageForFrameFocus(event, context, doc, nav) {
    if (!event || event.isTrusted !== true) return null;
    if (!context || context.enabled === false || !isHttpsUrl(context.pageUrl)) return null;
    var active = doc && doc.activeElement;
    if (!active || String(active.tagName || '').toLowerCase() !== 'iframe') return null;
    var provider = providerFromFrameSrc(attr(active, 'src'), context.pageUrl);
    if (!provider) return null;
    var activation = nav && nav.userActivation;
    if (!activation || activation.isActive !== true) return null;
    var rect = typeof active.getBoundingClientRect === 'function' ? active.getBoundingClientRect() : null;
    if (!pointerWasOver(context.pointer, rect, context.now)) return null;
    return { type: 'recordSocialSignIn', pageUrl: context.pageUrl, provider: provider, source: 'frame' };
  }

  // The visible buttons of `provider` on the page, strongest signal first, then document order.
  function findProviderButtons(root, provider, isVisible, base) {
    if (!root || typeof root.querySelectorAll !== 'function' || !isProvider(provider)) return [];
    var visible = typeof isVisible === 'function' ? isVisible : function () { return true; };
    var found = [];
    var seen = [];
    Array.prototype.forEach.call(root.querySelectorAll(CANDIDATES), function (el, index) {
      var hit = detect(el, base);
      if (!hit || hit.provider !== provider || !visible(el)) return;
      // A marker container and the button inside it are one button: keep the outermost.
      for (var i = 0; i < seen.length; i += 1) {
        if (typeof seen[i].contains === 'function' && seen[i].contains(el)) return;
      }
      seen.push(el);
      found.push({ el: el, score: hit.score, index: index });
    });
    found.sort(function (a, b) { return b.score - a.score || a.index - b.index; });
    return found.map(function (item) { return item.el; });
  }

  // The ring's box in document coordinates: the button's box grown by `pad` on every side.
  function highlightBox(rect, scrollX, scrollY, pad) {
    var grow = typeof pad === 'number' ? pad : 4;
    return {
      left: Math.round(rect.left + (scrollX || 0) - grow),
      top: Math.round(rect.top + (scrollY || 0) - grow),
      width: Math.round(rect.width + grow * 2),
      height: Math.round(rect.height + grow * 2)
    };
  }

  // The overlay: a host outside the page's layout (absolutely positioned under <html>, no pointer
  // events) with a closed shadow root, so the page's CSS cannot restyle it and it cannot move
  // anything. Holds the green ring and the badge with `label`.
  function buildHighlight(doc, label) {
    var host = doc.createElement('mypass-social-sign-in');
    host.setAttribute('data-mypass-social-highlight', '');
    host.style.cssText = 'all:initial;position:absolute;pointer-events:none;z-index:2147483646;'
      + 'left:0;top:0;width:0;height:0;display:block;';
    var shadow = typeof host.attachShadow === 'function' ? host.attachShadow({ mode: 'closed' }) : host;
    var ring = doc.createElement('div');
    ring.setAttribute('data-part', 'ring');
    ring.style.cssText = 'box-sizing:border-box;position:absolute;inset:0;border:2px solid ' + RING_COLOR + ';'
      + 'border-radius:8px;box-shadow:0 0 0 3px rgba(115,179,89,0.25);';
    var badge = doc.createElement('div');
    badge.setAttribute('data-part', 'badge');
    badge.setAttribute('role', 'tooltip');
    badge.style.cssText = 'position:absolute;left:0;bottom:100%;margin-bottom:4px;padding:3px 8px;'
      + 'border-radius:6px;background:' + RING_COLOR + ';color:#fff;white-space:nowrap;'
      + 'font:600 12px/1.3 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;'
      + 'box-shadow:0 1px 4px rgba(0,0,0,0.2);';
    badge.textContent = label;
    shadow.appendChild(ring);
    shadow.appendChild(badge);
    return { host: host, ring: ring, badge: badge };
  }

  function placeHighlight(host, rect, scrollX, scrollY) {
    var box = highlightBox(rect, scrollX, scrollY);
    host.style.left = box.left + 'px';
    host.style.top = box.top + 'px';
    host.style.width = box.width + 'px';
    host.style.height = box.height + 'px';
  }

  // The lowercase host of an https URL, or null.
  function httpsHost(value) {
    try {
      var url = new URL(String(value || ''));
      return url.protocol === 'https:' && url.hostname ? url.hostname.toLowerCase() : null;
    } catch (error) {
      return null;
    }
  }

  // What a page may learn from a lookup. The ring is part of the page's DOM, so the page can see
  // it: only a method recorded on exactly this page's host (`response.host`) is passed on, never
  // one recorded on a sibling host under the same registrable domain or picked in the app (no
  // host). Anything else reaches the page as "no method" (`enabled` is kept).
  function responseForPage(response, pageUrl) {
    if (!response || response.type !== 'socialSignIn') return response;
    var host = httpsHost(pageUrl);
    var recorded = typeof response.host === 'string' ? response.host.toLowerCase() : null;
    if (response.enabled !== false && host && recorded === host && isProvider(response.provider)) {
      return { type: 'socialSignIn', enabled: true, provider: response.provider, host: recorded };
    }
    return { type: 'socialSignIn', enabled: response.enabled !== false };
  }

  // ---- pending records (the service worker) -------------------------------------------------
  // One record per tab and site (`tabId|origin`), the newest click wins. Records older than the
  // TTL, or malformed, are dropped by `prunePending`.
  function pendingKey(tabId, pageUrl) {
    try {
      return String(tabId) + '|' + new URL(pageUrl).origin;
    } catch (error) {
      return null;
    }
  }

  function prunePending(records, now) {
    var out = {};
    if (!records || typeof records !== 'object') return out;
    Object.keys(records).forEach(function (key) {
      var record = records[key];
      if (!record || typeof record !== 'object' || !isProvider(record.provider)) return;
      if (typeof record.at !== 'number' || now - record.at > PENDING_TTL_MS || record.at > now) return;
      if (!isHttpsUrl(record.pageUrl)) return;
      out[key] = { pageUrl: record.pageUrl, provider: record.provider, at: record.at,
        source: record.source === 'click' ? 'click' : 'frame' };
    });
    return out;
  }

  function addPending(records, tabId, pageUrl, provider, now, source) {
    var next = prunePending(records, now);
    var key = pendingKey(tabId, pageUrl);
    if (key && isProvider(provider) && isHttpsUrl(pageUrl)) {
      next[key] = { pageUrl: pageUrl, provider: provider, at: now, source: source === 'click' ? 'click' : 'frame' };
    }
    return next;
  }

  return {
    PROVIDERS: PROVIDERS,
    PROVIDER_NAMES: PROVIDER_NAMES,
    PENDING_TTL_MS: PENDING_TTL_MS,
    RING_COLOR: RING_COLOR,
    isProvider: isProvider,
    isHttpsUrl: isHttpsUrl,
    httpsHost: httpsHost,
    responseForPage: responseForPage,
    providerFromText: providerFromText,
    providerFromHref: providerFromHref,
    providerFromMarkers: providerFromMarkers,
    providerFromFrameSrc: providerFromFrameSrc,
    pointerWasOver: pointerWasOver,
    FRAME_POINTER_WINDOW_MS: FRAME_POINTER_WINDOW_MS,
    FRAME_POINTER_SLOP_PX: FRAME_POINTER_SLOP_PX,
    detect: detect,
    providerForClickTarget: providerForClickTarget,
    recordMessageForClick: recordMessageForClick,
    recordMessageForFrameFocus: recordMessageForFrameFocus,
    findProviderButtons: findProviderButtons,
    highlightBox: highlightBox,
    buildHighlight: buildHighlight,
    placeHighlight: placeHighlight,
    pendingKey: pendingKey,
    prunePending: prunePending,
    addPending: addPending
  };
})();

if (typeof module !== 'undefined') module.exports = mypassSocialSignIn;
