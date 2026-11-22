// After a sign-up the user must be asked to save the new login. On accounts.hetzner.com/signUp
// nothing was asked: the page's "Sign up" is a native submit button whose click the single-page
// app cancels (it posts the form itself, so no `submit` event ever fires), and a password the
// user revealed with the page's "show" toggle was a text field by then, so the form no longer
// had a new-password field at all. Enter in a field of a form-less group submits nothing either.
// The password still travels only after one of those user submits, never on a fill, and only
// once the page shows the submit worked: a new URL, or the form gone or hidden. A click that
// only brought up the page's own validation errors leaves the form in place and asks nothing.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { parseHtml, typeInto } = require('./miniDom.js');
const formDetection = require('../shared/form-detection.js');
const { createRegistrationSubmitWatcher, snapshotRegistration, showSaveHintToast } = require('../content.js');

const SIGN_UP = '<main><form id="signup" novalidate><div class="field"><label for="email">Email</label>'
  + '<input id="email" type="email" name="email"></div>'
  + '<div class="field"><label for="pw">Password</label><input id="pw" type="password" name="password">'
  + '<button type="button" id="show">Show</button></div>'
  + '<div class="field"><label for="pw2">Confirm password</label><input id="pw2" type="password" name="password_confirmation"></div>'
  + '<button type="submit" id="go">Sign up</button></form></main>';
const FORMLESS = SIGN_UP.replace('<form id="signup" novalidate>', '<section id="signup">').replace('</form>', '</section>')
  .replace('<button type="submit" id="go">', '<button type="button" id="go">');
const START_URL = 'https://example.test/signup';

// Timers run in rounds: each round fires every pending timer once (the watcher only ever has
// poll timers of one interval), so `advance(ms)` is deterministic.
function harness(markup) {
  const doc = parseHtml(markup);
  const remembered = new Set(doc.querySelectorAll('input[type="password"]'));
  const proposals = [];
  const queue = [];
  const timers = new Map();
  let nextTimer = 1;
  const page = { url: START_URL };
  const watcher = createRegistrationSubmitWatcher({
    isUserSubmit: (event) => event.isTrusted === true,
    defer: (fn) => queue.push(fn),
    snapshot: (node, submitter) => snapshotRegistration(doc, node, submitter, { wasPassword: (n) => remembered.has(n) }),
    propose: (captured, url) => proposals.push({ ...captured, url }),
    getUrl: () => page.url,
    isShown: (node) => formDetection.isControlShown(node),
    setTimer: (fn, ms) => { const id = nextTimer++; timers.set(id, { fn, ms }); return id; },
    clearTimer: (id) => { timers.delete(id); },
  });
  const flush = () => { while (queue.length) queue.shift()(); };
  const advance = (ms) => {
    let elapsed = 0;
    while (elapsed < ms && timers.size) {
      const due = Array.from(timers.values());
      timers.clear();
      elapsed += due[0].ms;
      for (const timer of due) timer.fn();
    }
  };
  return { doc, watcher, proposals, flush, advance, page, timers, el: (id) => doc.getElementById(id) };
}

function fillIn(h) {
  typeInto(h.el('email'), 'olena@example.test');
  typeInto(h.el('pw'), 'Gen-3rated!');
  typeInto(h.el('pw2'), 'Gen-3rated!');
}

const click = (target, prevented) => ({ type: 'click', isTrusted: true, target, defaultPrevented: !!prevented });
const enterOn = (target) => ({ type: 'keydown', key: 'Enter', isTrusted: true, target, defaultPrevented: false });
const submitOf = (h) => ({ type: 'submit', isTrusted: true, target: h.el('signup'), submitter: h.el('go'), defaultPrevented: false });

// A click on the submit button that the page's own handler cancels (it runs after ours: we
// listen in the capture phase), then the deferred check.
function cancelledClick(h) {
  const event = click(h.el('go'), false);
  h.watcher.onClick(event);
  event.defaultPrevented = true;
  h.flush();
}

test('a cancelled submit click that only shows validation errors asks nothing', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  cancelledClick(h);
  assert.equal(h.proposals.length, 0, 'nothing before the page showed the submit worked');
  h.advance(4750);
  assert.equal(h.proposals.length, 0);
  h.advance(5000);
  assert.equal(h.proposals.length, 0, 'fields still shown and the URL unchanged after 5 s: discarded');
  assert.equal(h.timers.size, 0, 'no timer left behind');
  h.page.url = 'https://example.test/welcome';
  h.watcher.onUrlChange();
  h.watcher.onPageHide();
  assert.equal(h.proposals.length, 0, 'a later route change does not revive a discarded submit');
});

test('a cancelled submit click followed by a single-page route change asks to save', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  cancelledClick(h);
  h.advance(1000);
  assert.equal(h.proposals.length, 0);
  h.page.url = 'https://example.test/welcome'; // history.pushState: no event reaches us, the poll sees it
  h.advance(250);
  assert.equal(h.proposals.length, 1);
  assert.equal(h.proposals[0].username, 'olena@example.test');
  assert.equal(h.proposals[0].password, 'Gen-3rated!');
  assert.equal(h.proposals[0].url, START_URL, 'the login is saved for the sign-up page, not the page after it');
  h.advance(6000);
  assert.equal(h.proposals.length, 1, 'asked once');
});

