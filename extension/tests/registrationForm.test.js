// Registration (sign-up) forms: detection in several languages, the fields that are never
// touched, filling every field from an Address entry, and what a submit captures.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const registration = require('../shared/registration-form.js');

// A small fake DOM: inputs, selects and buttons with the attributes the module reads.
function page() {
  const fields = [];
  const buttons = [];
  const doc = {
    querySelectorAll(selector) {
      return selector.startsWith('input, select') ? fields.slice() : buttons.slice();
    },
  };
  const form = { tagName: 'FORM' };
  function node(tag, attrs, extra) {
    const created = {
      tagName: tag, value: '', isConnected: true, disabled: false, readOnly: false, multiple: false,
      form, labels: [], attrs: { ...attrs },
      getAttribute(name) { return Object.hasOwn(this.attrs, name) ? this.attrs[name] : null; },
      closest(selector) { return selector === 'form' ? this.form : null; },
      ...extra,
    };
    created.type = tag === 'INPUT' ? (attrs.type || 'text') : '';
    created.name = attrs.name || '';
    created.id = attrs.id || '';
    if (extra && extra.label) created.labels = [{ textContent: extra.label, contains: () => false }];
    return created;
  }
  return {
    doc, form,
    input(attrs = {}, extra = {}) {
      const created = node('INPUT', attrs, extra);
      fields.push(created);
      return created;
    },
    select(attrs, options, extra = {}) {
      const created = node('SELECT', attrs, { options, size: 0, ...extra });
      const chosen = options.find((option) => option.selected);
      created.value = chosen ? chosen.value : '';
      fields.push(created);
      return created;
    },
    button(text, attrs = {}) {
      const created = node(attrs.type === 'submit' && attrs.input ? 'INPUT' : 'BUTTON', attrs, { textContent: text });
      buttons.push(created);
      return created;
    },
  };
}

const visible = { isVisible: (node) => !node.invisible };
const kinds = (found) => found.fields.map((field) => field.kind);
const write = (node, value) => { node.value = value; };

test('an English sign-up form: email, names, phone, new password and its confirmation', () => {
  const p = page();
  p.input({ type: 'email', name: 'email' });
  p.input({ name: 'fname' }, { label: 'First name' });
  p.input({ name: 'lastName' });
  p.input({ type: 'tel', autocomplete: 'tel' });
  const password = p.input({ type: 'password', autocomplete: 'new-password' });
  p.input({ type: 'password', name: 'confirm' });
  const [found, ...rest] = registration.find(p.doc, visible);
  assert.equal(rest.length, 0);
  assert.equal(found.newPassword, password);
  assert.deepEqual(kinds(found), ['email', 'givenName', 'familyName', 'phone']);
});

test('a Ukrainian form with one password and a register button', () => {
  const p = page();
  p.input({}, { label: "\u0406\u043c'\u044f" });
  p.input({}, { label: '\u041f\u0440\u0456\u0437\u0432\u0438\u0449\u0435' });
  p.input({}, { label: '\u0422\u0435\u043b\u0435\u0444\u043e\u043d' });
  p.input({}, { label: '\u0415\u043b\u0435\u043a\u0442\u0440\u043e\u043d\u043d\u0430 \u043f\u043e\u0448\u0442\u0430' });
  const password = p.input({ type: 'password' });
  p.button('\u0417\u0430\u0440\u0435\u0454\u0441\u0442\u0440\u0443\u0432\u0430\u0442\u0438\u0441\u044f');
  const [found] = registration.find(p.doc, visible);
  assert.equal(found.newPassword, password, 'a single password next to identity fields is the new one');
  assert.deepEqual(kinds(found), ['givenName', 'familyName', 'phone', 'email']);
});

test('a Russian form: username, birthday, email and two passwords', () => {
  const p = page();
  p.input({ placeholder: '\u0418\u043c\u044f \u043f\u043e\u043b\u044c\u0437\u043e\u0432\u0430\u0442\u0435\u043b\u044f' });
  p.input({ type: 'date', placeholder: '\u0414\u0430\u0442\u0430 \u0440\u043e\u0436\u0434\u0435\u043d\u0438\u044f' });
  p.input({ placeholder: '\u042d\u043b\u0435\u043a\u0442\u0440\u043e\u043d\u043d\u0430\u044f \u043f\u043e\u0447\u0442\u0430' });
  p.input({ type: 'password' });
  p.input({ type: 'password' });
  const [found] = registration.find(p.doc, visible);
  assert.deepEqual(kinds(found), ['username', 'birthday', 'email']);
});

