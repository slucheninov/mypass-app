const assert = require('node:assert');

function fakeInput(attrs = {}, extra = {}) {
  return {
    getAttribute: (name) => (Object.prototype.hasOwnProperty.call(attrs, name) ? attrs[name] : null),
    offsetParent: extra.hidden ? null : {},
    disabled: !!extra.disabled,
    readOnly: !!extra.readOnly,
    closest: (sel) => (sel === 'form' ? extra.form || null : sel === 'label' ? extra.label || null : null),
    tagName: 'INPUT',
  };
}

// labels: optional map of id -> label text, used to answer label[for="<id>"] lookups the
// same way scoreUsernameLikeness queries them (against the same scope findFillableFields
// is already using).
function fakeScope(inputs, labels = {}) {
  return {
    querySelectorAll: (selector) => {
      if (selector === 'input[type="password"]') {
        return inputs.filter((i) => i.getAttribute('type') === 'password');
      }
      // findOtpField's selector — unfiltered, exercised on every findFillableFields/
      // findLoginForm call below since both call findOtpField internally.
      if (selector === 'input, textarea') return inputs;
      // Address-field probe used to keep a checkout phone out of login fill.
      if (selector.indexOf('street-address') !== -1) {
        return inputs.filter((i) => /street-address|address-line|address-level|postal-code/.test(i.getAttribute('autocomplete') || ''));
      }
      // the username selector, including phone login fields
      return inputs.filter((i) => {
        const t = i.getAttribute('type');
        const inputMode = (i.getAttribute('inputmode') || '').toLowerCase();
        return t === 'text' || t === 'email' || t === 'tel' || inputMode === 'tel'
          || (i.getAttribute('autocomplete') || '').includes('username');
      });
    },
    querySelector: (selector) => {
      const match = /^label\[for="([^"]+)"\]$/.exec(selector);
      if (match && Object.prototype.hasOwnProperty.call(labels, match[1])) {
        return { textContent: labels[match[1]] };
      }
      return null;
    },
  };
}

const { isFieldVisible, findFillableFields, findLoginForm, resolveApplyFillTargets } = require('../content.js');

// isFieldVisible
assert.strictEqual(isFieldVisible(fakeInput({ type: 'text' })), true, 'plain visible input is fillable');
assert.strictEqual(isFieldVisible(fakeInput({ type: 'text' }, { hidden: true })), false, 'offsetParent null is not fillable');
assert.strictEqual(isFieldVisible(fakeInput({ type: 'text', disabled: '' })), false, 'disabled attribute excluded');
assert.strictEqual(isFieldVisible(fakeInput({ type: 'text' }, { readOnly: true })), false, 'readOnly property excluded');
// position:fixed inputs have offsetParent === null even though they are rendered, so the old
// offsetParent-only test excluded them and they never got an icon. A rendered box (client rects)
// that is not visibility:hidden is what "visible" means; display:none still has no box.
function fixedInput(rects, visibility, attrs) {
  const input = fakeInput({ type: 'text', ...(attrs || {}) }, { hidden: true });
  input.getClientRects = () => new Array(rects).fill({});
  input.ownerDocument = { defaultView: { getComputedStyle: () => ({ visibility: visibility || 'visible' }) } };
  return input;
}
assert.strictEqual(isFieldVisible(fixedInput(1)), true, 'a position:fixed input (offsetParent null, has a box) is fillable');
assert.strictEqual(isFieldVisible(fixedInput(0)), false, 'display:none (no box) stays excluded');
assert.strictEqual(isFieldVisible(fixedInput(1, 'hidden')), false, 'visibility:hidden stays excluded');
assert.strictEqual(isFieldVisible(fixedInput(1, 'visible', { disabled: '' })), false, 'a fixed but disabled input is still excluded');
console.log('inlineIcon.test.js: isFieldVisible OK');

// findFillableFields
const pw = fakeInput({ type: 'password' });
const user = fakeInput({ type: 'text', name: 'username' });
const kinds = (fields) => fields.map((f) => f.kind).sort();

assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([user, pw]))),
  ['password', 'username'],
  'password outside any form still yields both fields'
);

const userOnly = findFillableFields(fakeScope([fakeInput({ type: 'email', name: 'email' })]));
assert.deepStrictEqual(kinds(userOnly), ['username'], 'identifier-first page with no password field still gets a username field');

assert.deepStrictEqual(
  findFillableFields(fakeScope([fakeInput({ type: 'password' }, { hidden: true })])),
  [],
  'hidden password field yields nothing'
);

// MIN_USERNAME_FIELD_SCORE — the floor applies ONLY when the page has no password field.
// A bare `<input type="text">` (score 1) is a search box or a comment field, not a login.
assert.deepStrictEqual(
  findFillableFields(fakeScope([fakeInput({ type: 'text' })])),
  [],
  'a bare text input with no password field on the page gets NO icon'
);
assert.deepStrictEqual(
  findFillableFields(fakeScope([fakeInput({ type: 'text' }), fakeInput({ type: 'text' })])),
  [],
  'several bare text inputs still get nothing — the floor is on the best candidate'
);
// But a password field's presence is itself the evidence, so the floor is waived there:
// dropping this would regress applyFill on a bare `<input type="text">` +
// `<input type="password">` login form.
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([fakeInput({ type: 'text' }), fakeInput({ type: 'password' })]))),
  ['password', 'username'],
  'a bare text input ALONGSIDE a password field still gets a username icon'
);
// The floor lets through the weakest intentional identifier-first signals.
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([fakeInput({ type: 'text', name: 'login' })]))),
  ['username'],
  'name=login scores 3 and clears the floor with no password field present'
);
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([fakeInput({ type: 'text', autocomplete: 'username' })]))),
  ['username'],
  'autocomplete=username clears the floor with no password field present'
);

// Identifier-first fields hinted ONLY by placeholder/aria-label/label (no autocomplete,
// type, name, or id clue) — the coverage gap this feature closes.
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([fakeInput({ type: 'text', placeholder: 'Email' })]))),
  ['username'],
  'placeholder="Email" alone clears the floor with no password field present'
);
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([fakeInput({ type: 'text', 'aria-label': 'Username' })]))),
  ['username'],
  'aria-label="Username" alone clears the floor with no password field present'
);
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([fakeInput({ type: 'text' }, { label: { textContent: 'Email ' } })]))),
  ['username'],
  'wrapped in <label>Email <input></label> clears the floor with no password field present'
);
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([fakeInput({ type: 'text', id: 'signin-id' })], { 'signin-id': 'Email' }))),
  ['username'],
  'label[for="<id>"] association clears the floor with no password field present'
);
// Regression guard the user specifically asked for: a bare search box must still get NO
// icon, even though placeholder/aria-label/label are now consulted.
assert.deepStrictEqual(
  findFillableFields(fakeScope([fakeInput({ type: 'text', placeholder: 'Search' })])),
  [],
  'placeholder="Search" alone still gets NO field — a key icon must not appear in every search box'
);

// Phone-first login, the YASNO shape: one telephone field, no password on this step.
const yasnoPhone = fakeInput({ type: 'tel', placeholder: 'Номер телефону' });
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([yasnoPhone]))),
  ['username'],
  'a phone field with placeholder "Номер телефону" is the login on an identifier-first page'
);
assert.deepStrictEqual(
  findFillableFields(fakeScope([fakeInput({ type: 'tel' })])),
  [],
  'a bare type=tel with no phone hint and no password gets no login field'
);
const billingPhone = fakeInput({ type: 'tel', autocomplete: 'billing tel', placeholder: 'Phone' });
const street = fakeInput({ type: 'text', autocomplete: 'street-address' });
assert.deepStrictEqual(
  findFillableFields(fakeScope([billingPhone, street])),
  [],
  'a billing phone next to a street field is address fill, not a login'
);
const otpTel = fakeInput({ type: 'tel', autocomplete: 'one-time-code' });
assert.deepStrictEqual(
  kinds(findFillableFields(fakeScope([otpTel]))),
  ['otp'],
  'a type=tel one-time code stays an OTP field'
);

// An OTP field also matches the username selector — OTP must win, and the field
// must appear exactly once.
const otp = fakeInput({ type: 'text', autocomplete: 'one-time-code' });
const otpFields = findFillableFields(fakeScope([otp]));
assert.deepStrictEqual(kinds(otpFields), ['otp'], 'one-time-code field is classified otp, not username');
assert.strictEqual(otpFields.length, 1, 'a field is never emitted twice');
console.log('inlineIcon.test.js: findFillableFields OK');

// findLoginForm — reads the global `document`, so stub it per case and restore it after.
const originalDocument = global.document;

