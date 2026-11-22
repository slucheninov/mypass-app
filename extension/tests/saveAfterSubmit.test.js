// After a sign-up the user must be asked to save the new login. On accounts.hetzner.com/signUp
// nothing was asked: the page's "Sign up" is a native submit button whose click the single-page
// app cancels (it posts the form itself, so no `submit` event ever fires), and a password the
// user revealed with the page's "show" toggle was a text field by then, so the form no longer
// had a new-password field at all. Enter in a field of a form-less group submits nothing either.
// The password still travels only after one of those user submits, never on a fill.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { parseHtml, typeInto } = require('./miniDom.js');
const registration = require('../shared/registration-form.js');
const { createRegistrationSubmitWatcher, showSaveHintToast } = require('../content.js');

const SIGN_UP = '<main><form id="signup" novalidate><div class="field"><label for="email">Email</label>'
  + '<input id="email" type="email" name="email"></div>'
  + '<div class="field"><label for="pw">Password</label><input id="pw" type="password" name="password">'
  + '<button type="button" id="show">Show</button></div>'
  + '<div class="field"><label for="pw2">Confirm password</label><input id="pw2" type="password" name="password_confirmation"></div>'
  + '<button type="submit" id="go">Sign up</button></form></main>';

function harness(markup) {
  const doc = parseHtml(markup);
  const remembered = new Set(doc.querySelectorAll('input[type="password"]'));
  const proposals = [];
  const queue = [];
  const watcher = createRegistrationSubmitWatcher({
    isUserSubmit: (event) => event.isTrusted === true,
    defer: (fn) => queue.push(fn),
    propose: (node, submitter) => {
      const form = registration.formFor(doc, node, { submitter, wasPassword: (n) => remembered.has(n) });
      const captured = form && registration.capture(form);
      if (captured) proposals.push(captured);
    },
  });
  const flush = () => { while (queue.length) queue.shift()(); };
  return { doc, watcher, proposals, flush, el: (id) => doc.getElementById(id) };
}

function fillIn(h) {
  typeInto(h.el('email'), 'olena@example.test');
  typeInto(h.el('pw'), 'Gen-3rated!');
  typeInto(h.el('pw2'), 'Gen-3rated!');
}

const click = (target, prevented) => ({ type: 'click', isTrusted: true, target, defaultPrevented: !!prevented });

test('a submit-button click the page cancels to post the form itself asks to save', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  const event = click(h.el('go'), false);
  h.watcher.onClick(event);
  event.defaultPrevented = true; // the app's own click handler, after ours (capture phase)
  assert.equal(h.proposals.length, 0, 'nothing is sent while the click is still being handled');
  h.flush();
  assert.equal(h.proposals.length, 1);
  assert.equal(h.proposals[0].username, 'olena@example.test');
  assert.equal(h.proposals[0].password, 'Gen-3rated!');
});

test('a click that led to a native submit is counted once, by the submit event', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  h.watcher.onClick(click(h.el('go'), false));
  h.watcher.onSubmit({ type: 'submit', isTrusted: true, target: h.el('signup'), submitter: h.el('go') });
  h.flush();
  assert.equal(h.proposals.length, 1);
});

test('a click the browser blocked (validation failed, no submit) asks nothing', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  h.watcher.onClick(click(h.el('go'), false));
  h.flush();
  assert.equal(h.proposals.length, 0);
});

test('a revealed password still counts as the new password', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  h.el('pw').setAttribute('type', 'text');
  h.watcher.onSubmit({ type: 'submit', isTrusted: true, target: h.el('signup'), submitter: h.el('go') });
  assert.equal(h.proposals.length, 1);
  assert.equal(h.proposals[0].password, 'Gen-3rated!');
});

test('Enter in a field of a form-less sign-up group asks to save', () => {
  const markup = SIGN_UP.replace('<form id="signup" novalidate>', '<section id="signup">').replace('</form>', '</section>')
    .replace('<button type="submit" id="go">', '<button type="button" id="go">');
  const h = harness(markup);
  fillIn(h);
  h.watcher.onKeydown({ type: 'keydown', key: 'Enter', isTrusted: true, target: h.el('pw2'), defaultPrevented: false });
  h.flush();
  assert.equal(h.proposals.length, 1);
  h.watcher.onKeydown({ type: 'keydown', key: 'a', isTrusted: true, target: h.el('pw2'), defaultPrevented: false });
  h.watcher.onKeydown({ type: 'keydown', key: 'Enter', isComposing: true, isTrusted: true, target: h.el('pw2'), defaultPrevented: false });
  h.flush();
  assert.equal(h.proposals.length, 1, 'other keys and an IME commit are no submit');
});

test('Enter in a <form> counts through its submit event, or when the page cancels it', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  const enter = { type: 'keydown', key: 'Enter', isTrusted: true, target: h.el('pw2'), defaultPrevented: false };
  h.watcher.onKeydown(enter);
  h.flush();
  assert.equal(h.proposals.length, 0, 'no submit followed and the page did not take the key');
  const taken = { ...enter };
  h.watcher.onKeydown(taken);
  taken.defaultPrevented = true;
  h.flush();
  assert.equal(h.proposals.length, 1);
});

test('untrusted events and clicks outside a sign-up form never send a password', () => {
  const h = harness(SIGN_UP + '<button type="button" id="other">Next</button>');
  fillIn(h);
  const fake = { type: 'click', isTrusted: false, target: h.el('go'), defaultPrevented: true };
  h.watcher.onClick(fake);
  h.watcher.onSubmit({ type: 'submit', isTrusted: false, target: h.el('signup'), submitter: null });
  h.watcher.onClick(click(h.el('other'), true));
  h.watcher.onClick(click(h.el('show'), true));
  h.flush();
  assert.equal(h.proposals.length, 0);
});

test('a hint after a generated password says the login will be offered for saving on submit', () => {
  const doc = parseHtml('<main><input id="pw" type="password"></main>');
  const timers = [];
  const host = showSaveHintToast(doc, { setTimeout: (fn, ms) => timers.push({ fn, ms }), text: 'Save on submit' });
  assert.ok(host.hasAttribute('data-mypass-save-hint'));
  assert.equal(doc.querySelectorAll('[data-mypass-save-hint]').length, 1);
  const status = host.shadowRoot.querySelector('[role="status"]');
  assert.equal(status.getAttribute('aria-live'), 'polite');
  assert.match(status.textContent, /Save on submit/);
  assert.equal(doc.body.textContent.includes('Save on submit'), false, 'the text is in a shadow root, not the page DOM');
  const again = showSaveHintToast(doc, { setTimeout: (fn, ms) => timers.push({ fn, ms }), text: 'Save on submit' });
  assert.equal(doc.querySelectorAll('[data-mypass-save-hint]').length, 1, 'one hint at a time');
  assert.equal(again.isConnected, true);
  timers.at(-1).fn();
  assert.equal(doc.querySelectorAll('[data-mypass-save-hint]').length, 0, 'hides by itself');
  assert.ok(timers.every((timer) => timer.ms >= 4000));
});
