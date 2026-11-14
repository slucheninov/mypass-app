const assert = require('node:assert');

const elements = {};
function makeEl(id) {
  const el = {
    id,
    hidden: false,
    textContent: '',
    innerHTML: '',
    value: '',
    onclick: null,
    className: '',
    style: {},
    children: [],
    appendChild(child) {
      this.children.push(child);
    },
    addEventListener() {},
    setAttribute(name, value) {
      this[name] = value;
    }
  };
  elements[id] = el;
  return el;
}

[
  'locked-panel',
  'locked-title',
  'locked-helper',
  'locked-retry-btn',
  'app-unavailable-panel',
  'app-unavailable-status',
  'app-unavailable-actions',
  'retry-btn',
  'open-mypass-btn',
  'open-mypass-helper',
  'empty-panel',
  'empty-title',
  'list-panel',
  'list-header',
  'list',
  'pairing-panel',
  'pairing-title',
  'connect-btn',
  'code-entry',
  'code-input',
  'submit-code-btn',
  'pairing-error'
].forEach((id) => makeEl(id));

const documentListeners = [];
const documentRemovedListeners = [];

global.document = {
  getElementById: (id) => {
    if (!elements[id]) makeEl(id);
    return elements[id];
  },
  createElement: (tag) => makeEl(tag),
  // buildTotpCircleElement draws the countdown ring with SVG nodes; the fake
  // elements from makeEl (children/appendChild/setAttribute/style) are good
  // enough shape for that non-interactive construction to succeed.
  createElementNS: (_ns, tag) => makeEl(tag),
  addEventListener: (type, handler, capture) => {
    documentListeners.push({ type, handler, capture });
  },
  removeEventListener: (type, handler, capture) => {
    documentRemovedListeners.push({ type, handler, capture });
  }
};

let sendMessageCall = 0;
global.chrome = {
  storage: {
    local: {
      get: (_keys, cb) => cb({}),
      set: (_items, cb) => cb && cb(),
      remove: (_keys, cb) => cb && cb()
    },
    session: null
  },
  runtime: {
    // Popup init also fires getPasskeyProxyStatus (answered generically below) and the
    // fire-and-forget refreshPasskeyProxy (no callback at all) — only getCredentials
    // calls are counted here, matching popup-fallback.test.js's stub shape.
    sendMessage: (msg, cb) => {
      if (!msg || msg.type !== 'getCredentials') {
        if (cb) cb({ type: 'passkeyProxyStatus', unlocked: true, paired: true, attachAllowed: false, enabled: false });
        return;
      }
      sendMessageCall += 1;
      if (sendMessageCall === 1) {
        cb({ type: 'error', message: 'native_host_not_found' });
        return;
      }
      if (sendMessageCall === 2) {
        cb({
          type: 'credentialList',
          credentials: [{ id: '1', title: 'Example', username: 'alice@example.com' }]
        });
      }
    }
  },
  tabs: {
    query: (_opts, cb) => cb([{ url: 'https://example.com' }])
  }
};

function makeFakeCircleElement(options = {}) {
  const classes = new Set();
  const listeners = {};
  return {
    isConnected: options.isConnected !== undefined ? options.isConnected : true,
    classList: {
      add(...names) { names.forEach((n) => classes.add(n)); },
      remove(...names) { names.forEach((n) => classes.delete(n)); },
      contains(name) { return classes.has(name); },
    },
    textContent: '',
    addEventListener(type, handler) {
      listeners[type] = listeners[type] || [];
      listeners[type].push(handler);
    },
    dispatch(type, event) {
      (listeners[type] || []).forEach((handler) => handler(event));
    },
    contains() { return false; },
  };
}

function lastDocumentClickListener() {
  for (let i = documentListeners.length - 1; i >= 0; i -= 1) {
    if (documentListeners[i].type === 'click') return documentListeners[i];
  }
  return undefined;
}

const {
  shouldShowTotpCircle,
  createTotpCircleController,
  buildTotpCircleElement,
  totpRemainingFraction,
  totpRingDashOffset,
  tickTotpRings,
  registerTotpRing,
  TOTP_RING_CIRCUMFERENCE
} = require('../popup.js');

assert.strictEqual(shouldShowTotpCircle(true), true);
assert.strictEqual(shouldShowTotpCircle(false), false);
assert.strictEqual(shouldShowTotpCircle(undefined), false);
console.log('totpCircle.test.js: shouldShowTotpCircle OK');

// --- totpRemainingFraction: remaining fraction of the current 30 s window. ---
const PERIOD = 30;
assert.strictEqual(totpRemainingFraction(5 * PERIOD * 1000, PERIOD), 1, 'exactly at a window boundary the ring is full');
assert.ok(
  Math.abs(totpRemainingFraction(5 * PERIOD * 1000 + 29000, PERIOD) - 1 / 30) < 1e-9,
  '1 s before the next window, only 1/30th of the ring remains'
);
{
  // Monotonically decreasing across the whole window.
  let previous = totpRemainingFraction(5 * PERIOD * 1000, PERIOD);
  for (let ms = 1000; ms < PERIOD * 1000; ms += 1000) {
    const fraction = totpRemainingFraction(5 * PERIOD * 1000 + ms, PERIOD);
    assert.ok(fraction < previous, 'fraction must strictly decrease as time advances within a window');
    previous = fraction;
  }
}
console.log('totpCircle.test.js: totpRemainingFraction OK');