// 1. Password field inside a <form> yields {form, passwordInput, usernameInput} with a
// non-null form and both inputs identified correctly.
const formInputs = [];
const formEl = Object.assign(fakeScope(formInputs), { tagName: 'FORM' });
const pwInForm = fakeInput({ type: 'password' }, { form: formEl });
const userInForm = fakeInput({ type: 'text', name: 'username' }, { form: formEl });
formInputs.push(pwInForm, userInForm);
global.document = fakeScope([userInForm, pwInForm]);
const withForm = findLoginForm();
assert.notStrictEqual(withForm, null, 'password field inside a form yields a result');
assert.strictEqual(withForm.form, formEl, 'form is the enclosing <form> element');
assert.strictEqual(withForm.passwordInput, pwInForm, 'passwordInput is the password field');
assert.strictEqual(withForm.usernameInput, userInForm, 'usernameInput is the username field');

// 2. Password field with NO enclosing <form> — deliberate behaviour change from the old
// findLoginForm, which returned null here and silently made applyFill do nothing on such
// pages. The new findLoginForm returns a non-null result with form: null so applyFill can
// still fill the password.
const pwNoForm = fakeInput({ type: 'password' });
global.document = fakeScope([pwNoForm]);
const withoutForm = findLoginForm();
assert.notStrictEqual(withoutForm, null, 'password field with no enclosing form still yields a result');
assert.strictEqual(
  withoutForm.form,
  null,
  'deliberate behaviour change: a password field outside any <form> now returns form: null instead of the old null result, so applyFill can still fill it'
);
assert.strictEqual(withoutForm.passwordInput, pwNoForm, 'passwordInput is set even without a form');

// 3. No password field on the page.
global.document = fakeScope([fakeInput({ type: 'text', name: 'username' })]);
assert.strictEqual(findLoginForm(), null, 'no password field on the page yields null');

// 4. Password plus a username field — usernameInput must be the username field, not the
// password field.
const pw2 = fakeInput({ type: 'password' });
const user2 = fakeInput({ type: 'text', name: 'username' });
global.document = fakeScope([pw2, user2]);
const withUsername = findLoginForm();
assert.strictEqual(withUsername.usernameInput, user2, 'usernameInput is the username field, not the password field');
assert.notStrictEqual(withUsername.usernameInput, withUsername.passwordInput, 'password and username inputs are distinct fields');

global.document = originalDocument;
console.log('inlineIcon.test.js: findLoginForm OK');

// Popup fill on a phone-only step. findLoginForm stays null without a password, and
// that used to make applyFill write nothing after the number had been saved.
{
  const phone = fakeInput({ type: 'tel', placeholder: 'Номер телефону' });
  const filled = resolveApplyFillTargets(
    { type: 'applyFill', username: '+38 066 183 74 99' },
    null,
    fakeScope([phone])
  );
  assert.strictEqual(filled.usernameInput, phone, 'popup fill writes the saved phone into the telephone field');
  assert.strictEqual(filled.passwordInput, null, 'a phone-only step has no password field');
}

const { chooseInlinePanel } = require('../content.js');

const withTotp = { id: '1', title: 'Example', username: 'alice', hasTotp: true };
const noTotp = { id: '2', title: 'Other', username: 'bob', hasTotp: false };
const listResponse = { type: 'credentialList', credentials: [withTotp, noTotp] };

assert.deepStrictEqual(
  chooseInlinePanel(listResponse, 'password'),
  { panel: 'entries', entries: [withTotp, noTotp] },
  'password field lists every match'
);

assert.deepStrictEqual(
  chooseInlinePanel(listResponse, 'otp'),
  { panel: 'entries', entries: [withTotp] },
  'otp field lists only entries that have a TOTP secret'
);

assert.deepStrictEqual(
  chooseInlinePanel({ type: 'credentialList', credentials: [noTotp] }, 'otp'),
  { panel: 'empty', entries: [] },
  'a non-empty list with no TOTP entries renders empty on an otp field'
);

assert.deepStrictEqual(
  chooseInlinePanel({ type: 'credentialList', credentials: [] }, 'password'),
  { panel: 'empty', entries: [] },
  'no matches for this site'
);

assert.strictEqual(chooseInlinePanel({ type: 'locked' }, 'password').panel, 'locked');
assert.strictEqual(chooseInlinePanel({ type: 'pairingRequired' }, 'password').panel, 'pairingRequired');
assert.strictEqual(chooseInlinePanel({ type: 'error' }, 'password').panel, 'unavailable');
assert.strictEqual(chooseInlinePanel(null, 'password').panel, 'unavailable', 'no response is treated as app unavailable');
assert.strictEqual(chooseInlinePanel({ type: 'somethingNew' }, 'password').panel, 'unavailable', 'unknown response never renders entries');
console.log('inlineIcon.test.js: chooseInlinePanel OK');

const { isTrustedActivation, handleEntrySelect, handleIconActivate } = require('../content.js');

assert.strictEqual(isTrustedActivation({ isTrusted: true }), true);
assert.strictEqual(isTrustedActivation({ isTrusted: false }), false, 'synthetic events are rejected');
assert.strictEqual(isTrustedActivation({}), false, 'a missing isTrusted is rejected, not assumed true');
assert.strictEqual(isTrustedActivation(null), false, 'no event is rejected');

// handleEntrySelect: the ONLY message it may send, and only on a trusted event.
const sent = [];
const record = (message) => sent.push(message);

assert.strictEqual(
  handleEntrySelect({ isTrusted: false }, 'entry-1', 'https://example.com', record),
  false,
  'synthetic click does not fill'
);
assert.deepStrictEqual(sent, [], 'synthetic click sends nothing at all');

assert.strictEqual(handleEntrySelect({ isTrusted: true }, 'entry-1', 'https://example.com', record), true);
assert.deepStrictEqual(
  sent,
  [{ type: 'fillCredential', id: 'entry-1', url: 'https://example.com' }],
  'fill message is exactly what popup.js sends — no secret, no extra field'
);

// handleIconActivate
let opened = 0;
assert.strictEqual(handleIconActivate({ isTrusted: false }, () => { opened += 1; }), false);
assert.strictEqual(opened, 0, 'synthetic click does not open the panel');
assert.strictEqual(handleIconActivate({ isTrusted: true }, () => { opened += 1; }), true);
assert.strictEqual(opened, 1);
console.log('inlineIcon.test.js: trusted-event gate OK');

const {
  buildShadowHost, buildIconHost, buildPanelHost, positionHostToField, KEY_ICON_PATH_D,
  ICON_HOST_STYLE, PANEL_HOST_STYLE,
} = require('../content.js');

const SVG_NS_EXPECTED = 'http://www.w3.org/2000/svg';

// Records setProperty's THIRD argument. Without it the entire `!important` page-CSS defence
// could be deleted from content.js and every suite in this file would stay green.
function fakeStyle() {
  const style = {
    priority: {},
    setProperty(prop, value, priority) {
      style[prop] = value;
      style.priority[prop] = priority || '';
    },
  };
  return style;
}

function fakeElement(tag) {
  const el = {
    tagName: tag.toUpperCase(),
    style: fakeStyle(),
    children: [],
    attributes: {},
    shadowMode: null,
    setAttribute(name, value) { this.attributes[name] = value; },
    getAttribute(name) { return Object.prototype.hasOwnProperty.call(this.attributes, name) ? this.attributes[name] : null; },
    appendChild(child) { this.children.push(child); return child; },
    listeners: {},
    addEventListener(type, handler) {
      (this.listeners[type] || (this.listeners[type] = [])).push(handler);
    },
    dispatch(type, event) {
      (this.listeners[type] || []).forEach((handler) => handler(event));
    },
    attachShadow(options) {
      this.shadowMode = options.mode;
      const root = fakeElement('shadow-root');
      this.__shadow = root;
      return root;
    },
  };
  return el;
}

const fakeDocument = {
  createElement: (tag) => fakeElement(tag),
  createElementNS: (ns, tag) => {
    const el = fakeElement(tag);
    el.namespaceURI = ns;
    return el;
  },
};

const built = buildIconHost(fakeDocument);
assert.strictEqual(built.host.shadowMode, 'closed', 'shadow root MUST be closed so the page cannot read the panel');
assert.ok(built.shadow, 'a shadow root is returned');
assert.ok(built.button, 'a clickable button lives inside the shadow root');
assert.ok(/^M/.test(KEY_ICON_PATH_D), 'the glyph is SVG path data');
assert.ok(!/chrome-extension:/.test(KEY_ICON_PATH_D), 'the glyph never references an extension URL');
const glyph = built.button.children[0];
assert.strictEqual(glyph.namespaceURI, SVG_NS_EXPECTED, 'the svg element is namespaced, not an HTML element');
assert.strictEqual(glyph.children[0].getAttribute('d'), KEY_ICON_PATH_D, 'the path carries the key glyph data');
assert.strictEqual(built.button.innerHTML, undefined, 'the icon is built from DOM nodes, never innerHTML — Trusted Types sites throw on innerHTML');

// Every layout property on the host must carry `!important`: a page rule such as
// [data-mypass-inline-icon]{display:none!important} beats a plain inline style.
for (const prop of ['position', 'z-index', 'display', 'pointer-events']) {
  assert.strictEqual(
    built.host.style.priority[prop],
    'important',
    `host ${prop} is set !important so a page !important rule cannot override it`
  );
}