test('a popstate or hashchange route change is evidence at once', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  cancelledClick(h);
  h.page.url = START_URL + '#done';
  h.watcher.onUrlChange();
  assert.equal(h.proposals.length, 1);
});

test('a submit after which the page hides or replaces the form asks to save', () => {
  const hidden = harness(SIGN_UP);
  fillIn(hidden);
  cancelledClick(hidden);
  hidden.el('signup').setAttribute('style', 'display:none');
  hidden.advance(250);
  assert.equal(hidden.proposals.length, 1, 'form hidden');

  const replaced = harness(SIGN_UP);
  fillIn(replaced);
  cancelledClick(replaced);
  replaced.el('pw').value = ''; // the app clears its state before it swaps in the success view
  replaced.el('pw2').value = '';
  replaced.el('signup').remove();
  replaced.doc.querySelector('main').appendChild(replaced.doc.createElement('p'));
  replaced.advance(250);
  assert.equal(replaced.proposals.length, 1, 'form replaced');
  assert.equal(replaced.proposals[0].password, 'Gen-3rated!', 'the values are the ones at submit time');

  const passwordGone = harness(FORMLESS);
  fillIn(passwordGone);
  passwordGone.watcher.onKeydown(enterOn(passwordGone.el('pw2')));
  passwordGone.flush();
  passwordGone.el('pw').hidden = true;
  passwordGone.advance(250);
  assert.equal(passwordGone.proposals.length, 1, 'the group\'s password field hidden');
});

test('a native submit the page lets through asks at once, before the page unloads', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  h.watcher.onClick(click(h.el('go'), false));
  h.watcher.onSubmit(submitOf(h));
  h.flush();
  assert.equal(h.proposals.length, 1, 'counted once, by the submit event');
  h.advance(6000);
  h.watcher.onPageHide();
  assert.equal(h.proposals.length, 1);
});

test('a submit the page cancels to post it itself waits for evidence', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  const submit = submitOf(h);
  h.watcher.onSubmit(submit);
  submit.defaultPrevented = true;
  h.flush();
  assert.equal(h.proposals.length, 0);
  h.advance(5000);
  assert.equal(h.proposals.length, 0, 'validation errors only: discarded');

  const ok = harness(SIGN_UP);
  fillIn(ok);
  const submit2 = submitOf(ok);
  ok.watcher.onSubmit(submit2);
  submit2.defaultPrevented = true;
  ok.flush();
  ok.watcher.onPageHide();
  assert.equal(ok.proposals.length, 1, 'the page navigated away');
});

test('a click the browser blocked (validation failed, no submit) asks nothing', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  h.watcher.onClick(click(h.el('go'), false));
  h.flush();
  h.watcher.onPageHide();
  assert.equal(h.proposals.length, 0);
});

test('a revealed password still counts as the new password', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  h.el('pw').setAttribute('type', 'text');
  h.watcher.onSubmit(submitOf(h));
  h.flush();
  assert.equal(h.proposals.length, 1);
  assert.equal(h.proposals[0].password, 'Gen-3rated!');
});

test('Enter in a field of a form-less sign-up group asks to save once the page acts on it', () => {
  const h = harness(FORMLESS);
  fillIn(h);
  h.watcher.onKeydown(enterOn(h.el('pw2')));
  h.flush();
  h.advance(5000);
  assert.equal(h.proposals.length, 0, 'Enter the page ignored: nothing happened, nothing asked');
  h.watcher.onKeydown(enterOn(h.el('pw2')));
  h.flush();
  h.page.url = 'https://example.test/welcome';
  h.advance(250);
  assert.equal(h.proposals.length, 1);
  h.watcher.onKeydown({ ...enterOn(h.el('pw2')), key: 'a' });
  h.watcher.onKeydown({ ...enterOn(h.el('pw2')), isComposing: true });
  h.flush();
  h.watcher.onPageHide();
  assert.equal(h.proposals.length, 1, 'other keys and an IME commit are no submit');
});

test('Enter in a <form> counts through its submit event, or when the page cancels it and acts', () => {
  const h = harness(SIGN_UP);
  fillIn(h);
  h.watcher.onKeydown(enterOn(h.el('pw2')));
  h.flush();
  h.watcher.onPageHide();
  assert.equal(h.proposals.length, 0, 'no submit followed and the page did not take the key');
  const taken = enterOn(h.el('pw2'));
  h.watcher.onKeydown(taken);
  taken.defaultPrevented = true;
  h.flush();
  h.el('signup').hidden = true;
  h.advance(250);
  assert.equal(h.proposals.length, 1);
});

test('untrusted events and clicks outside a sign-up form never send a password', () => {
  const h = harness(SIGN_UP + '<button type="button" id="other">Next</button>');
  fillIn(h);
  h.watcher.onClick({ type: 'click', isTrusted: false, target: h.el('go'), defaultPrevented: true });
  h.watcher.onSubmit({ ...submitOf(h), isTrusted: false });
  h.watcher.onClick(click(h.el('other'), true));
  h.watcher.onClick(click(h.el('show'), true));
  h.flush();
  h.watcher.onPageHide();
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
