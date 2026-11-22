// The page side of social sign-in (shared/social-sign-in.js startPage, wired by content.js): the
// ring for the method recorded on this exact host, trusted-click reporting, and the "Remember
// social sign-in" setting turned off and on while the page is loaded.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const social = require('../shared/social-sign-in.js');

function eventTarget() {
  const listeners = [];
  return {
    listeners,
    addEventListener(type, fn, options) { listeners.push({ type, fn, options }); },
    removeEventListener(type, fn) {
      const at = listeners.findIndex((l) => l.type === type && l.fn === fn);
      if (at !== -1) listeners.splice(at, 1);
    },
    dispatch(type, event) { listeners.filter((l) => l.type === type).forEach((l) => l.fn(event)); },
  };
}

// A page with one "Continue with GitHub" button on https://shop.example.
function world({ stored, host = 'shop.example', queueFrames = false } = {}) {
  const rendered = [];
  const frames = [];
  const observers = [];
  const context = { alive: true };
  let rect = { left: 10, top: 20, width: 100, height: 40 };
  const button = {
    tagName: 'BUTTON', textContent: 'Continue with GitHub', isConnected: true, parentElement: null,
    getAttribute: () => null,
    closest: (selector) => (selector === 'form' ? null : button),
    contains: () => false,
    getBoundingClientRect: () => rect,
  };
  const doc = Object.assign(eventTarget(), {
    querySelectorAll: () => [button],
    body: { tagName: 'BODY' },
    documentElement: { appendChild(node) { rendered.push(node); node.remove = () => rendered.splice(rendered.indexOf(node), 1); } },
    createElement() {
      return {
        style: { cssText: '' }, textContent: '',
        setAttribute() {}, appendChild(child) { return child; }, attachShadow() { return this; },
      };
    },
  });
  const win = Object.assign(eventTarget(), {
    scrollX: 0, scrollY: 0, navigator: {}, getComputedStyle: () => ({ visibility: 'visible', display: 'block' }),
  });
  const changed = { fns: [], addListener(fn) { this.fns.push(fn); }, removeListener(fn) { this.fns = this.fns.filter((f) => f !== fn); } };
  const sent = [];
  const replies = [];
  const env = {
    window: win,
    document: doc,
    location: { href: `https://${host}/login`, hostname: host },
    storage: { get: (keys, callback) => callback(stored === undefined ? {} : { [social.SETTING_KEY]: stored }) },
    storageChanged: changed,
    sendMessage(message, callback) {
      sent.push(message);
      if (message.type === 'getSocialSignIn') callback(replies.shift());
      else callback({ type: 'socialSignIn', enabled: true });
    },
    t: (key, [name]) => `${key}:${name}`,
    now: () => 1_800_000_000_000,
    setTimeout: () => 0,
    requestAnimationFrame: (fn) => (queueFrames ? frames.push(fn) : fn()),
    contextAlive: () => context.alive,
    ResizeObserver: class {
      constructor(callback) { this.callback = callback; this.observed = []; this.connected = true; observers.push(this); }
      observe(node) { this.observed.push(node); }
      disconnect() { this.connected = false; this.observed = []; }
      fire() { this.callback([]); }
    },
  };
  return {
    env, button, rendered, sent, replies, frames, observers, context, doc, win, changed,
    moveTo: (next) => { rect = next; },
    flushFrames: () => frames.splice(0).forEach((fn) => fn()),
    click: () => doc.dispatch('click', { isTrusted: true, target: button }),
    setting: (newValue) => changed.fns.forEach((fn) => fn({ [social.SETTING_KEY]: { newValue } }, 'local')),
    types: () => sent.map((m) => m.type),
  };
}

const remembered = { type: 'socialSignIn', enabled: true, provider: 'github', host: 'shop.example' };

test('the remembered button is ringed and its clicks are reported', () => {
  const w = world({ stored: true });
  w.replies.push(remembered);
  const page = social.startPage(w.env);
  assert.equal(w.rendered.length, 1);
  assert.equal(page.snapshot().ringed, w.button);
  w.click();
  assert.deepEqual(w.sent.at(-1), { type: 'recordSocialSignIn', pageUrl: 'https://shop.example/login', provider: 'github', source: 'click' });
});

test('a method recorded on another host is never ringed', () => {
  const w = world({ stored: true });
  w.replies.push({ ...remembered, host: 'accounts.shop.example' });
  social.startPage(w.env);
  assert.equal(w.rendered.length, 0);
});