// positioning writes only to the host's own style — the page's input is untouched. The host is
// `position:fixed` and uses VIEWPORT coordinates (getBoundingClientRect numbers): scroll offsets
// are deliberately NON-ZERO here and must have no effect, because adding them is exactly what
// displaced the icon on pages whose <body> has a margin, `position:relative` or a transform.
// The expected values are pinned exactly so the arithmetic cannot drift.
const host = fakeElement('div');
positionHostToField(host, { top: 100, left: 50, width: 200, height: 30 }, { scrollX: 40, scrollY: 200 });
assert.strictEqual(host.style.position, 'fixed');
assert.strictEqual(host.style.top, '102px', 'the 26px button is vertically centered in the 30px field; scroll is ignored');
assert.strictEqual(host.style.left, '218px', 'the button has a 6px trailing inset; scroll is ignored');
assert.strictEqual(host.style.priority.top, 'important', 'top is set !important');
assert.strictEqual(host.style.priority.left, 'important', 'left is set !important');
assert.strictEqual(host.style.priority.position, 'important', 'position is set !important');
assert.strictEqual(built.host.style.position, 'fixed', 'a freshly built host is already fixed');

const unscrolledHost = fakeElement('div');
positionHostToField(unscrolledHost, { top: 100, left: 50, width: 200, height: 30 }, { scrollX: 0, scrollY: 0 });
assert.strictEqual(unscrolledHost.style.top, '102px', 'identical with and without scroll');
assert.strictEqual(unscrolledHost.style.left, '218px', 'identical with and without scroll');

// The one-time containing-block correction (an `html` transform, zoom) shifts the written
// coordinates by the measured offset and nothing else.
const correctedHost = fakeElement('div');
positionHostToField(correctedHost, { top: 100, left: 50, width: 200, height: 30 }, {}, [], null, { dx: -8, dy: -19 });
assert.strictEqual(correctedHost.style.top, '83px');
assert.strictEqual(correctedHost.style.left, '210px');
const { measurePlacementOffset, settlePlacementOffset, mountParentFor, buildSettingsRow } = require('../content.js');
assert.deepStrictEqual(
  measurePlacementOffset({ getBoundingClientRect: () => ({ left: 18, top: 39, width: 26, height: 26 }) }, { left: 10, top: 20 }),
  { dx: -8, dy: -19 },
  'a host that landed 8/19 px away is measured, not guessed'
);
assert.strictEqual(
  measurePlacementOffset({ getBoundingClientRect: () => ({ left: 10, top: 20, width: 26, height: 26 }) }, { left: 10, top: 20 }),
  null,
  'a host that landed correctly needs no correction'
);
assert.strictEqual(
  measurePlacementOffset({ getBoundingClientRect: () => ({ left: 0, top: 0, width: 0, height: 0 }) }, { left: 10, top: 20 }),
  null,
  'a display:none host measures as 0x0 and is never "corrected"'
);

// A containing block that scales and shifts its `position:fixed` host (`html{transform:scale(s)}`
// with some origin offset): `left`/`top` are LOCAL pixels, the measured rect is VIEWPORT pixels.
// One unscaled shift of the viewport gap used to leave d * (1 - s) behind (scale .5: a 50 px gap
// closed by 25 px), for the icon and for the picker alike.
function scaledHost(scaleX, scaleY, originX, originY) {
  const host = fakeElement('div');
  const local = (name) => parseFloat(host.style[name]) || 0;
  // Rendered geometry = origin + local * scale, for the position AND for the size: a local 28 px
  // renders 14 px under scale .5. (An earlier fake scaled only the origin and kept the size, which
  // is how a half-sized icon passed.) offsetWidth/Height are the rounded LOCAL sizes, as in Chrome.
  host.getBoundingClientRect = () => ({
    left: originX + local('left') * scaleX,
    top: originY + local('top') * scaleY,
    width: local('width') * scaleX,
    height: local('height') * scaleY,
  });
  Object.defineProperty(host, 'offsetWidth', { get: () => Math.round(local('width')) });
  Object.defineProperty(host, 'offsetHeight', { get: () => Math.round(local('height')) });
  return host;
}
const iconField = { top: 300, left: 40, width: 300, height: 40 };
// uniform 0.5 and 2, the non-uniform 0.5 x 2, and 1 (also with a translation) as the baseline
[[0.5, 0.5], [2, 2], [0.5, 2], [2, 0.5], [1, 1]].forEach(([scaleX, scaleY]) => {
  const name = scaleX + ' x ' + scaleY;
  // inline icon: same call sequence as placeIconHost
  const iconHost = scaledHost(scaleX, scaleY, 30, 70);
  const first = positionHostToField(iconHost, iconField, {});
  const iconSettled = settlePlacementOffset(iconHost, first.intended, null, (offset) => {
    positionHostToField(iconHost, iconField, {}, [], null, offset);
  });
  const iconRect = iconHost.getBoundingClientRect();
  assert.ok(iconSettled.settled, 'icon at scale ' + name + ' settles');
  assert.ok(Math.abs(iconRect.left - first.intended.left) <= 1 && Math.abs(iconRect.top - first.intended.top) <= 1,
    'icon at scale ' + name + ' lands on its intended viewport position, not d * (1 - scale) away');
  // The RENDERED size is what the user sees: 28 px on screen, whatever the containing block does.
  assert.ok(Math.abs(iconRect.width - first.intended.width) <= 1 && Math.abs(iconRect.height - first.intended.height) <= 1,
    'icon at scale ' + name + ' renders ' + iconRect.width + 'x' + iconRect.height + ', expected ' + first.intended.width + 'x' + first.intended.height);
  assert.ok(Math.abs((iconRect.left + iconRect.width) - (iconField.left + iconField.width - 6)) <= 1,
    'icon at scale ' + name + ' keeps its 6 px gap to the right end of the field');
  assert.ok(Math.abs((iconRect.top + iconRect.height / 2) - (iconField.top + iconField.height / 2)) <= 1,
    'icon at scale ' + name + ' stays centred on the field');
  if (scaleX === 1 && scaleY === 1) {
    assert.ok(!iconSettled.offset.sx && !iconSettled.offset.sy, 'at baseline 1 no scale is recorded, the old coordinates are kept');
  }

  // account picker: same call sequence as positionOpenPanel
  const panelHost = scaledHost(scaleX, scaleY, -25, 55);
  panelHost.style.setProperty('width', String(240 / scaleX) + 'px', 'important');
  panelHost.style.setProperty('height', String(120 / scaleY) + 'px', 'important');
  const placed = require('../shared/placement.js').placeFloating(iconField, { width: 240, height: 120 }, { innerWidth: 1280, innerHeight: 800 });
  const panelSettled = settlePlacementOffset(panelHost, { left: placed.left, top: placed.top }, { dx: 0, dy: 0 }, (offset) => {
    panelHost.style.setProperty('top', String(placed.top / scaleY + offset.dy) + 'px', 'important');
    panelHost.style.setProperty('left', String(placed.left / scaleX + offset.dx) + 'px', 'important');
  });
  const panelRect = panelHost.getBoundingClientRect();
  assert.ok(panelSettled.settled, 'picker at scale ' + name + ' settles');
  assert.ok(Math.abs(panelRect.left - placed.left) <= 1 && Math.abs(panelRect.top - placed.top) <= 1,
    'picker at scale ' + name + ' lands on its intended viewport position');
  assert.ok(Math.abs(panelRect.width - 240) <= 1 && Math.abs(panelRect.height - 120) <= 1, 'picker keeps its rendered size at scale ' + name);
});

// positionHostToField writes LOCAL pixels: size / scale, and position viewport / scale + offset.
const scaledWrite = fakeElement('div');
positionHostToField(scaledWrite, { top: 100, left: 50, width: 200, height: 30 }, {}, [], null, { dx: 3, dy: -4, sx: 0.5, sy: 2 });
assert.strictEqual(scaledWrite.style.width, '52px', 'a 26 px icon (the field is 30 tall) under an x scale of .5 is written as 52 local px');
assert.strictEqual(scaledWrite.style.height, '13px', 'and under a y scale of 2 as 13 local px');
assert.strictEqual(scaledWrite.style.left, String((50 + 200 - 26 - 6) / 0.5 + 3) + 'px');
assert.strictEqual(scaledWrite.style.top, String((100 + 2) / 2 - 4) + 'px');

// The scale is read from the written size: a host whose rendered size already matches needs no
// size fix, one that renders half of it reports the scale.
assert.deepStrictEqual(
  measurePlacementOffset({ style: { width: '28px', height: '28px' }, getBoundingClientRect: () => ({ left: 10, top: 20, width: 14, height: 14 }) },
    { left: 10, top: 20, width: 28, height: 28 }),
  { dx: 0, dy: 0, sx: 0.5, sy: 0.5 }, 'a half-size render is reported as a scale, without a position fix in the same pass');
assert.strictEqual(
  measurePlacementOffset({ style: { width: '28px', height: '28px' }, getBoundingClientRect: () => ({ left: 10, top: 20, width: 28, height: 28 }) },
    { left: 10, top: 20, width: 28, height: 28 }),
  null, 'a correctly sized and placed host needs nothing');