test('a German form with address parts and a "Konto erstellen" button', () => {
  const p = page();
  p.input({ name: 'vorname' });
  p.input({ 'aria-label': 'Nachname' });
  p.input({ type: 'email' }, { label: 'E-Mail-Adresse' });
  p.input({}, { label: 'Geburtsdatum' });
  p.input({}, { label: 'Stra\u00dfe' });
  p.input({}, { label: 'PLZ' });
  p.input({}, { label: 'Stadt' });
  p.input({ type: 'password' });
  p.button('Konto erstellen');
  const [found] = registration.find(p.doc, visible);
  assert.deepEqual(kinds(found), ['givenName', 'familyName', 'email', 'birthday', 'addressLine1', 'postalCode', 'city']);
});

test('login and change-password forms are not registration forms', () => {
  const login = page();
  login.input({ type: 'email', name: 'email' });
  login.input({ type: 'password' });
  login.button('Sign in');
  assert.deepEqual(registration.find(login.doc, visible), []);

  const change = page();
  change.input({ type: 'password', autocomplete: 'current-password' });
  change.input({ type: 'password', autocomplete: 'new-password' });
  change.input({ type: 'password', autocomplete: 'new-password' });
  assert.deepEqual(registration.find(change.doc, visible), []);

  const unhinted = page();
  unhinted.input({ name: 'first_name' });
  for (let i = 0; i < 3; i++) unhinted.input({ type: 'password' });
  assert.deepEqual(registration.find(unhinted.doc, visible), [], 'three unhinted passwords are a change-password form');
});

test('hidden, card, code, search, disabled and invisible fields are never classified', () => {
  const p = page();
  p.input({ type: 'email', name: 'email' });
  p.input({ name: 'first_name' });
  p.input({ type: 'hidden', name: 'last_name' });
  p.input({ autocomplete: 'cc-number', name: 'phone' });
  p.input({}, { label: 'CVV' });
  p.input({}, { label: 'Card number' });
  p.input({ autocomplete: 'one-time-code' });
  p.input({ name: 'q', placeholder: 'Search' });
  p.input({ name: 'city' }, { disabled: true });
  p.input({ name: 'region' }, { invisible: true });
  p.input({ type: 'checkbox', name: 'newsletter' }, { label: 'Newsletter' });
  p.input({ type: 'password', autocomplete: 'new-password' });
  const [found] = registration.find(p.doc, visible);
  assert.deepEqual(kinds(found), ['email', 'givenName']);
});

test('a passwordless form with several identity fields is offered for filling, never for saving', () => {
  const p = page();
  p.input({ autocomplete: 'name' });
  p.input({ type: 'email' });
  p.input({ autocomplete: 'address-level2' });
  const [found] = registration.find(p.doc, visible);
  assert.equal(found.newPassword, null);
  assert.equal(registration.capture(found), null);
  const two = page();
  two.input({ autocomplete: 'name' });
  two.input({ type: 'email' });
  assert.deepEqual(registration.find(two.doc, visible), [], 'a name and an email alone are a newsletter box');
});

const address = {
  id: 'a1', fullName: 'Ann Lee', givenName: '', familyName: '', organization: 'Example Ltd',
  addressLine1: '1 Main St', addressLine2: '', city: 'Lviv', region: '', postalCode: '79000',
  countryCode: 'UA', countryName: 'Ukraine', phone: '+380501234567', email: 'ann@example.test',
};

