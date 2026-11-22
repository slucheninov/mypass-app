// A generated password goes into every new-password field of the form it was generated for:
// on accounts.hetzner.com/signUp the generator filled the password but not "Confirm password",
// and the form refused to submit because the two did not match. Fields for the current
// password, and the fields of another form on the page, are never touched.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { parseHtml } = require('./miniDom.js');
const registration = require('../shared/registration-form.js');
const formDetection = require('../shared/form-detection.js');
const { fillGeneratedPassword } = require('../content.js');

const write = (node, value) => { node.value = value; };
const filledIds = (doc) => Array.from(doc.querySelectorAll('input')).filter((node) => node.value === 'Gen-3rated!').map((node) => node.id);

// Hetzner-like: a single-page app, no <form>, each field in its own component wrapper.
function field(id, label, attrs) {
  return `<div class="form-group"><label for="${id}">${label}</label><div class="input-wrapper">`
    + `<input id="${id}" ${attrs}><button type="button" class="reveal">Show</button></div></div>`;
}
const SIGN_UP = '<main><div class="sign-up"><h1>Create your account</h1><div class="card"><div class="card-body">'
  + field('email', 'Email', 'type="email" name="email" autocomplete="email"')
  + field('password', 'Password', 'type="password" name="password" autocomplete="new-password"')
  + field('password-confirm', 'Confirm password', 'type="password" name="passwordConfirm" autocomplete="off"')
  + '<button type="button" class="btn-primary">Sign up</button></div></div></div></main>';

test('a sign-up page fills the password and its confirm field with the same value', () => {
  for (const start of ['password', 'password-confirm']) {
    const doc = parseHtml(SIGN_UP);
    const filled = fillGeneratedPassword(doc, doc.getElementById(start), 'Gen-3rated!', write);
    assert.equal(filled.length, 2, start);
    assert.deepEqual(filledIds(doc), ['password', 'password-confirm'], start);
  }
});

test('a confirm field revealed as text (a show-password toggle) still gets the value', () => {
  const doc = parseHtml(SIGN_UP);
  const confirm = doc.getElementById('password-confirm');
  const remembered = new Set([doc.getElementById('password'), confirm]);
  confirm.setAttribute('type', 'text');
  fillGeneratedPassword(doc, doc.getElementById('password'), 'Gen-3rated!', write, { wasPassword: (node) => remembered.has(node) });
  assert.deepEqual(filledIds(doc), ['password', 'password-confirm']);
});

test('two unhinted password fields of a form are password and confirm', () => {
  const doc = parseHtml('<form><input id="u" name="login"><input id="a" type="password" name="pw1">'
    + '<input id="b" type="password" name="pw2"><button>Register</button></form>');
  fillGeneratedPassword(doc, doc.getElementById('a'), 'Gen-3rated!', write);
  assert.deepEqual(filledIds(doc), ['a', 'b']);
});

test('a change-password form never gets the generated value in its current-password field', () => {
  const markup = '<form><label for="cur">Current password</label><input id="cur" type="password">'
    + '<label for="new">New password</label><input id="new" type="password">'
    + '<label for="rep">Repeat new password</label><input id="rep" type="password"><button>Save</button></form>';
  for (const start of ['new', 'rep']) {
    const doc = parseHtml(markup);
    fillGeneratedPassword(doc, doc.getElementById(start), 'Gen-3rated!', write);
    assert.deepEqual(filledIds(doc), ['new', 'rep'], start);
  }
  const unhinted = parseHtml('<form><input id="a" type="password"><input id="b" type="password"><input id="c" type="password"></form>');
  fillGeneratedPassword(unhinted, unhinted.getElementById('b'), 'Gen-3rated!', write);
  assert.deepEqual(filledIds(unhinted), ['b', 'c'], 'three unhinted fields: the first is the current password');
});

test('the sign-in form next to a sign-up form is left alone', () => {
  const doc = parseHtml('<form id="login"><input id="lu" name="username"><input id="lp" type="password" autocomplete="current-password"></form>'
    + '<form id="signup"><input id="su" type="email"><input id="sp" type="password" autocomplete="new-password">'
    + '<input id="sc" type="password" autocomplete="new-password"></form>');
  fillGeneratedPassword(doc, doc.getElementById('sp'), 'Gen-3rated!', write);
  assert.deepEqual(filledIds(doc), ['sp', 'sc']);
});

test('a lone password field gets the value alone, and PIN or code fields never do', () => {
  const doc = parseHtml('<form><input id="p" type="password" autocomplete="new-password">'
    + '<label for="pin">PIN code</label><input id="pin" type="password"></form>');
  fillGeneratedPassword(doc, doc.getElementById('p'), 'Gen-3rated!', write);
  assert.deepEqual(filledIds(doc), ['p']);
});

test('confirm, repeat and retype wording is recognised in four languages', () => {
  for (const label of ['Confirm password', 'Repeat password', 'Retype password', 'Password again', 'Re-enter password',
    'Passwort bestätigen', 'Passwort wiederholen', '\u041f\u043e\u0432\u0442\u043e\u0440\u0456\u0442\u044c \u043f\u0430\u0440\u043e\u043b\u044c', '\u041f\u0430\u0440\u043e\u043b\u044c \u0449\u0435 \u0440\u0430\u0437', '\u041f\u043e\u0432\u0442\u043e\u0440\u0438\u0442\u0435 \u043f\u0430\u0440\u043e\u043b\u044c',
    '\u041f\u0430\u0440\u043e\u043b\u044c \u0435\u0449\u0451 \u0440\u0430\u0437', '\u041f\u043e\u0434\u0442\u0432\u0435\u0440\u0434\u0438\u0442\u0435 \u043f\u0430\u0440\u043e\u043b\u044c']) {
    const doc = parseHtml(`<label for="x">${label}</label><input id="x" type="password">`);
    assert.equal(formDetection.isConfirmPasswordHint(doc.getElementById('x')), true, label);
  }
  for (const name of ['password_confirmation', 'passwordRepeat', 'confirmPassword', 'password2']) {
    const doc = parseHtml(`<input id="x" type="password" name="${name}">`);
    assert.equal(formDetection.isConfirmPasswordHint(doc.getElementById('x')), true, name);
  }
  const plain = parseHtml('<label for="x">Password</label><input id="x" type="password">');
  assert.equal(formDetection.isConfirmPasswordHint(plain.getElementById('x')), false);
  assert.equal(typeof registration.newPasswordTargets, 'function');
});