// An unmeasurable host (display:none, 0x0) is accepted as settled without being moved.
const hiddenSettle = settlePlacementOffset({ getBoundingClientRect: () => ({ left: 0, top: 0, width: 0, height: 0 }) },
  { left: 10, top: 20 }, null, () => assert.fail('a 0x0 host must never be shifted'));
assert.ok(hiddenSettle.settled);
// A host that never converges (it ignores the style writes) stops after a bounded number of passes.
let stubbornApplies = 0;
const stubborn = settlePlacementOffset({ getBoundingClientRect: () => ({ left: 90, top: 90, width: 20, height: 20 }) },
  { left: 10, top: 10 }, null, () => { stubbornApplies += 1; });
assert.strictEqual(stubborn.settled, false);
assert.strictEqual(stubbornApplies, 3, 'the settle loop is bounded');

// Mount rule: inside the open modal dialog for a dialog field, otherwise under <html> (never
// <body>, whose transform/margin used to shift the icon).
const rootEl = { tagName: 'HTML' };
const dialogEl = { tagName: 'DIALOG' };
assert.strictEqual(
  mountParentFor({ ownerDocument: { documentElement: rootEl }, closest: (sel) => (sel === 'dialog:modal' ? dialogEl : null) }),
  dialogEl, 'a field inside dialog:modal mounts inside that dialog so it is not under the top layer'
);
assert.strictEqual(
  mountParentFor({ ownerDocument: { documentElement: rootEl }, closest: () => null }),
  rootEl, 'an ordinary field mounts under documentElement'
);
assert.strictEqual(
  mountParentFor({ ownerDocument: { documentElement: rootEl }, closest: () => { throw new SyntaxError(':modal unsupported'); } }),
  rootEl, 'an unsupported :modal selector falls back to documentElement instead of losing the icon'
);

// The settings row is a real <button>; only a trusted click may act, and the row carries no page data.
const settingsClicks = [];
const settingsRow = buildSettingsRow(fakeDocument, (event) => { if (event.isTrusted === true) settingsClicks.push(event); });
assert.strictEqual(settingsRow.tagName, 'BUTTON');
assert.strictEqual(settingsRow.getAttribute('type'), 'button');
assert.strictEqual(settingsRow.textContent, 'MyPass settings');
settingsRow.dispatch('click', { isTrusted: false });
settingsRow.dispatch('click', { isTrusted: true });
assert.strictEqual(settingsClicks.length, 1, 'only the trusted click is acted on');

// The icon button announces a popup list and its state (the accessible name is unchanged).
assert.strictEqual(built.button.getAttribute('aria-haspopup'), 'true');
assert.strictEqual(built.button.getAttribute('aria-expanded'), 'false');
assert.strictEqual(built.button.getAttribute('aria-label'), 'Fill from MyPass');