test('choosing an Address entry fills every empty fillable field of the form at once', () => {
  const p = page();
  const email = p.input({ type: 'email', name: 'email' });
  const given = p.input({ name: 'first_name' });
  const family = p.input({ name: 'last_name' });
  const phone = p.input({ type: 'tel', autocomplete: 'tel' }, { value: '+48 typed by the user' });
  const username = p.input({ autocomplete: 'username' });
  const city = p.input({ autocomplete: 'address-level2' });
  const country = p.select({ autocomplete: 'country' }, [
    { value: '', textContent: 'Choose', selected: true, defaultSelected: true },
    { value: 'UA', textContent: 'Ukraine', selected: false, defaultSelected: false },
  ]);
  p.input({ type: 'password', autocomplete: 'new-password' });
  const [found] = registration.find(p.doc, visible);
  const selection = registration.selectionFor(p.doc, given, visible);
  const events = [];
  const filled = registration.fillSelection(p.doc, selection, address, (node, value) => { events.push(node); write(node, value); }, visible);
  assert.equal(filled, 5);
  assert.deepEqual([email.value, given.value, family.value, city.value, country.value],
    ['ann@example.test', 'Ann', 'Lee', 'Lviv', 'UA']);
  assert.equal(phone.value, '+48 typed by the user', 'what the user typed stays');
  assert.equal(username.value, '', 'an Address entry has no username');
  assert.equal(events.at(0), email);
  assert.deepEqual(registration.fillableFields(found).map((field) => field.kind),
    ['email', 'givenName', 'familyName', 'phone', 'city', 'country']);
});

test('the fill goes only into the controls chosen at the click, each checked again before its write', () => {
  const p = page();
  const given = p.input({ name: 'first_name' });
  const family = p.input({ name: 'last_name' });
  const email = p.input({ type: 'email', name: 'email' });
  const city = p.input({ autocomplete: 'address-level2' });
  const zip = p.input({ autocomplete: 'postal-code' });
  const phone = p.input({ type: 'tel', autocomplete: 'tel' });
  p.input({ type: 'password', autocomplete: 'new-password' });
  const selection = registration.selectionFor(p.doc, given, visible);
  // Added after the click: never part of the fill.
  const late = p.input({ autocomplete: 'organization' });
  const other = { tagName: 'FORM' };
  const written = [];
  const filled = registration.fillSelection(p.doc, selection, { ...address, organization: 'Example Ltd' }, (node, value) => {
    written.push(node);
    write(node, value);
    if (node === given) {
      // The page's input handler moves, hides, retypes and fills the remaining controls.
      family.form = other;
      email.invisible = true;
      city.attrs.autocomplete = 'one-time-code';
      zip.value = '12345';
    }
  }, visible);
  assert.deepEqual(written, [given, phone]);
  assert.equal(filled, 2);
  assert.deepEqual([family.value, email.value, city.value, zip.value, late.value], ['', '', '', '12345', '']);
  // A fill whose page went away writes nothing.
  const again = page();
  const name = again.input({ name: 'first_name' });
  again.input({ type: 'email' });
  again.input({ type: 'password', autocomplete: 'new-password' });
  const chosen = registration.selectionFor(again.doc, name, visible);
  assert.equal(registration.fillSelection(again.doc, chosen, address, write, { ...visible, stillCurrent: () => false }), 0);
  name.isConnected = false;
  assert.equal(registration.fillSelection(again.doc, chosen, address, write, visible), 0);
  assert.equal(registration.selectionFor(again.doc, null, visible), null);
});

test('a submit captures the new password, the username (else the email) and the other fields', () => {
  const p = page();
  p.input({ type: 'email', name: 'email' }, { value: 'ann@example.test' });
  p.input({ type: 'email', name: 'email_confirm' }, { value: 'Ann@Example.test ' });
  p.input({ name: 'first_name' }, { value: 'Ann' });
  p.input({ name: 'team' }, { label: 'Favourite team', value: 'Karpaty' });
  p.input({ name: 'mypass_field' }, { label: 'MyPass.Notes', value: 'tampered' });
  p.input({ autocomplete: 'address-level2' }, { label: 'kp2a_city', value: 'Lviv' });
  p.input({ name: 'motto' }, { label: 'Motto', value: 'x'.repeat(513) });
  p.input({ name: 'about' }, { label: 'About', value: 'two\nlines' });
  p.select({ autocomplete: 'country' }, [{ value: 'UA', textContent: 'Ukraine', selected: true }]);
  const password = p.input({ type: 'password', autocomplete: 'new-password' }, { value: 'N3w-Passw0rd' });
  p.input({ type: 'password' }, { value: 'N3w-Passw0rd' });
  const [found] = registration.find(p.doc, visible);
  const captured = registration.capture(found);
  assert.equal(captured.password, 'N3w-Passw0rd');
  assert.equal(captured.username, 'ann@example.test', 'no username field: the email is the login');
  // The confirm email repeats the login, a reserved label is dropped (and an "other" field
  // without a label with it), too long and multi-line values stay behind, a select gives its text.
  assert.deepEqual(captured.fields, [
    { kind: 'givenName', label: 'first_name', value: 'Ann' },
    { kind: 'other', label: 'Favourite team', value: 'Karpaty' },
    { kind: 'city', label: '', value: 'Lviv' },
    { kind: 'country', label: '', value: 'Ukraine' },
  ]);
  assert.equal(captured.usernameIsEmail, true, 'the login is the email');
  password.value = '';
  assert.equal(registration.capture(found), null, 'no password, no prompt');
});