test('turned off while the page is open: the ring goes and clicks are no longer reported', () => {
  const w = world({ stored: true });
  w.replies.push(remembered);
  const page = social.startPage(w.env);
  assert.equal(w.rendered.length, 1);
  w.setting(false);
  assert.equal(w.rendered.length, 0);
  assert.equal(page.snapshot().enabled, false);
  const before = w.sent.length;
  w.click();
  w.env.window.dispatch('blur', { isTrusted: true });
  assert.equal(w.sent.length, before, 'nothing is reported while off');
});

test('turned back on while the page is open: the lookup resumes and the ring returns', () => {
  const w = world({ stored: true });
  w.replies.push(remembered);
  social.startPage(w.env);
  w.setting(false);
  w.replies.push(remembered);
  w.setting(true);
  assert.deepEqual(w.types(), ['getSocialSignIn', 'getSocialSignIn']);
  assert.equal(w.rendered.length, 1);
  w.click();
  assert.equal(w.types().at(-1), 'recordSocialSignIn');
  // The same value again is not a transition: no extra lookup.
  w.setting(true);
  assert.equal(w.types().filter((t) => t === 'getSocialSignIn').length, 2);
});

test('a cached "off" still asks on page load, and an "on" answer turns the page on', () => {
  const w = world({ stored: false });
  w.replies.push(remembered);
  const page = social.startPage(w.env);
  assert.deepEqual(w.types(), ['getSocialSignIn'], 'the cached value never skips the lookup');
  assert.equal(page.snapshot().enabled, true);
  assert.equal(w.rendered.length, 1);
});

test('an "off" answer on page load keeps the page quiet', () => {
  const w = world({ stored: true });
  w.replies.push({ type: 'socialSignIn', enabled: false });
  social.startPage(w.env);
  w.click();
  assert.deepEqual(w.types(), ['getSocialSignIn']);
  assert.equal(w.rendered.length, 0);
});

test('the ring follows the button when its layout changes, at most once per frame', () => {
  const w = world({ stored: true, queueFrames: true });
  w.replies.push(remembered);
  social.startPage(w.env);
  const ring = w.rendered[0];
  assert.equal(ring.style.left, '6px');
  assert.equal(w.observers.length, 1);
  const observer = w.observers[0];
  assert.deepEqual(observer.observed, [w.button, w.doc.body]);
  w.moveTo({ left: 50, top: 120, width: 100, height: 40 });
  observer.fire();
  observer.fire();
  observer.fire();
  assert.equal(w.frames.length, 1, 'three resizes in one frame schedule one placement');
  assert.equal(ring.style.left, '6px', 'nothing moves before the frame');
  w.flushFrames();
  assert.equal(ring.style.left, '46px');
  assert.equal(ring.style.top, '116px');
  // Turning the setting off disconnects the observer along with the ring.
  w.setting(false);
  assert.equal(observer.connected, false);
});

test('a lost extension context removes the ring and every listener at the next event', () => {
  const w = world({ stored: true });
  w.replies.push(remembered);
  const page = social.startPage(w.env);
  assert.equal(w.rendered.length, 1);
  assert.ok(w.doc.listeners.length > 0 && w.win.listeners.length > 0);
  w.context.alive = false;
  const before = w.sent.length;
  w.click();
  assert.equal(w.sent.length, before, 'nothing is sent from a dead context');
  assert.equal(w.rendered.length, 0);
  assert.equal(w.observers[0].connected, false);
  assert.equal(w.doc.listeners.length, 0);
  assert.equal(w.win.listeners.length, 0);
  assert.equal(w.changed.fns.length, 0);
  assert.equal(page.snapshot().stopped, true);
});

test('a lost context noticed by a resize or a setting change also tears down', () => {
  for (const trigger of ['resize', 'setting']) {
    const w = world({ stored: true });
    w.replies.push(remembered);
    social.startPage(w.env);
    w.context.alive = false;
    if (trigger === 'resize') w.observers[0].fire();
    else w.setting(true);
    assert.equal(w.rendered.length, 0, trigger);
    assert.equal(w.doc.listeners.length, 0, trigger);
  }
});

test('a send that throws (context invalidated mid-call) stops the page instead of throwing', () => {
  const w = world({ stored: true });
  w.replies.push(remembered);
  const page = social.startPage(w.env);
  w.env.sendMessage = () => { throw new Error('Extension context invalidated.'); };
  assert.doesNotThrow(() => w.click());
  assert.equal(page.snapshot().stopped, true);
  assert.equal(w.rendered.length, 0);
});

test('stop is idempotent and leaves nothing behind', () => {
  const w = world({ stored: true });
  w.replies.push(remembered);
  const page = social.startPage(w.env);
  page.stop();
  page.stop();
  assert.equal(w.rendered.length, 0);
  assert.equal(w.doc.listeners.length + w.win.listeners.length + w.changed.fns.length, 0);
});