// Actual overlapping controls move the button; unrelated submit buttons and controls
// on a neighbouring row must not. Input coordinates remain the positioning boundary.
const field = { top: 100, left: 50, width: 300, height: 48 };
const reveal = { top: 108, left: 310, width: 32, height: 32 };
const clear = { top: 108, left: 276, width: 30, height: 32 };
const obstacleHost = fakeElement('div');
positionHostToField(obstacleHost, field, {}, [reveal, clear]);
assert.strictEqual(obstacleHost.style.left, '244px', 'the key clears both visible trailing controls');
positionHostToField(obstacleHost, field, {}, [clear, reveal]);
assert.strictEqual(obstacleHost.style.left, '244px', 'DOM order does not affect adjacent control clearance');
positionHostToField(obstacleHost, field, {}, [
  { top: 160, left: 300, width: 40, height: 32 },
  { top: 108, left: 354, width: 80, height: 32 },
  { top: 108, left: 320, width: 0, height: 32 },
]);
assert.strictEqual(obstacleHost.style.left, '316px', 'controls outside the icon bounds do not add arbitrary inset');
positionHostToField(obstacleHost, { top: 0, left: 0, width: 48, height: 32 }, {});
assert.strictEqual(obstacleHost.style.display, 'none', 'a tiny field keeps its typing space');
positionHostToField(obstacleHost, { top: 0, left: 0, width: 300, height: 16 }, {});
assert.strictEqual(obstacleHost.style.display, 'none', 'a short field never gets an oversized button');
positionHostToField(obstacleHost, field, {}, [{ top: 108, left: 100, width: 242, height: 32 }]);
assert.strictEqual(obstacleHost.style.display, 'none', 'no usable space after reserving site controls hides the icon');
positionHostToField(obstacleHost, field, {});
assert.strictEqual(obstacleHost.style.display, 'block', 'the icon returns when the field has space again');
// A body-mounted host must inherit the effective clipping of nested field wrappers.
// Axis-specific overflow, borders, and scrollbars all affect the usable rectangle.
const { fieldClipRect } = require('../content.js');
const clipDocument = { body: {}, documentElement: {} };
const outerClip = {
  parentElement: clipDocument.body, clientLeft: 2, clientTop: 2,
  clientWidth: 196, clientHeight: 176, offsetWidth: 200, offsetHeight: 180,
  getBoundingClientRect: () => ({ left: 40, top: 30, width: 200, height: 180 }),
  overflow: { overflowX: 'hidden', overflowY: 'visible' },
};
const innerClip = {
  parentElement: outerClip, clientLeft: 1, clientTop: 1,
  clientWidth: 278, clientHeight: 98, offsetWidth: 280, offsetHeight: 100,
  getBoundingClientRect: () => ({ left: 20, top: 70, width: 280, height: 100 }),
  overflow: { overflowX: 'visible', overflowY: 'auto' },
};
const clipViewport = { innerWidth: 800, innerHeight: 600, getComputedStyle: (el) => el.overflow };
const clipBounds = fieldClipRect({ ownerDocument: clipDocument, parentElement: innerClip }, clipViewport);
assert.deepStrictEqual(clipBounds, { left: 42, top: 71, right: 238, bottom: 169 });
const clippedHost = fakeElement('div');
const visibleInClip = { left: 50, top: 100, width: 180, height: 48 };
positionHostToField(clippedHost, visibleInClip, {}, [], clipBounds);
assert.strictEqual(clippedHost.style.display, 'block', 'nested wrappers show a fully contained logo');
positionHostToField(clippedHost, { ...visibleInClip, top: 40 }, {}, [], clipBounds);
assert.strictEqual(clippedHost.style.display, 'none', 'a field scrolled above its container hides the floating logo');
positionHostToField(clippedHost, { ...visibleInClip, top: 55 }, {}, [], clipBounds);
assert.strictEqual(clippedHost.style.display, 'none', 'even a partially clipped logo cannot intercept outside clicks');
positionHostToField(clippedHost, { ...visibleInClip, left: 100 }, {}, [], clipBounds);
assert.strictEqual(clippedHost.style.display, 'none', 'horizontal clipping is respected independently');
positionHostToField(clippedHost, visibleInClip, {}, [], clipBounds);
assert.strictEqual(clippedHost.style.display, 'block', 'scrolling the field back into the container restores the logo');
// A position:fixed field is laid out against the viewport, so an overflow:hidden wrapper that is
// not its containing block does not clip it (reproduced in headless Chrome with a 100 x 40 px
// wrapper: the field stayed fully visible, and the icon was wrongly hidden). A wrapper that IS
// the containing block (transform, filter, contain: ...) does clip it.
{
  const doc = { body: {}, documentElement: {} };
  const wrapper = (style, parent) => ({
    parentElement: parent || doc.body, clientLeft: 0, clientTop: 0, clientWidth: 100, clientHeight: 40,
    offsetWidth: 100, offsetHeight: 40, getBoundingClientRect: () => ({ left: 8, top: 8, width: 100, height: 40 }),
    style: { overflowX: 'hidden', overflowY: 'hidden', ...style },
  });
  const view = { innerWidth: 800, innerHeight: 600, getComputedStyle: (el) => el.style || {} };
  const field = (position, parent) => ({ ownerDocument: doc, parentElement: parent, style: { position } });
  const viewport = { left: 0, top: 0, right: 800, bottom: 600 };
  assert.deepStrictEqual(fieldClipRect(field('fixed', wrapper({})), view), viewport,
    'a fixed field escapes a plain overflow:hidden wrapper');
  assert.deepStrictEqual(fieldClipRect(field('static', wrapper({})), view), { left: 8, top: 8, right: 108, bottom: 48 },
    'a normal field is still clipped by it');
  assert.deepStrictEqual(fieldClipRect(field('fixed', wrapper({ transform: 'matrix(1, 0, 0, 1, 0, 0)' })), view),
    { left: 8, top: 8, right: 108, bottom: 48 }, 'a transformed wrapper is the containing block and clips');
  assert.deepStrictEqual(fieldClipRect(field('fixed', wrapper({ contain: 'paint' })), view),
    { left: 8, top: 8, right: 108, bottom: 48 }, 'contain: paint too');
  assert.deepStrictEqual(fieldClipRect(field('fixed', wrapper({ filter: 'blur(0px)' })), view),
    { left: 8, top: 8, right: 108, bottom: 48 }, 'filter too');
  // an overflow wrapper ABOVE the containing block still clips; one BELOW it (between field and block) does not
  const outerClipping = wrapper({});
  const block = wrapper({ transform: 'translateZ(0)', overflowX: 'visible', overflowY: 'visible' }, outerClipping);
  const openBlock = wrapper({ transform: 'translateZ(0)', overflowX: 'visible', overflowY: 'visible' });
  assert.deepStrictEqual(fieldClipRect(field('fixed', wrapper({}, openBlock)), view), viewport,
    'between field and containing block: bypassed');
  assert.deepStrictEqual(fieldClipRect(field('fixed', wrapper({}, block)), view), { left: 8, top: 8, right: 108, bottom: 48 },
    'above the containing block: still clips');
}
// fieldVisualBox: the outermost tight ancestor box around a slim input (a Material field is a
// 48 px box around a 22 px input; the box carries the visible height). Measured on pz.gov.pl:
// infix 59,305 516x46 / flex 59,305 516x46 / wrapper 42,304 550x48 / form-field 30,296 574x56.
{
  const { fieldVisualBox } = require('../content.js');
  const box = (left, top, width, height, parent) => ({ parentElement: parent || null, getBoundingClientRect: () => ({ left, top, width, height, right: left + width, bottom: top + height }) });
  const formField = box(30, 296, 574, 56);
  const wrapper = box(42, 304, 550, 48, formField);
  const flex = box(59, 305, 516, 46, wrapper);
  const infix = box(59, 305, 516, 46, flex);
  const slimInput = box(59, 317, 516, 22, infix);
  assert.deepStrictEqual(fieldVisualBox(slimInput), { left: 42, top: 304, width: 550, height: 48 },
    'takes the outermost tight box and stops at the one with a 29 px side gap');
  assert.strictEqual(fieldVisualBox(box(59, 317, 516, 22, null)), null, 'no ancestor, no box');
  assert.strictEqual(fieldVisualBox(box(59, 317, 516, 22, box(59, 317, 516, 22))), null, 'a wrapper no taller than the input adds nothing');
  const tallRow = box(0, 200, 900, 400);
  assert.strictEqual(fieldVisualBox(box(59, 317, 516, 22, tallRow)), null, 'a big layout container is not a field box');
  const collapsed = box(0, 0, 0, 0, wrapper);
  assert.deepStrictEqual(fieldVisualBox(box(59, 317, 516, 22, collapsed)), { left: 42, top: 304, width: 550, height: 48 },
    'a display:contents style zero-size ancestor is skipped, not a stopper');
  // pz.gov.pl PASSWORD field, measured live: the site's own CSS (`width: calc(100% - 3.375rem)`)
  // makes the input 54 px narrower than its container to reserve room for a 24 px reveal button
  // and a 14 px clear button, so the container is wider than the input on the end side only.
  // infix/flex 59,448 516x46 / wrapper 42,447 550x48 / form-field 30,439 574x56; input 59,460 462x22.
  {
    const { findFieldBox } = require('../content.js');
    const formField = box(30, 439, 574, 56);
    const wrapper = box(42, 447, 550, 48, formField);
    const flex = box(59, 448, 516, 46, wrapper);
    const infix = box(59, 448, 516, 46, flex);
    const password = box(59, 460, 462, 22, infix);
    assert.deepStrictEqual(fieldVisualBox(password), { left: 42, top: 447, width: 550, height: 48 },
      'room reserved at the end for the site\'s own controls does not stop the box');
    assert.strictEqual(findFieldBox(password).element, wrapper, 'the chosen box is the element, so controls can be read from it');
    // The extra end room is bounded: a field next to a wide sibling area is not a field box.
    assert.strictEqual(fieldVisualBox(box(59, 460, 300, 22, box(59, 448, 500, 46))), null, '200 px of free end room is not reserved control space');
    // ...and only the end side gets it: a gap before the input, or above/below it, still stops the walk.
    assert.strictEqual(fieldVisualBox(box(100, 460, 462, 22, box(59, 448, 503, 46))), null, 'a 41 px gap before the input stops the walk');
    assert.strictEqual(fieldVisualBox(box(59, 460, 462, 22, box(59, 420, 516, 100))), null, 'a tall container above and below stops the walk');
  }
  // A field that is already tall enough never uses the box, so the ancestors are not measured at all.
  {
    let reads = 0;
    const spyParent = { parentElement: null, getBoundingClientRect() { reads += 1; return { left: 0, top: 0, width: 600, height: 60, right: 600, bottom: 60 }; } };
    assert.strictEqual(fieldVisualBox(box(10, 10, 500, 36, spyParent)), null, 'a 36 px input is sized from itself');
    assert.strictEqual(reads, 0, 'no ancestor geometry is read for a tall input');
  }
  // the box reaches the placed host: a 22 px input gets a visible, 28 px icon
  const slimHost = fakeElement('div');
  positionHostToField(slimHost, { top: 317, left: 59, width: 516, height: 22 }, {}, [],
    { left: 42, top: 304, right: 592, bottom: 352 }, null, { left: 42, top: 304, width: 550, height: 48 });
  assert.strictEqual(slimHost.style.display, 'block', 'the slim input gets its icon');
  assert.deepStrictEqual([slimHost.style.width, slimHost.style.top, slimHost.style.left], ['28px', '314px', '558px']);
}
// Site controls are read from the chosen field box, not just from the input's own container: a
// Material suffix button is a sibling of the container that holds the input.
{
  const { fieldControlRects } = require('../content.js');
  const rectOf = (left, top, width, height) => ({ left, top, width, height, right: left + width, bottom: top + height });
  const control = (left, top, width, height, display) => ({ display: display || 'block', getBoundingClientRect: () => rectOf(left, top, width, height) });
  const view = { getComputedStyle: (el) => ({ visibility: 'visible', display: el.display }) };
  const scope = (controls) => ({ querySelectorAll: () => controls });
  const suffix = control(536, 444, 40, 40);
  const reveal = control(527, 460, 24, 25);
  const hiddenClear = control(0, 0, 0, 0, 'none');
  const input = { parentElement: scope([reveal, hiddenClear]), ownerDocument: { defaultView: view } };
  assert.deepStrictEqual(fieldControlRects(input), [rectOf(527, 460, 24, 25)], 'default scope is the input\'s container; display:none is skipped');
  assert.deepStrictEqual(fieldControlRects(input, scope([reveal, suffix, hiddenClear])), [rectOf(527, 460, 24, 25), rectOf(536, 444, 40, 40)],
    'a given field box is scanned instead, so a suffix outside the container is an obstacle');
}
console.log('inlineIcon.test.js: icon host OK');

// buildShadowHost is the bare scaffolding shared by the icon and the panel — the panel
// is a list, not a button, and must never end up with the icon's glyph inside it.
const bareHost = buildShadowHost(fakeDocument, 'data-mypass-inline-panel', PANEL_HOST_STYLE);
assert.strictEqual(bareHost.host.shadowMode, 'closed', 'bare host shadow root MUST be closed too');
assert.strictEqual(bareHost.shadow.children.length, 1, 'bare shadow root contains only the style element');
assert.strictEqual(bareHost.shadow.children[0].tagName, 'STYLE', 'the sole child is the style element');
assert.ok(
  !bareHost.shadow.children.some((child) => child.tagName === 'BUTTON'),
  'no button anywhere in the bare shadow root'
);
assert.strictEqual(bareHost.host.getAttribute('data-mypass-inline-panel'), '', 'the bare host carries whichever marker it was given');
assert.strictEqual(bareHost.host.getAttribute('data-mypass-inline-icon'), null, 'the bare host does not also carry the icon marker');

const iconHostBuilt = buildIconHost(fakeDocument);
assert.strictEqual(iconHostBuilt.host.getAttribute('data-mypass-inline-icon'), '', 'the icon host carries the icon marker');
assert.strictEqual(iconHostBuilt.shadow.children.length, 2, 'icon shadow root still contains both the style and the button');
assert.ok(iconHostBuilt.shadow.children.some((child) => child.tagName === 'STYLE'), 'icon shadow root still contains the style element');
assert.ok(iconHostBuilt.shadow.children.includes(iconHostBuilt.button), 'icon shadow root still contains the button');
assert.strictEqual(iconHostBuilt.button.children[0].namespaceURI, SVG_NS_EXPECTED, 'the button still carries the namespaced svg glyph');