test('a username field is the login and the email stays a field; at most 24 fields travel', () => {
  const p = page();
  p.input({ autocomplete: 'username' }, { value: 'ann_l' });
  p.input({ type: 'email' }, { value: 'ann@example.test' });
  for (let i = 0; i < 30; i++) p.input({ name: 'extra' + i }, { label: 'Extra ' + i, value: 'v' + i });
  p.input({ type: 'password', autocomplete: 'new-password' }, { value: 'pw' });
  const captured = registration.capture(registration.find(p.doc, visible)[0]);
  assert.equal(captured.username, 'ann_l');
  assert.equal(captured.usernameIsEmail, false);
  assert.equal(captured.fields.length, 24);
  assert.deepEqual(captured.fields[0], { kind: 'email', label: '', value: 'ann@example.test' });
  assert.deepEqual(captured.fields[1], { kind: 'other', label: 'Extra 0', value: 'v0' });
});

test('labels are cleaned and cut to 64 characters', () => {
  const p = page();
  const field = p.input({ name: 'x' }, { label: '  Your favourite\n colour:* ' + 'y'.repeat(80) });
  assert.equal(registration.readableLabel(field).length, 64);
  const short = p.input({ name: 'y' }, { label: 'Colour\u200b:' });
  assert.equal(registration.readableLabel(short), 'Colour');
});

test('submit controls: submit buttons and localized sign-up buttons, not other buttons', () => {
  const p = page();
  assert.equal(registration.isSubmitControl(p.button('Go', { type: 'submit' })), true);
  assert.equal(registration.isSubmitControl(p.button('Create account', { type: 'button' })), true);
  assert.equal(registration.isSubmitControl(p.button('Registrieren', { type: 'button' })), true);
  assert.equal(registration.isSubmitControl(p.button('\u0417\u0430\u0440\u0435\u0433\u0438\u0441\u0442\u0440\u0438\u0440\u043e\u0432\u0430\u0442\u044c\u0441\u044f', { type: 'button' })), true);
  assert.equal(registration.isSubmitControl(p.button('Show password', { type: 'button' })), false);
  assert.equal(registration.isSubmitControl(null), false);
});

test('formFor finds the registration form of a field or of its button', () => {
  const p = page();
  const email = p.input({ type: 'email' });
  p.input({ name: 'first_name' });
  p.input({ type: 'password', autocomplete: 'new-password' });
  const button = p.button('Sign up', { type: 'submit' });
  assert.equal(registration.formFor(p.doc, email, visible).owner, p.form);
  assert.equal(registration.formFor(p.doc, button, visible).owner, p.form);
  assert.equal(registration.formFor(p.doc, null, visible), null);
});