// --- totpRingDashOffset: maps a remaining fraction onto the SVG dash offset. ---
assert.strictEqual(totpRingDashOffset(1, TOTP_RING_CIRCUMFERENCE), 0, 'full remaining fraction draws the whole ring (offset 0)');
assert.strictEqual(totpRingDashOffset(0.5, TOTP_RING_CIRCUMFERENCE), TOTP_RING_CIRCUMFERENCE / 2);
console.log('totpCircle.test.js: totpRingDashOffset OK');

// --- buildTotpCircleElement: draws the ring as a non-interactive SVG child,
// keeping the digits in a separate label so reveal/hide never wipes the ring. ---
{
  const el = buildTotpCircleElement({ id: 'ring-1' }, 'https://example.com');
  const svgChild = el.children.find((child) => child.class === 'totp-ring');
  assert.ok(svgChild, 'buildTotpCircleElement must append the countdown ring SVG');
  const ringCircle = svgChild.children[0];
  assert.ok(ringCircle, 'the ring SVG must contain a circle element');
  assert.strictEqual(ringCircle['stroke-dasharray'], String(TOTP_RING_CIRCUMFERENCE));
  const labelChild = el.children.find((child) => child.className === 'totp-circle-label');
  assert.ok(labelChild, 'buildTotpCircleElement must append a separate label element for the digits');

  // buildTotpCircleElement also registers this circle with the shared ticker;
  // the fake element here has no classList, so sweep it out immediately
  // (mirroring the isConnected self-clean) before any later tick would touch it.
  el.isConnected = false;
  tickTotpRings(Date.now());
}
console.log('totpCircle.test.js: buildTotpCircleElement ring structure OK');