// Each surface installs ITS OWN stylesheet. Handing the panel the icon's sheet rendered
// every panel row as a 20x20 transparent box with overflowing text. buildPanelHost is what
// the browser-only togglePanel actually calls, so asserting on it pins the real pairing.
const panelHostBuilt = buildPanelHost(fakeDocument);
assert.strictEqual(panelHostBuilt.host.getAttribute('data-mypass-inline-panel'), '', 'the panel host carries the panel marker');
assert.strictEqual(panelHostBuilt.host.getAttribute('data-mypass-inline-icon'), null, 'the panel host does not carry the icon marker');
assert.strictEqual(panelHostBuilt.host.shadowMode, 'closed', 'the panel shadow root is closed');
assert.strictEqual(
  panelHostBuilt.shadow.children[0].textContent,
  PANEL_HOST_STYLE,
  'the panel host installs the PANEL stylesheet, not the icon one'
);
assert.strictEqual(
  iconHostBuilt.shadow.children.find((child) => child.tagName === 'STYLE').textContent,
  ICON_HOST_STYLE,
  'the icon host installs the ICON stylesheet'
);
assert.strictEqual(bareHost.shadow.children[0].textContent, PANEL_HOST_STYLE, 'buildShadowHost installs whichever sheet it is given');
assert.notStrictEqual(PANEL_HOST_STYLE, ICON_HOST_STYLE, 'the two stylesheets are genuinely different');