test('sensitive hints win over autocomplete tokens and every positive hint, in four languages', () => {
  const p = page();
  const cases = [
    [{ autocomplete: 'tel', name: 'phone' }, { label: 'Verification code' }],
    [{ name: 'mfaCode' }, { label: 'Authentication code' }],
    [{ autocomplete: 'name' }, { label: 'CVV' }],
    [{ autocomplete: 'email' }, { label: 'Card holder email' }],
    [{ autocomplete: 'postal-code', 'aria-label': 'Coupon code' }, {}],
    [{ autocomplete: 'username', name: 'otp' }, {}],
    [{ type: 'email', name: 'twoFactorCode' }, {}],
    [{ name: 'pin' }, { label: 'PIN' }],
    [{ autocomplete: 'given-name' }, { label: 'Captcha' }],
    [{ autocomplete: 'organization' }, { label: 'IBAN' }],
    [{ autocomplete: 'nickname' }, { label: 'Passport number' }],
    [{ autocomplete: 'bday' }, { label: 'SSN' }],
    [{ autocomplete: 'address-line1' }, { label: 'Security answer' }],
    [{ type: 'tel' }, { label: 'One-time password' }],
    [{ autocomplete: 'name', name: 'search' }, {}],
    [{ autocomplete: 'username', name: 'api_token' }, {}],
    [{ autocomplete: 'tel' }, { label: 'Best\u00e4tigungscode' }],
    [{ autocomplete: 'tel' }, { label: '\u041a\u043e\u0434 \u043f\u0456\u0434\u0442\u0432\u0435\u0440\u0434\u0436\u0435\u043d\u043d\u044f' }],
    [{ autocomplete: 'tel' }, { label: '\u041a\u043e\u0434 \u043f\u043e\u0434\u0442\u0432\u0435\u0440\u0436\u0434\u0435\u043d\u0438\u044f' }],
    [{ autocomplete: 'tel' }, { label: '\u041e\u0434\u043d\u043e\u0440\u0430\u0437\u043e\u0432\u0438\u0439 \u043a\u043e\u0434' }],
    [{ autocomplete: 'name' }, { label: '\u041f\u0418\u041d-\u043a\u043e\u0434' }],
  ];
  for (const [attrs, extra] of cases) {
    assert.equal(registration.classify(p.input(attrs, extra)), null, JSON.stringify([attrs, extra]));
  }
  // The positive hints still work where nothing sensitive is said.
  assert.equal(registration.classify(p.input({ autocomplete: 'tel' }, { label: 'Phone' })), 'phone');
  assert.equal(registration.classify(p.input({ autocomplete: 'postal-code' }, { label: 'ZIP code' })), 'postalCode');
  assert.equal(registration.classify(p.input({ autocomplete: 'country' }, { label: 'Country code' })), 'country');
});

test('one password counts as new only when the control that submits says "Sign up"', () => {
  // Any other button of the form saying "Create account" is no evidence.
  const login = page();
  login.input({ type: 'tel', autocomplete: 'tel' }, { label: 'Phone' });
  login.input({ name: 'first_name' });
  login.input({ type: 'password' });
  const signIn = login.button('Sign in', { type: 'submit' });
  const link = login.button('Create account', { type: 'button' });
  assert.deepEqual(registration.find(login.doc, visible), [], 'a phone login with a "Create account" link');
  assert.deepEqual(registration.find(login.doc, { ...visible, submitter: signIn }), []);
  assert.equal(registration.find(login.doc, { ...visible, submitter: link }).length, 1,
    'a click on the script "Create account" control is evidence for itself');
  // The submitter decides, not the default button.
  const signup = page();
  signup.input({ type: 'email' });
  signup.input({ type: 'password' });
  const register = signup.button('Register', { type: 'submit' });
  const other = signup.button('Sign in', { type: 'submit' });
  assert.equal(registration.find(signup.doc, visible).length, 1, 'the default button says "Register"');
  assert.equal(registration.find(signup.doc, { ...visible, submitter: register }).length, 1);
  assert.deepEqual(registration.find(signup.doc, { ...visible, submitter: other }), []);
});

test('a field for the current password makes any form a change-password form', () => {
  const labels = ['Current password', 'Old password', 'Existing password', 'Aktuelles Passwort',
    '\u041f\u043e\u0442\u043e\u0447\u043d\u0438\u0439 \u043f\u0430\u0440\u043e\u043b\u044c',
    '\u0422\u0435\u043a\u0443\u0449\u0438\u0439 \u043f\u0430\u0440\u043e\u043b\u044c'];
  for (const label of labels) {
    const p = page();
    p.input({ type: 'email' });
    p.input({ name: 'first_name' });
    p.input({ type: 'password' }, { label });
    p.input({ type: 'password' });
    p.button('Sign up', { type: 'submit' });
    assert.deepEqual(registration.find(p.doc, visible), [], label);
  }
  const named = page();
  named.input({ type: 'email' });
  named.input({ type: 'password', name: 'oldPassword' });
  named.input({ type: 'password', autocomplete: 'new-password' });
  assert.deepEqual(registration.find(named.doc, visible), []);
  // A card code typed into a password field is not a password at all.
  const card = page();
  card.input({ type: 'email' });
  card.input({ type: 'password', name: 'cvv' });
  card.button('Sign up', { type: 'submit' });
  assert.deepEqual(registration.find(card.doc, visible), []);
});