(async () => {
  const circle = makeFakeCircleElement();
  let requestCount = 0;
  const requestTotp = () => {
    requestCount += 1;
    return Promise.resolve('123456');
  };
  createTotpCircleController(circle, requestTotp);

  let stopped = false;
  circle.dispatch('click', { stopPropagation: () => { stopped = true; } });
  await Promise.resolve();
  await Promise.resolve();

  assert.strictEqual(stopped, true, 'circle click must stopPropagation so it never bubbles to the row click handler');
  assert.strictEqual(circle.textContent, '123456', 'left-click reveals the code');
  assert.strictEqual(requestCount, 1);

  circle.dispatch('click', { stopPropagation: () => {} });
  assert.strictEqual(circle.textContent, '', 'second left-click hides the code');

  console.log('totpCircle.test.js: createTotpCircleController reveal/hide OK');

  // --- (a) copied flash dismissal: a second left-click during the "Copied" flash
  // must hide immediately, without issuing a new getTotp request. ---
  // Node ships a read-only, getter-only `navigator` global (no setter) — plain
  // assignment silently no-ops, so redefine the property to install the stub.
  Object.defineProperty(global, 'navigator', {
    value: { clipboard: { writeText: () => Promise.resolve() } },
    configurable: true,
    writable: true
  });

  const copyCircle = makeFakeCircleElement();
  let copyRequestCount = 0;
  const copyRequestTotp = () => {
    copyRequestCount += 1;
    return Promise.resolve('654321');
  };
  createTotpCircleController(copyCircle, copyRequestTotp);

  copyCircle.dispatch('contextmenu', { preventDefault: () => {}, stopPropagation: () => {} });
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();

  assert.strictEqual(copyCircle.textContent, 'Copied', 'right-click copies and shows the flash');
  assert.strictEqual(copyRequestCount, 1);

  copyCircle.dispatch('click', { stopPropagation: () => {} });
  assert.strictEqual(copyCircle.textContent, '', 'second left-click during copied flash hides, not re-reveals');
  assert.strictEqual(copyRequestCount, 1, 'second left-click during copied flash must not issue a new getTotp request');

  console.log('totpCircle.test.js: copied flash second-click dismissal OK');

  // --- (b) outside-click handler is registered in the capture phase, and hides a
  // revealed circle when the click target is outside it. ---
  const outsideCircle = makeFakeCircleElement();
  const outsideRequestTotp = () => Promise.resolve('111222');
  createTotpCircleController(outsideCircle, outsideRequestTotp);

  const outsideListener = lastDocumentClickListener();
  assert.ok(outsideListener, 'createTotpCircleController must register a document click listener');
  assert.strictEqual(outsideListener.capture, true, 'outside-click listener must be registered on the capture phase');

  outsideCircle.dispatch('click', { stopPropagation: () => {} });
  await Promise.resolve();
  await Promise.resolve();
  assert.strictEqual(outsideCircle.textContent, '111222', 'left-click reveals before the outside-click check');

  outsideListener.handler({ target: {} });
  assert.strictEqual(outsideCircle.textContent, '', 'invoking the captured outside-click handler with a foreign target hides the revealed code');

  console.log('totpCircle.test.js: outside-click capture-phase dismissal OK');

  // --- (c) self-clean: once the circle is no longer in the DOM, the outside-click
  // handler removes itself instead of accumulating across re-renders. ---
  const disconnectedCircle = makeFakeCircleElement({ isConnected: false });
  const disconnectedRequestTotp = () => Promise.resolve('999000');
  createTotpCircleController(disconnectedCircle, disconnectedRequestTotp);

  const disconnectedListener = lastDocumentClickListener();
  assert.ok(disconnectedListener, 'createTotpCircleController must register a document click listener');

  const removedBefore = documentRemovedListeners.length;
  disconnectedListener.handler({ target: {} });
  assert.strictEqual(documentRemovedListeners.length, removedBefore + 1, 'a disconnected circle must remove its own outside-click listener');
  const removedCall = documentRemovedListeners[documentRemovedListeners.length - 1];
  assert.strictEqual(removedCall.type, 'click');
  assert.strictEqual(removedCall.handler, disconnectedListener.handler);
  assert.strictEqual(removedCall.capture, true);
  assert.strictEqual(disconnectedCircle.textContent, '', 'a disconnected circle must not reveal/hide as a side effect of self-cleaning');

  console.log('totpCircle.test.js: stale listener self-clean on disconnect OK');

  // --- (d) rollover refresh: while digits are revealed, a live window rollover
  // must silently re-request the code in place, without resetting the auto-hide
  // timer. Driven directly via the exported tick/register seam with fabricated
  // times — no real timers, so the process still exits immediately. ---
  const rolloverHost = makeFakeCircleElement();
  const rolloverLabel = { textContent: '' };
  const rolloverRing = { style: {} };
  let rolloverRequestCount = 0;
  const rolloverRequestTotp = () => {
    rolloverRequestCount += 1;
    return Promise.resolve(`code-${rolloverRequestCount}`);
  };
  createTotpCircleController(rolloverHost, rolloverRequestTotp, { textEl: rolloverLabel });

  rolloverHost.dispatch('click', { stopPropagation: () => {} });
  await Promise.resolve();
  await Promise.resolve();
  assert.strictEqual(rolloverLabel.textContent, 'code-1', 'reveal sets the injected textEl, not the host');
  assert.strictEqual(rolloverRequestCount, 1);

  registerTotpRing({
    hostEl: rolloverHost,
    textEl: rolloverLabel,
    svgCircleEl: rolloverRing,
    circumference: TOTP_RING_CIRCUMFERENCE,
    requestTotp: rolloverRequestTotp
  }, 0);

  tickTotpRings(0);
  assert.strictEqual(rolloverRequestCount, 1, 'ticking within the same window must not re-request');

  tickTotpRings(30000);
  await Promise.resolve();
  await Promise.resolve();
  assert.strictEqual(rolloverRequestCount, 2, 'a window rollover while revealed must re-request exactly once');
  assert.strictEqual(rolloverLabel.textContent, 'code-2', 'the refreshed code replaces the stale digits in place');

  console.log('totpCircle.test.js: rollover refresh while revealed OK');

  // --- (e) rollover refresh must NOT fire during the copied flash. ---
  const copiedHost = makeFakeCircleElement();
  const copiedLabel = { textContent: '' };
  const copiedRing = { style: {} };
  let copiedRequestCount = 0;
  const copiedRequestTotp = () => {
    copiedRequestCount += 1;
    return Promise.resolve(`copy-${copiedRequestCount}`);
  };
  createTotpCircleController(copiedHost, copiedRequestTotp, { textEl: copiedLabel });

  copiedHost.dispatch('contextmenu', { preventDefault: () => {}, stopPropagation: () => {} });
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  assert.strictEqual(copiedLabel.textContent, 'Copied');
  assert.strictEqual(copiedRequestCount, 1);

  registerTotpRing({
    hostEl: copiedHost,
    textEl: copiedLabel,
    svgCircleEl: copiedRing,
    circumference: TOTP_RING_CIRCUMFERENCE,
    requestTotp: copiedRequestTotp
  }, 0);

  tickTotpRings(30000);
  await Promise.resolve();
  await Promise.resolve();
  assert.strictEqual(copiedRequestCount, 1, 'the copied flash must not re-request on rollover');
  assert.strictEqual(copiedLabel.textContent, 'Copied', 'the copied flash text must be untouched by rollover');

  console.log('totpCircle.test.js: rollover refresh suppressed during copied flash OK');

  // Clean up the shared ticker: mark every still-registered host disconnected
  // and tick once more so the registry empties and the interval self-clears —
  // otherwise the shared 1 Hz ticker would keep the node process alive.
  rolloverHost.isConnected = false;
  copiedHost.isConnected = false;
  tickTotpRings(60000);
  console.log('totpCircle.test.js: shared ring ticker self-cleans OK');
})();