// What the panel stylesheet must actually provide — it is a list surface, not a 20x20 glyph.
assert.ok(/:host\{all:initial\}/.test(PANEL_HOST_STYLE), 'the panel keeps :host{all:initial} isolation');
assert.ok(/\.panel\{[^}]*background:#ffffff/.test(PANEL_HOST_STYLE), 'the panel has an opaque background');
assert.ok(/\.panel\{[^}]*border:1px solid/.test(PANEL_HOST_STYLE), 'the panel has a border');
assert.ok(/\.panel\{[^}]*border-radius:/.test(PANEL_HOST_STYLE), 'the panel has a border radius');
assert.ok(/\.panel\{[^}]*min-width:/.test(PANEL_HOST_STYLE), 'the panel has a min width');
assert.ok(/\.panel\{[^}]*max-width:/.test(PANEL_HOST_STYLE), 'the panel has a max width');
assert.ok(/\.panel\{[^}]*padding:/.test(PANEL_HOST_STYLE), 'the panel has padding');
assert.ok(/\.row\{[^}]*width:100%/.test(PANEL_HOST_STYLE), 'panel rows are full width');
assert.ok(/\.row\{[^}]*text-align:left/.test(PANEL_HOST_STYLE), 'panel rows are left aligned');
assert.ok(/\.row:hover/.test(PANEL_HOST_STYLE), 'panel rows have a hover state');
assert.ok(!/\.row\{[^}]*height:20px/.test(PANEL_HOST_STYLE), 'panel rows are never the icon-sized 20x20 box');

// Dark theme: string-level checks only — this node suite never renders CSS, so this proves
// the dark rules are present in the stylesheet text, not that they actually render correctly
// in a browser. That still has to be eyeballed manually (see RUN.md).
assert.ok(
  /@media \(prefers-color-scheme: dark\)\{[^]*\}/.test(PANEL_HOST_STYLE),
  'the panel stylesheet has a prefers-color-scheme: dark block'
);
const darkBlockMatch = /@media \(prefers-color-scheme: dark\)\{([^]*)\}$/.exec(PANEL_HOST_STYLE.trim());
assert.ok(darkBlockMatch, 'the dark media block is the trailing block in the stylesheet');
assert.ok(
  /\.panel\{[^}]*background:#(?!ffffff)[0-9a-f]{6}/.test(darkBlockMatch[1]),
  'the dark block redefines .panel background to something other than the light #ffffff'
);
console.log('inlineIcon.test.js: shadow host separation OK');

const { buildPanelContent } = require('../content.js');
const { letterFromTitle, colorForTitle } = require('../shared/entry-visuals.js');

const selectCalls = [];
const entriesPanel = buildPanelContent(
  fakeDocument,
  { panel: 'entries', entries: [withTotp, noTotp] },
  { onSelect: (event, entryId) => selectCalls.push({ event, entryId }), onUnlock: () => {} }
);
assert.strictEqual(entriesPanel.rows.length, 2, 'one row per entry');
// The rows are styled by PANEL_HOST_STYLE's class rules, so they must carry those classes.
assert.strictEqual(entriesPanel.root.getAttribute('class'), 'panel', 'the panel root carries the .panel class');
assert.strictEqual(entriesPanel.rows[0].getAttribute('class'), 'row', 'each entry row carries the .row class');
assert.strictEqual(entriesPanel.rows[1].getAttribute('class'), 'row', 'every entry row carries it, not just the first');

// The row is no longer a single textContent string — title and username must be findable in
// their own elements, matching the popup's letter-avatar + title/username layout, and the
// old ' — ' concatenation must be gone everywhere in the row.
function findDescendants(el, predicate, found = []) {
  for (const child of el.children || []) {
    if (predicate(child)) found.push(child);
    findDescendants(child, predicate, found);
  }
  return found;
}

for (const row of entriesPanel.rows) {
  const allDescendants = findDescendants(row, () => true);
  for (const node of [row, ...allDescendants]) {
    if (typeof node.textContent === 'string') {
      assert.ok(!node.textContent.includes(' — '), 'no element text carries the old title/username concatenation');
    }
  }
}

// Row 0 (withTotp: title 'Example', username 'alice') — avatar, title, username, ring.
const row0 = entriesPanel.rows[0];
const avatar0 = findDescendants(row0, (el) => el.attributes && el.attributes.class === 'avatar')[0];
assert.ok(avatar0, 'row has an avatar element');
assert.strictEqual(avatar0.style.backgroundColor, colorForTitle('Example'), 'avatar background colour is colorForTitle(title)');
const avatarLetter0 = avatar0.children[0];
assert.ok(avatarLetter0, 'avatar contains a letter element');
assert.strictEqual(avatarLetter0.textContent, letterFromTitle('Example'), 'avatar letter is letterFromTitle(title)');

const title0 = findDescendants(row0, (el) => el.attributes && el.attributes.class === 'entry-title')[0];
const username0 = findDescendants(row0, (el) => el.attributes && el.attributes.class === 'entry-username')[0];
assert.ok(title0, 'row has a title element');
assert.strictEqual(title0.textContent, 'Example', 'title element carries only the title');
assert.ok(username0, 'row has a username element when the entry has a username');
assert.strictEqual(username0.textContent, 'alice', 'username element carries only the username');
assert.notStrictEqual(title0, username0, 'title and username are separate elements');

const ring0 = findDescendants(row0, (el) => el.attributes && el.attributes.class === 'ring')[0];
assert.ok(ring0, 'an entry with hasTotp: true gets a ring element');
// Indicator-only: pin this so a later change cannot quietly make the ring interactive or
// reveal the code. The ring must never call getTotp and must never show/copy the code.
const ringDescendants = [ring0, ...findDescendants(ring0, () => true)];
for (const node of ringDescendants) {
  assert.ok(
    !(typeof node.textContent === 'string' && /\d/.test(node.textContent)),
    'no text node inside the ring carries digits — it must never render the code'
  );
  assert.ok(
    !(node.listeners && node.listeners.click && node.listeners.click.length > 0),
    'no click listener is registered on the ring or anything inside it — it is an indicator only'
  );
  // textContent alone would miss a code leaked into an attribute instead of a text node
  // (e.g. a stray title/aria-label/data-* meant for debugging). Check every attribute on
  // every ring descendant for a run of 6 digits — the shape of a TOTP code.
  if (node.attributes) {
    for (const [attrName, attrValue] of Object.entries(node.attributes)) {
      const isCheckedAttr = attrName === 'title' || attrName === 'aria-label' || attrName.startsWith('data-');
      if (!isCheckedAttr) continue;
      assert.ok(
        !/\d{6}/.test(String(attrValue)),
        `attribute ${attrName} inside the ring must not carry a 6-digit run (leaked code)`
      );
    }
  }
}

// panelRingFraction — the wall-clock-only source for the ring's fill. Pin the boundary
// arithmetic so a future refactor that rewires the ring to a fetched value (instead of
// Date.now()) fails here rather than shipping silently: this is the panel's entire trust
// model for "the ring never carries the secret". Timestamps are passed explicitly — this
// test never calls Date.now().
const { panelRingFraction } = require('../content.js');
assert.strictEqual(panelRingFraction(0), 1, 'exactly on a 30s boundary the ring is full (fraction 1)');
assert.strictEqual(panelRingFraction(15000), 0.5, '15s past the boundary the ring is half full');
assert.ok(
  panelRingFraction(29999) > 0 && panelRingFraction(29999) < 0.001,
  'just before the next boundary the fraction approaches 0'
);
console.log('inlineIcon.test.js: panelRingFraction OK');

// Row 1 (noTotp: hasTotp: false) — no ring at all.
const row1 = entriesPanel.rows[1];
const ring1 = findDescendants(row1, (el) => el.attributes && el.attributes.class === 'ring')[0];
assert.strictEqual(ring1, undefined, 'an entry with hasTotp: false gets no ring element');
console.log('inlineIcon.test.js: panel row visuals OK');

const firstClickEvent = { type: 'click', target: entriesPanel.rows[0] };
entriesPanel.rows[0].dispatch('click', firstClickEvent);
assert.strictEqual(selectCalls.length, 1, 'clicking the first row invokes onSelect');
assert.strictEqual(selectCalls[0].event, firstClickEvent, 'onSelect receives the click event');
assert.strictEqual(selectCalls[0].entryId, '1', 'the first row reports the first entry id');

const secondClickEvent = { type: 'click', target: entriesPanel.rows[1] };
entriesPanel.rows[1].dispatch('click', secondClickEvent);
assert.strictEqual(selectCalls.length, 2, 'clicking the second row invokes onSelect again');
assert.strictEqual(selectCalls[1].entryId, '2', 'the second row reports the second entry id, not the first (no closure-capture bug)');

const emptyPanel = buildPanelContent(fakeDocument, { panel: 'empty', entries: [] }, { onSelect: () => {}, onUnlock: () => {} });
assert.strictEqual(emptyPanel.rows.length, 0, 'the empty state has no selectable rows');
assert.strictEqual(emptyPanel.root.children[0].textContent, 'No entries for this site', 'the empty state renders its message');
assert.strictEqual(emptyPanel.root.children[0].getAttribute('class'), 'message', 'the message carries the .message class');

const unlockCalls = [];
const lockedPanel = buildPanelContent(fakeDocument, { panel: 'locked', entries: [] }, { onSelect: () => {}, onUnlock: (event) => unlockCalls.push(event) });
assert.strictEqual(lockedPanel.rows.length, 0, 'the locked state offers no entries');
assert.ok(lockedPanel.unlockButton, 'the locked state offers an unlock button');
assert.strictEqual(lockedPanel.unlockButton.getAttribute('class'), 'unlock', 'the unlock button carries the .unlock class');
assert.strictEqual(lockedPanel.root.children[0].textContent, 'MyPass is locked', 'the locked state renders its message');

const unlockClickEvent = { type: 'click' };
lockedPanel.unlockButton.dispatch('click', unlockClickEvent);
assert.strictEqual(unlockCalls.length, 1, 'clicking the unlock button invokes onUnlock');
assert.strictEqual(unlockCalls[0], unlockClickEvent, 'onUnlock receives the click event');

const pairingPanel = buildPanelContent(fakeDocument, { panel: 'pairingRequired', entries: [] }, { onSelect: () => {}, onUnlock: () => {} });
assert.strictEqual(pairingPanel.unlockButton, null, 'the pairing state offers no unlock button');
assert.strictEqual(pairingPanel.root.children[0].textContent, 'Pair MyPass to use it here', 'the pairing state renders its message');

const unavailablePanel = buildPanelContent(fakeDocument, { panel: 'unavailable', entries: [] }, { onSelect: () => {}, onUnlock: () => {} });
assert.strictEqual(unavailablePanel.unlockButton, null, 'the unavailable state offers no unlock button');
assert.strictEqual(unavailablePanel.root.children[0].textContent, 'MyPass is not running', 'the unavailable state renders its message');

const unknownPanel = buildPanelContent(fakeDocument, { panel: 'someUnknownState', entries: [] }, { onSelect: () => {}, onUnlock: () => {} });
assert.strictEqual(unknownPanel.root.children[0].textContent, 'MyPass is not running', 'an unrecognized panel value falls back to the unavailable message');
console.log('inlineIcon.test.js: panel content OK');

// resolveFillTargets — the clicked-field-only fill decision. Clicking an icon must fill ONLY
// the field whose icon was clicked; the popup's own fill path never calls this function at
// all (it has no clicked target and falls back to findLoginForm), so that path is unaffected.
const { resolveFillTargets } = require('../content.js');

function fakeFillInput(name) {
  return { tagName: 'INPUT', name };
}

// 1. Clicking a password field fills only the password.
const pwOnly = fakeFillInput('password-field');
const pwTargets = resolveFillTargets({ input: pwOnly, kind: 'password' });
assert.strictEqual(pwTargets.passwordInput, pwOnly, 'a password-initiated fill targets the clicked password input');
assert.strictEqual(pwTargets.usernameInput, null, 'a password-initiated fill must not also target a username');
assert.strictEqual(pwTargets.otpInput, null, 'a password-initiated fill must not also target an otp field');

// 2. Clicking a username field fills only the username.
const usernameOnly = fakeFillInput('username-field');
const userTargets = resolveFillTargets({ input: usernameOnly, kind: 'username' });
assert.strictEqual(userTargets.usernameInput, usernameOnly, 'a username-initiated fill targets the clicked username input');
assert.strictEqual(userTargets.passwordInput, null, 'a username-initiated fill must not also target a password');
assert.strictEqual(userTargets.otpInput, null, 'a username-initiated fill must not also target an otp field');

// 3. Clicking an otp field fills only the otp code.
const otpOnly = fakeFillInput('otp-field');
const otpTargets = resolveFillTargets({ input: otpOnly, kind: 'otp' });
assert.strictEqual(otpTargets.otpInput, otpOnly, 'an otp-initiated fill targets the clicked otp input');
assert.strictEqual(otpTargets.usernameInput, null, 'an otp-initiated fill must not also target a username');
assert.strictEqual(otpTargets.passwordInput, null, 'an otp-initiated fill must not also target a password');

// 4. THE MANUAL-WALK DEFECT, pinned under the new behaviour: a page with two separate login
// groups. Clicking the SECOND group's password must resolve to that exact input, by identity,
// and must leave the first group's inputs completely untouched. This is the same defect
// confirmed live in Chrome on inline-icon-noform.html (four password fields, wrong one filled
// every time) — a future refactor that reintroduces any cross-group lookup must fail this.
const group1Pw = fakeFillInput('group1-password');
const group1User = fakeFillInput('group1-username');
const group2Pw = fakeFillInput('group2-password');
const group2User = fakeFillInput('group2-username');

const secondGroupTargets = resolveFillTargets({ input: group2Pw, kind: 'password' });
assert.strictEqual(
  secondGroupTargets.passwordInput, group2Pw,
  'manual-walk defect (multi-form fill-targeting bug): clicking the second group\'s password must fill that exact input'
);
assert.strictEqual(
  secondGroupTargets.usernameInput, null,
  'manual-walk defect (multi-form fill-targeting bug): clicking a password must never also fill a username, in any group'
);
assert.notStrictEqual(
  secondGroupTargets.passwordInput, group1Pw,
  'manual-walk defect (multi-form fill-targeting bug): must never fill the first group\'s password field'
);
assert.notStrictEqual(
  secondGroupTargets.passwordInput, group1User,
  'manual-walk defect (multi-form fill-targeting bug): must never touch the first group\'s username field either'
);
// Resolving the first group's own password proves the behaviour is symmetric: it too
// resolves to itself and only itself, not just group 2's field from above.
const firstGroupTargets = resolveFillTargets({ input: group1Pw, kind: 'password' });
assert.strictEqual(
  firstGroupTargets.passwordInput, group1Pw,
  'the first group\'s own password-initiated fill targets that exact input'
);
assert.strictEqual(
  firstGroupTargets.usernameInput, null,
  'the first group\'s password-initiated fill must not also target a username'
);
assert.strictEqual(
  firstGroupTargets.otpInput, null,
  'the first group\'s password-initiated fill must not also target an otp field'
);

// 5. A null/malformed target yields all three null.
assert.deepStrictEqual(
  resolveFillTargets(null),
  { usernameInput: null, passwordInput: null, otpInput: null },
  'a null target resolves to no fill targets'
);
assert.deepStrictEqual(
  resolveFillTargets({}),
  { usernameInput: null, passwordInput: null, otpInput: null },
  'a malformed target with no input resolves to no fill targets'
);
assert.deepStrictEqual(
  resolveFillTargets({ kind: 'password' }),
  { usernameInput: null, passwordInput: null, otpInput: null },
  'a target missing its input resolves to no fill targets even with a kind present'
);

console.log('inlineIcon.test.js: fill targeting OK');

// isFillTargetUsable — the consume-once/TTL lifecycle. This wiring (setPendingFillTarget/
// takePendingFillTarget) lives inside the browser-only message listener and is otherwise
// untestable from node, which is exactly where the stale-target bug hid: a fillCredential
// request that never gets an applyFill response (locked vault, native error, hostname
// mismatch) used to leave the target set forever for a later, unrelated fill — including a
// popup-initiated one with no clicked field — to inherit.
const { isFillTargetUsable } = require('../content.js');

const TTL_MS = 5000;
const connectedInput = { isConnected: true };
const disconnectedInput = { isConnected: false };

assert.strictEqual(isFillTargetUsable(null, 1000, TTL_MS), false, 'no target at all is unusable');
assert.strictEqual(
  isFillTargetUsable({ input: disconnectedInput, kind: 'password', at: 1000 }, 1000, TTL_MS),
  false,
  'a target whose input left the document is unusable, even with no time elapsed'
);
// These two pin the boundary exactly: (now - at) === ttlMs must be usable and
// (now - at) === ttlMs + 1 must not, which distinguishes a `<=` comparison from a `<` one —
// swapping the operator flips the first assertion below without touching the second.
assert.strictEqual(
  isFillTargetUsable({ input: connectedInput, kind: 'password', at: 1000 }, 1000 + TTL_MS, TTL_MS),
  true,
  'a target at exactly the TTL boundary (now - at === ttlMs) is still usable'
);
assert.strictEqual(
  isFillTargetUsable({ input: connectedInput, kind: 'password', at: 1000 }, 1000 + TTL_MS + 1, TTL_MS),
  false,
  'a target one millisecond past the TTL boundary is unusable — an unanswered fillCredential must not linger forever'
);
assert.strictEqual(
  isFillTargetUsable({ input: connectedInput, kind: 'password', at: 1000 }, 1200, TTL_MS),
  true,
  'a fresh, connected target within the TTL is usable'
);
console.log('inlineIcon.test.js: fill target lifecycle OK');

const { isFeatureEnabledInFrame, FRAME_FEATURE_POLICY } = require('../content.js');

// In the top frame everything runs.
for (const feature of Object.keys(FRAME_FEATURE_POLICY)) {
  assert.strictEqual(isFeatureEnabledInFrame(feature, true), true, feature + ' runs in the top frame');
}

// In a subframe only the inline icon does.
assert.strictEqual(isFeatureEnabledInFrame('inlineIcon', false), true, 'the icon is the whole point — it runs in subframes');
assert.strictEqual(isFeatureEnabledInFrame('saveBanner', false), false, 'a password save banner must never render inside a subframe');
assert.strictEqual(isFeatureEnabledInFrame('totpBanner', false), false, 'a 2FA save banner must never render inside a subframe');
assert.strictEqual(isFeatureEnabledInFrame('totpScan', false), false, 'the TOTP candidate scan is top-frame work');
assert.strictEqual(
  isFeatureEnabledInFrame('totpCandidateQuery', false),
  false,
  'only one frame may answer the popup, or responses collide'
);
assert.strictEqual(
  isFeatureEnabledInFrame('pageIdentifierQuery', false),
  false,
  'only the top frame reports the page identifier'
);

// Fail closed: a feature nobody gave a policy to must NOT start running in ad frames.
assert.strictEqual(
  isFeatureEnabledInFrame('somethingAddedLater', false),
  false,
  'an unknown feature fails closed in subframes rather than silently running everywhere'
);
assert.strictEqual(isFeatureEnabledInFrame('somethingAddedLater', true), false, 'an unknown feature is not enabled anywhere');
console.log('inlineIcon.test.js: frame policy OK');

const { buildMutationSweep } = require('../content.js');

// In a subframe, icons still get attached but the banner scan is skipped entirely.
{
  let attachCalls = 0;
  let scanCalls = 0;
  const sweep = buildMutationSweep({
    onAttachIcons: () => { attachCalls += 1; },
    onScanForBanner: () => { scanCalls += 1; },
    isTopFrame: false,
  });
  sweep();
  assert.strictEqual(attachCalls, 1, 'attachIcons runs once in a subframe — the icon is the whole point');
  assert.strictEqual(scanCalls, 0, 'the banner scan must not run in a subframe');
}

// In the top frame, both halves run.
{
  let attachCalls = 0;
  let scanCalls = 0;
  const sweep = buildMutationSweep({
    onAttachIcons: () => { attachCalls += 1; },
    onScanForBanner: () => { scanCalls += 1; },
    isTopFrame: true,
  });
  sweep();
  assert.strictEqual(attachCalls, 1, 'attachIcons runs once in the top frame');
  assert.strictEqual(scanCalls, 1, 'the banner scan runs once in the top frame');
}

// Ordering: icons are attached before the banner scan runs.
{
  const order = [];
  const sweep = buildMutationSweep({
    onAttachIcons: () => { order.push('attach'); },
    onScanForBanner: () => { order.push('scan'); },
    isTopFrame: true,
  });
  sweep();
  assert.deepStrictEqual(order, ['attach', 'scan'], 'attachIcons must run before the banner scan');
}

// A throwing attachIcons must not take the banner scan down with it — the try/catch is
// a deliberate earlier fix in this feature's history, not an accident to regress.
{
  let scanCalls = 0;
  const sweep = buildMutationSweep({
    onAttachIcons: () => { throw new Error('attach failed'); },
    onScanForBanner: () => { scanCalls += 1; },
    isTopFrame: true,
  });
  sweep();
  assert.strictEqual(scanCalls, 1, 'a throwing attachIcons must not prevent the banner scan from running');
}
console.log('inlineIcon.test.js: mutation sweep framing OK');

const { observeTotpCandidates } = require('../content.js');

// A frame is not always an HTML document. SVG and XML documents are ordinary iframe
// targets and have no document.body, so MutationObserver.observe(undefined) throws an
// uncaught TypeError there — the exact "errors on a page with several ad iframes"
// failure this feature must not produce, and one that only appears now that content
// scripts run with all_frames: true.
{
  const savedDocument = global.document;
  const savedObserver = global.MutationObserver;
  let observeCalls = 0;

  // Mirrors the browser: observe() rejects a non-Node target with a TypeError rather
  // than quietly ignoring it. Without this the guard could "pass" for the wrong reason.
  global.MutationObserver = class {
    constructor(callback) { this.callback = callback; }
    observe(target) {
      observeCalls += 1;
      if (!target) throw new TypeError("Failed to execute 'observe' on 'MutationObserver': parameter 1 is not of type 'Node'.");
    }
    disconnect() {}
  };

  // An SVG/XML document: everything a document normally has, except a body.
  global.document = { querySelectorAll: () => [], querySelector: () => null };
  const state = { totpBannerDismissed: null };

  let result;
  assert.doesNotThrow(
    () => { result = observeTotpCandidates(state, () => {}, false); },
    'observing a document with no body must not throw — SVG/XML frames have no document.body'
  );
  assert.strictEqual(result, null, 'a bodyless document yields no observer for callers to hold');
  assert.strictEqual(observeCalls, 0, 'observe() must not even be attempted without a body');

  // Control: with a body present the observer is still created and still attached to it,
  // so the guard cannot pass by disabling the feature everywhere.
  const body = { nodeType: 1 };
  global.document = { body, querySelectorAll: () => [], querySelector: () => null };
  const observer = observeTotpCandidates(state, () => {}, false);
  assert.ok(observer, 'a normal HTML document must still get its observer');
  assert.strictEqual(observeCalls, 1, 'observe() runs exactly once when a body exists');

  global.document = savedDocument;
  global.MutationObserver = savedObserver;
}
console.log('inlineIcon.test.js: bodyless-frame observer guard OK');

const { isExtensionContextAlive, sendRuntimeMessage } = require('../content.js');

// isExtensionContextAlive: chrome.runtime.id is the live signal. Reading a missing
// `runtime` (or a missing chrome altogether) must never throw — that is the entire point
// of this helper existing instead of a bare `chrome.runtime.id` check at each call site.
assert.strictEqual(isExtensionContextAlive({ runtime: { id: 'abc' } }), true, 'a present runtime.id means the context is alive');
assert.strictEqual(isExtensionContextAlive({ runtime: {} }), false, 'a runtime with no id means the context is gone');
assert.strictEqual(isExtensionContextAlive({}), false, 'no runtime at all means the context is gone');
assert.doesNotThrow(() => isExtensionContextAlive(null), 'a null chromeRef must not throw');
assert.strictEqual(isExtensionContextAlive(null), false, 'a null chromeRef means the context is gone');
assert.doesNotThrow(() => isExtensionContextAlive(undefined), 'an undefined chromeRef must not throw');
assert.strictEqual(isExtensionContextAlive(undefined), false, 'an undefined chromeRef means the context is gone (no global chrome under node)');
console.log('inlineIcon.test.js: isExtensionContextAlive OK');

// sendRuntimeMessage: the single choke point every chrome.runtime.sendMessage call in
// content.js goes through. Assert on a recorded call list, not just a boolean, so the test
// actually proves the send reached (or didn't reach) chrome.runtime.sendMessage.
{
  const sentMessages = [];
  const aliveChrome = { runtime: { id: 'abc', sendMessage: (message, callback) => { sentMessages.push(message); if (callback) callback({ ok: true }); } } };
  sendRuntimeMessage({ type: 'getCredentials', url: 'https://example.com' }, () => {}, aliveChrome);
  assert.deepStrictEqual(sentMessages, [{ type: 'getCredentials', url: 'https://example.com' }], 'a live context sends the message through to chrome.runtime.sendMessage');
}
{
  const sentMessages = [];
  const deadChrome = { runtime: { id: undefined, sendMessage: (message) => { sentMessages.push(message); } } };
  sendRuntimeMessage({ type: 'getCredentials', url: 'https://example.com' }, () => {}, deadChrome);
  assert.deepStrictEqual(sentMessages, [], 'a dead context must not reach chrome.runtime.sendMessage at all');
}
console.log('inlineIcon.test.js: extension context guard OK');