test('script controls stand for a submit; native submit buttons wait for the submit event', () => {
  const p = page();
  assert.equal(registration.isScriptSubmitControl(p.button('Sign up', { type: 'submit' })), false);
  assert.equal(registration.isScriptSubmitControl(p.button('Register')), false, 'a typeless button submits its form');
  assert.equal(registration.isScriptSubmitControl(p.button('Sign up', { type: 'button' })), true);
  assert.equal(registration.isScriptSubmitControl(p.button('Next', { type: 'button' })), false);
  const loose = p.button('Create account');
  loose.form = null;
  assert.equal(registration.isScriptSubmitControl(loose), true, 'a submit button outside any form submits nothing');
});

// A styled fake for the real visibility check: computed styles, boxes and ancestors.
function styledPage() {
  const p = page();
  const styles = new Map();
  const view = { getComputedStyle: (node) => ({ visibility: 'visible', display: 'block', opacity: '1', ...styles.get(node) }) };
  const ownerDocument = { defaultView: view };
  const wrapper = { tagName: 'DIV', parentElement: null };
  const make = (kind) => (attrs = {}, extra = {}) => {
    const created = p[kind](attrs, ...(kind === 'select' ? [extra.options || [], extra] : [extra]));
    created.ownerDocument = ownerDocument;
    created.offsetParent = wrapper;
    created.parentElement = created.parentElement || wrapper;
    created.getBoundingClientRect = () => created.box || { width: 120, height: 24 };
    const closest = created.closest;
    created.closest = (selector) => (selector.includes('aria-hidden') && created.ariaHidden ? wrapper : closest.call(created, selector));
    created.matches = (selector) => selector === ':disabled' && created.fieldsetDisabled === true;
    return created;
  };
  return { ...p, styles, input: make('input') };
}

test('without an override, a field counts only when it is really shown and enabled', () => {
  const p = styledPage();
  p.input({ type: 'email', name: 'email' });
  p.input({ name: 'first_name' });
  const faded = p.input({ name: 'last_name' });
  p.styles.set(faded.parentElement = { tagName: 'DIV', parentElement: null }, { opacity: '0' });
  const hidden = p.input({ autocomplete: 'tel' });
  p.styles.set(hidden, { visibility: 'hidden' });
  p.input({ autocomplete: 'address-level2' }, { box: { width: 0, height: 0 } });
  p.input({ autocomplete: 'postal-code' }, { fieldsetDisabled: true });
  p.input({ autocomplete: 'organization' }, { ariaHidden: true });
  const gone = p.input({ autocomplete: 'country-name' });
  p.styles.set(gone.parentElement = { tagName: 'SECTION', parentElement: null }, { display: 'none' });
  p.input({ type: 'password', autocomplete: 'new-password' });
  p.input({ type: 'password' }, { fieldsetDisabled: true });
  const [found] = registration.find(p.doc);
  assert.deepEqual(kinds(found), ['email', 'givenName']);
  assert.equal(found.newPassword.attrs.autocomplete, 'new-password');
});

test('controls outside any form are grouped by proximity, never document-wide', () => {
  const p = page();
  const body = { tagName: 'BODY', parentElement: null };
  const signup = { tagName: 'SECTION', parentElement: body };
  const newsletter = { tagName: 'FOOTER', parentElement: body };
  const boundaries = new Map([[signup, signup], [newsletter, newsletter]]);
  const loose = (attrs, extra, section) => {
    const row = { tagName: 'DIV', parentElement: section };
    const created = p.input(attrs, extra);
    created.form = null;
    created.parentElement = row;
    created.closest = (selector) => (selector === 'form' ? null : selector.includes('section') ? boundaries.get(section) : null);
    return created;
  };
  const given = loose({ name: 'first_name' }, {}, signup);
  loose({ type: 'email' }, {}, signup);
  loose({ autocomplete: 'address-level2' }, {}, signup);
  loose({ type: 'password', autocomplete: 'new-password' }, {}, signup);
  loose({ autocomplete: 'name' }, {}, newsletter);
  loose({ type: 'email' }, {}, newsletter);
  loose({ autocomplete: 'postal-code' }, {}, newsletter);
  const forms = registration.find(p.doc, visible);
  assert.equal(forms.length, 2, 'the sign-up section and the newsletter footer are two groups');
  const selection = registration.selectionFor(p.doc, given, visible);
  assert.deepEqual(selection.controls.map((control) => control.kind), ['givenName', 'email', 'city']);
});
