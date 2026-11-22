// Test-only DOM and clock for exercising the popup's real classic scripts.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const chromeI18n = require('./chromeI18nShim.js');

const root = path.join(__dirname, '..');
const html = fs.readFileSync(path.join(root, 'popup.html'), 'utf8');
const flush = () => new Promise((resolve) => setImmediate(resolve));

function openPopup({ reply, startedAt, now = 200_000 } = {}) {
  const elements = new Map();
  const listeners = {};
  const storageListeners = [];
  const timers = new Map();
  const storage = startedAt == null ? {} : { pairingStartedAt: startedAt };
  const sent = [];
  let timerId = 0;
  let focused = null;
  let closed = 0;
  const windowsCreated = [];

  function element(id) {
    return {
      id, hidden: false, disabled: false, value: '', textContent: '', className: '',
      style: {}, dataset: {}, children: [], events: {},
      appendChild(child) { this.children.push(child); },
      addEventListener(name, fn) { (this.events[name] ??= []).push(fn); },
      setAttribute(name, value) { this[name] = value; },
      getAttribute(name) { return this[name]; },
      focus() { focused = this.id; },
      select() {},
      contains(node) { return this === node || this.children.includes(node); },
      set innerHTML(value) { this.children = []; this.markup = value; },
      get innerHTML() { return this.markup || ''; },
    };
  }

  for (const [, tag, id] of html.matchAll(/<([^<>]*\bid="([^"]+)"[^<>]*)>/g)) {
    const node = element(id);
    node.hidden = /\bhidden(?:\s|>|$)/.test(tag);
    node.type = tag.match(/\btype="([^"]+)"/)?.[1];
    elements.set(id, node);
  }
  const document = {
    getElementById(id) { return elements.get(id) || null; },
    createElement: element,
    addEventListener() {},
  };
  const local = {
    get: (_, cb) => cb({}),
    set: (_, cb) => cb?.(),
    remove: (_, cb) => cb?.(),
  };
  const session = {
    ...local,
    get: (_, cb) => cb(storage),
    set: (values, cb) => { Object.assign(storage, values); cb?.(); },
    remove: (key, cb) => { delete storage[key]; cb?.(); },
  };
  const chrome = chromeI18n.install({
    storage: { local, session, onChanged: { addListener: (fn) => storageListeners.push(fn) } },
    runtime: {
      getURL: (relative) => 'chrome-extension://extension-id/' + relative,
      sendMessage(message, cb) {
        sent.push({ ...message });
        const result = reply?.(message, cb);
        if (result !== undefined) cb?.(result);
      },
    },
    tabs: { query: (_, cb) => cb([{ url: 'https://example.com/' }]) },
    windows: { create: (options, cb) => { windowsCreated.push(options.url); cb?.(); } },
  });
  function schedule(fn, delay, repeat) {
    const id = ++timerId;
    timers.set(id, { fn, at: now + delay, interval: repeat ? delay : 0 });
    return id;
  }
  const context = vm.createContext({
    document, chrome, console, URL, URLSearchParams, TextEncoder,
    Date: class extends Date { static now() { return now; } },
    setTimeout: (fn, delay) => schedule(fn, delay, false),
    setInterval: (fn, delay) => schedule(fn, delay, true),
    clearTimeout: (id) => timers.delete(id),
    clearInterval: (id) => timers.delete(id),
    addEventListener: (name, cb) => { listeners[name] = cb; },
    close: () => { closed += 1; },
  });
  context.window = context;

  for (const [, relative] of html.matchAll(/<script\s+src="([^"]+)"><\/script>/g)) {
    if (relative === 'popup.js') context.module = { exports: {} };
    vm.runInContext(fs.readFileSync(path.join(root, relative), 'utf8'), context, { filename: relative });
  }

  async function advance(ms) {
    const end = now + ms;
    await flush();
    while (true) {
      const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!next) break;
      const [id, timer] = next;
      now = timer.at;
      if (timer.interval) timer.at += timer.interval;
      else timers.delete(id);
      timer.fn();
      await flush();
    }
    now = end;
  }

  const el = (id) => document.getElementById(id);
  function fire(id, name) {
    const node = el(id);
    const event = { preventDefault() {}, stopPropagation() {}, target: node };
    node[`on${name}`]?.(event);
    for (const fn of node.events[name] || []) fn(event);
  }
  return {
    el, fire, advance, sent, storage, listeners, timers,
    notifyStorage: (changes, area) => storageListeners.forEach((fn) => fn(changes, area)),
    focused: () => focused, closed: () => closed, windowsCreated, api: context.module.exports,
  };
}

module.exports = { openPopup, html, flush };
