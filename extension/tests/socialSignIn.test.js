// Social sign-in detection, trusted-click recording, highlight selection and the pending queue.
const assert = require('node:assert');
const social = require('../shared/social-sign-in.js');

// ---- a tiny DOM: enough for closest(), querySelectorAll() and contains() ---------------------
function matchesSimple(el, simple) {
  const re = /^([a-z0-9-]*)((?:\[[^\]]+\]|\.[A-Za-z0-9_-]+|#[A-Za-z0-9_-]+)*)$/;
  const parts = simple.trim().match(re);
  if (!parts) throw new Error('unsupported selector ' + simple);
  if (parts[1] && el.tagName.toLowerCase() !== parts[1]) return false;
  for (const piece of parts[2].match(/\[[^\]]+\]|\.[A-Za-z0-9_-]+|#[A-Za-z0-9_-]+/g) || []) {
    if (piece[0] === '.') {
      if (!(' ' + (el.getAttribute('class') || '') + ' ').includes(' ' + piece.slice(1) + ' ')) return false;
    } else if (piece[0] === '#') {
      if (el.getAttribute('id') !== piece.slice(1)) return false;
    } else {
      const [name, value] = piece.slice(1, -1).split('=');
      const actual = el.getAttribute(name);
      if (actual === null) return false;
      if (value !== undefined && actual !== value.replace(/^"|"$/g, '')) return false;
    }
  }
  return true;
}

function el(tag, attrs = {}, children = []) {
  const node = {
    nodeType: 1,
    tagName: tag.toUpperCase(),
    attrs,
    children: [],
    parentElement: null,
    getAttribute(name) { return Object.prototype.hasOwnProperty.call(attrs, name) ? attrs[name] : null; },
    get textContent() {
      return this.children.map((child) => (typeof child === 'string' ? child : child.textContent)).join(' ');
    },
    matches(selector) { return selector.split(',').some((simple) => matchesSimple(this, simple)); },
    closest(selector) {
      for (let at = this; at; at = at.parentElement) if (at.matches(selector)) return at;
      return null;
    },
    contains(other) {
      for (let at = other; at; at = at.parentElement) if (at === this) return true;
      return false;
    },
    querySelectorAll(selector) {
      const out = [];
      const walk = (parent) => {
        for (const child of parent.children) {
          if (typeof child === 'string') continue;
          if (child.matches(selector)) out.push(child);
          walk(child);
        }
      };
      walk(this);
      return out;
    }
  };
  for (const child of children) {
    if (typeof child !== 'string') child.parentElement = node;
    node.children.push(child);
  }
  return node;
}

const PAGE = 'https://shop.example/login';
const click = (target, isTrusted = true) => ({ isTrusted, target });

// ---- text in every language ------------------------------------------------------------------
const TEXTS = [
  ['Sign in with Google', 'google'],
  ['  Continue   with Facebook ', 'facebook'],
  ['Log in with GitHub', 'github'],
  ['Sign up with Apple', 'apple'],
  ['Login with Microsoft', 'microsoft'],
  ['Mit Google anmelden', 'google'],
  ['Anmelden mit Apple', 'apple'],
  ['Weiter mit Microsoft', 'microsoft'],
  ['Увійти через Google', 'google'],
  ['Продовжити з Facebook', 'facebook'],
  ['Увійти за допомогою Apple', 'apple'],
  ['Войти через GitHub', 'github'],
  ['Войти с помощью Microsoft', 'microsoft'],
  ['Продолжить с Google', 'google'],
];
for (const [text, provider] of TEXTS) {
  assert.strictEqual(social.providerFromText(text), provider, text);
  const button = el('button', {}, [el('span', {}, [text])]);
  assert.strictEqual(social.providerForClickTarget(button.children[0], PAGE), provider, 'button: ' + text);
}
for (const text of ['Sign in', 'Google', 'We use Google Analytics', 'Continue', 'Войти', 'Sign in with email',
  'x'.repeat(130) + ' sign in with Google']) {
  assert.strictEqual(social.providerFromText(text), null, text);
}
// A bare provider name counts in an aria-label or title (icon-only buttons), never as body text.
assert.strictEqual(social.providerForClickTarget(el('button', { 'aria-label': 'GitHub' }), PAGE), 'github');
assert.strictEqual(social.providerForClickTarget(el('a', { href: '#', title: 'Apple' }), PAGE), 'apple');
assert.strictEqual(social.providerForClickTarget(el('a', { href: '/about' }, ['Apple']), PAGE), null);

// ---- OAuth hosts and site paths --------------------------------------------------------------
const HREFS = [
  ['https://accounts.google.com/o/oauth2/v2/auth?client_id=x', 'google'],
  ['https://www.facebook.com/v19.0/dialog/oauth?client_id=1', 'facebook'],
  ['https://facebook.com/dialog/oauth?x=1', 'facebook'],
  ['https://appleid.apple.com/auth/authorize?client_id=x', 'apple'],
  ['https://login.microsoftonline.com/common/oauth2/v2.0/authorize', 'microsoft'],
  ['https://login.live.com/oauth20_authorize.srf', 'microsoft'],
  ['https://github.com/login/oauth/authorize?client_id=x', 'github'],
  ['/auth/google?next=/', 'google'],
  ['/users/auth/github', 'github'],
  ['/oauth2/microsoft/start', 'microsoft'],
];
for (const [href, provider] of HREFS) {
  assert.strictEqual(social.providerFromHref(href, PAGE), provider, href);
  assert.strictEqual(social.providerForClickTarget(el('a', { href }, [el('img', { alt: '' })]).children[0], PAGE), provider, 'link: ' + href);
}
for (const href of ['https://www.facebook.com/somepage', 'https://github.com/octo', 'http://accounts.google.com.evil.example/',
  'https://evil.example/accounts.google.com', 'javascript:void(0)', 'https://google.com/', '/googleplex']) {
  assert.strictEqual(social.providerFromHref(href, PAGE), null, href);
}
// A submit button names the form's target.
const form = el('form', { action: 'https://github.com/login/oauth/authorize' }, [el('button', { type: 'submit' }, ['Go'])]);
assert.strictEqual(social.providerForClickTarget(form.children[0], PAGE), 'github');

// ---- SDK markers -----------------------------------------------------------------------------
const gsi = el('div', { class: 'g_id_signin', 'data-type': 'standard' }, [el('div', { role: 'button' }, ['Weiter'])]);
assert.strictEqual(social.providerForClickTarget(gsi.children[0], PAGE), 'google', 'g_id_signin container');
assert.strictEqual(social.providerForClickTarget(el('div', { class: 'fb-login-button' }), PAGE), 'facebook');
assert.strictEqual(social.providerForClickTarget(el('div', { id: 'appleid-signin', 'data-color': 'black' }), PAGE), 'apple');
assert.strictEqual(social.providerForClickTarget(el('button', { 'data-provider': 'microsoft' }, ['Work account']), PAGE), 'microsoft');
assert.strictEqual(social.providerForClickTarget(el('button', { 'data-provider': 'GitHub' }), PAGE), 'github');
assert.strictEqual(social.providerForClickTarget(el('div', {}, ['Sign in with Google'])), null, 'not clickable');

// ---- trusted-click-only recording ------------------------------------------------------------
const google = el('button', {}, ['Continue with Google']);
assert.deepStrictEqual(social.recordMessageForClick(click(google), { pageUrl: PAGE }), {
  type: 'recordSocialSignIn', pageUrl: PAGE, provider: 'google'
});
assert.strictEqual(social.recordMessageForClick(click(google, false), { pageUrl: PAGE }), null, 'a script click() never records');
assert.strictEqual(social.recordMessageForClick({ target: google }, { pageUrl: PAGE }), null, 'isTrusted must be exactly true');
assert.strictEqual(social.recordMessageForClick(click(google), { pageUrl: 'http://shop.example/login' }), null, 'https only');
assert.strictEqual(social.recordMessageForClick(click(google), { pageUrl: PAGE, enabled: false }), null, 'setting off');
assert.strictEqual(social.recordMessageForClick(click(el('button', {}, ['Sign in'])), { pageUrl: PAGE }), null);

// Google's rendered button is a cross-origin iframe: the window blur plus a live activation.
const frame = el('iframe', { src: 'https://accounts.google.com/gsi/button?theme=outline' });
const doc = { activeElement: frame };
const active = { userActivation: { isActive: true } };
assert.deepStrictEqual(social.recordMessageForFrameFocus({ isTrusted: true }, { pageUrl: PAGE }, doc, active), {
  type: 'recordSocialSignIn', pageUrl: PAGE, provider: 'google'
});
assert.strictEqual(social.recordMessageForFrameFocus({ isTrusted: true }, { pageUrl: PAGE }, doc, { userActivation: { isActive: false } }), null);
assert.strictEqual(social.recordMessageForFrameFocus({ isTrusted: true }, { pageUrl: PAGE }, doc, {}), null, 'no API: fail closed');
assert.strictEqual(social.recordMessageForFrameFocus({ isTrusted: false }, { pageUrl: PAGE }, doc, active), null);
assert.strictEqual(social.recordMessageForFrameFocus({ isTrusted: true }, { pageUrl: PAGE },
  { activeElement: el('iframe', { src: 'https://ads.example/' }) }, active), null);

// ---- highlight selection ---------------------------------------------------------------------
const textOnly = el('button', {}, ['Sign in with GitHub']);
const linked = el('a', { href: 'https://github.com/login/oauth/authorize?x' }, ['GitHub']);
const hidden = el('a', { href: '/auth/github', 'data-hidden': '1' });
const marker = el('div', { 'data-provider': 'github' }, [el('button', {}, ['Continue with GitHub'])]);
const other = el('button', {}, ['Continue with Google']);
const root = el('body', {}, [textOnly, other, hidden, linked, marker]);
const visible = (node) => node.getAttribute('data-hidden') === null;
assert.deepStrictEqual(
  social.findProviderButtons(root, 'github', visible, PAGE),
  [marker, linked, textOnly],
  'marker first, then link, then text; hidden ones and the button inside a marker are left out'
);
assert.deepStrictEqual(social.findProviderButtons(root, 'apple', visible, PAGE), []);
assert.deepStrictEqual(social.findProviderButtons(root, 'twitter', visible, PAGE), []);
assert.deepStrictEqual(social.findProviderButtons(el('body', {}, [frame]), 'google', null, PAGE), [frame]);

assert.deepStrictEqual(
  social.highlightBox({ left: 10, top: 20, width: 100, height: 40 }, 0, 500),
  { left: 6, top: 516, width: 108, height: 48 }
);

// The overlay is a pointer-events-free host with the ring colour and the badge text.
const made = [];
const fakeDoc = {
  createElement(tag) {
    const node = {
      tag, style: { cssText: '' }, attributes: {}, children: [], textContent: '',
      setAttribute(name, value) { this.attributes[name] = value; },
      appendChild(child) { this.children.push(child); return child; },
      attachShadow(options) { this.shadowMode = options.mode; return this; },
    };
    made.push(node);
    return node;
  }
};
const overlay = social.buildHighlight(fakeDoc, 'Last time you signed in with Google');
assert.match(overlay.host.style.cssText, /pointer-events:none/);
assert.match(overlay.host.style.cssText, /position:absolute/);
assert.strictEqual(overlay.host.shadowMode, 'closed');
assert.match(overlay.ring.style.cssText, /border:2px solid #73b359/);
assert.match(overlay.ring.style.cssText, /border-radius:8px/);
assert.strictEqual(overlay.badge.textContent, 'Last time you signed in with Google');
social.placeHighlight(overlay.host, { left: 10, top: 20, width: 100, height: 40 }, 0, 0);
assert.deepStrictEqual(
  [overlay.host.style.left, overlay.host.style.top, overlay.host.style.width, overlay.host.style.height],
  ['6px', '16px', '108px', '48px']
);

// ---- the pending queue ------------------------------------------------------------------------
const T0 = 1_800_000_000_000;
let queue = social.addPending({}, 7, 'https://shop.example/login?a=1', 'google', T0);
queue = social.addPending(queue, 7, 'https://shop.example/other', 'apple', T0 + 1000);
queue = social.addPending(queue, 8, 'https://shop.example/login', 'github', T0 + 2000);
assert.deepStrictEqual(Object.keys(queue).sort(), ['7|https://shop.example', '8|https://shop.example']);
assert.strictEqual(queue['7|https://shop.example'].provider, 'apple', 'the newest click of a tab and site wins');
assert.deepStrictEqual(social.addPending({}, 7, 'http://shop.example/', 'google', T0), {}, 'https only');
assert.deepStrictEqual(social.addPending({}, 7, 'https://shop.example/', 'myspace', T0), {});
assert.deepStrictEqual(
  Object.keys(social.prunePending(queue, T0 + social.PENDING_TTL_MS + 1500)),
  ['8|https://shop.example'],
  'records older than ten minutes are dropped'
);
assert.deepStrictEqual(social.prunePending({ x: { pageUrl: PAGE, provider: 'google', at: T0 + 5 } }, T0), {}, 'a future time is malformed');
assert.deepStrictEqual(social.prunePending('nope', T0), {});

console.log('socialSignIn.test.js: detection, recording, highlight and queue OK');
