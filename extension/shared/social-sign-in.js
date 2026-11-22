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
    return provider ? { type: 'recordSocialSignIn', pageUrl: context.pageUrl, provider: provider } : null;
  }

  // A sign-in button that is a cross-origin iframe (Google's rendered button) never shows the
  // click to this page: the window loses focus to the frame instead. With a live user activation
  // (the browser propagates it to the parent; page script cannot forge it) that is the click.
  function recordMessageForFrameFocus(event, context, doc, nav) {
    if (!event || event.isTrusted !== true) return null;
    if (!context || context.enabled === false || !isHttpsUrl(context.pageUrl)) return null;
    var active = doc && doc.activeElement;
    if (!active || String(active.tagName || '').toLowerCase() !== 'iframe') return null;
    var activation = nav && nav.userActivation;
    if (!activation || activation.isActive !== true) return null;
    var provider = providerFromHref(attr(active, 'src'), context.pageUrl);
    return provider ? { type: 'recordSocialSignIn', pageUrl: context.pageUrl, provider: provider } : null;
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
      out[key] = record;
    });
    return out;
  }

  function addPending(records, tabId, pageUrl, provider, now) {
    var next = prunePending(records, now);
    var key = pendingKey(tabId, pageUrl);
    if (key && isProvider(provider) && isHttpsUrl(pageUrl)) {
      next[key] = { pageUrl: pageUrl, provider: provider, at: now };
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
    providerFromText: providerFromText,
    providerFromHref: providerFromHref,
    providerFromMarkers: providerFromMarkers,
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
