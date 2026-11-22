// The service-worker side of social sign-in: which senders may record or look up, that the page
// url sent on is sender.url, the locked queue in session storage and its flush after unlock, and
// the "Remember social sign-in" mirror in storage.local.
const assert = require("node:assert/strict");
const { beforeEach, test } = require("node:test");

const extensionUrl = (file) => `chrome-extension://test/${file}`;
const popup = { id: "test", url: extensionUrl("popup.html") };
const page = (url, extra = {}) => ({ id: "test", url, tab: { id: 7 }, frameId: 0, documentId: "doc", ...extra });
let nativeCalls, local, session, replies;
global.chrome = { runtime: { id: "test", getURL: extensionUrl, onMessage: { addListener() {} } } };

const background = require("../background.js");
const {
  handleRuntimeMessage, flushSocialSignInQueue, queueSocialSignIn, resetActionIconStateForTests,
  resetSocialSignInStateForTests,
  SOCIAL_SIGN_IN_PENDING_KEY, SOCIAL_SIGN_IN_ENABLED_KEY,
} = background;

function area(store) {
  return {
    get: (keys, callback) => {
      const list = Array.isArray(keys) ? keys : [keys];
      callback(Object.fromEntries(list.filter((k) => k in store).map((k) => [k, store[k]])));
    },
    set: (value, callback) => { Object.assign(store, value); callback && callback(); },
    remove: (keys, callback) => { for (const k of [].concat(keys)) delete store[k]; callback && callback(); },
  };
}

beforeEach(() => {
  nativeCalls = [];
  local = { pairingSecret: "proof" };
  session = {};
  replies = [];
  chrome.runtime.sendNativeMessage = (_, message, callback) => {
    nativeCalls.push(message);
    callback(replies.length ? replies.shift() : { type: "socialSignIn", enabled: true });
  };
  chrome.action = { setIcon: async () => {} };
  chrome.storage = { local: area(local), session: area(session) };
  resetActionIconStateForTests();
  resetSocialSignInStateForTests();
});

function request(message, sender) {
  return new Promise((resolve) => {
    if (!handleRuntimeMessage(message, sender, resolve)) resolve("refused");
  });
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("a page records its own url, not the one the message names", async () => {
  const sender = page("https://shop.example/login?next=/cart");
  const reply = await request({ type: "recordSocialSignIn", pageUrl: "https://shop.example/", provider: "google", source: "click" }, sender);
  assert.deepEqual(reply, { type: "socialSignIn", enabled: true });
  assert.deepEqual(nativeCalls, [{
    type: "recordSocialSignIn", pageUrl: "https://shop.example/login?next=/cart", provider: "google", source: "click", pairing: "proof",
  }]);
  assert.equal(local[SOCIAL_SIGN_IN_ENABLED_KEY], true);
});

test("anything but an explicit click is sent on as the frame heuristic", async () => {
  const sender = page("https://shop.example/");
  for (const source of [undefined, "frame", "script", 1]) {
    await request({ type: "recordSocialSignIn", pageUrl: sender.url, provider: "google", source }, sender);
  }
  assert.deepEqual(nativeCalls.map((m) => m.source), ["frame", "frame", "frame", "frame"]);
});

test("records from the wrong place never reach the app", async () => {
  const good = { type: "recordSocialSignIn", pageUrl: "https://shop.example/", provider: "google" };
  assert.equal(await request(good, popup), "refused", "the popup never records");
  assert.equal(await request(good, { ...page("https://shop.example/"), url: extensionUrl("app/app.html") }), "refused");
  for (const [message, sender] of [
    [good, page("https://shop.example/", { frameId: 3 })],
    [good, page("http://shop.example/")],
    [good, page("https://other.example/")],
    [{ ...good, provider: "twitter" }, page("https://shop.example/")],
  ]) assert.deepEqual(await request(message, sender), { type: "error", message: "invalid_request" });
  assert.deepEqual(nativeCalls, []);
});

test("a locked vault queues the click and the unlock sends it", async () => {
  replies = [{ type: "locked" }];
  const sender = page("https://shop.example/login");
  assert.deepEqual(
    await request({ type: "recordSocialSignIn", pageUrl: sender.url, provider: "github" }, sender),
    { type: "queued" },
  );
  const queued = session[SOCIAL_SIGN_IN_PENDING_KEY];
  assert.deepEqual(Object.keys(queued), ["7|https://shop.example"]);
  assert.equal(queued["7|https://shop.example"].provider, "github");

  // The popup unlocks: the unlocked reply flushes the queue once.
  replies = [{ type: "unlocked" }, { type: "socialSignIn", enabled: true }];
  await request({ type: "unlockWithPassword", password: "x" }, popup);
  await settle();
  await flushSocialSignInQueue();
  assert.deepEqual(nativeCalls.slice(1).map((m) => m.type), ["unlockWithPassword", "recordSocialSignIn"]);
  assert.equal(nativeCalls[2].pageUrl, "https://shop.example/login");
  assert.equal(SOCIAL_SIGN_IN_PENDING_KEY in session, false);
});

test("expired or still-locked records are handled on flush", async () => {
  const now = Date.now();
  session[SOCIAL_SIGN_IN_PENDING_KEY] = {
    "1|https://old.example": { pageUrl: "https://old.example/", provider: "apple", at: now - 11 * 60 * 1000 },
    "2|https://new.example": { pageUrl: "https://new.example/", provider: "apple", at: now - 1000 },
  };
  replies = [{ type: "locked" }];
  await flushSocialSignInQueue();
  assert.deepEqual(nativeCalls.map((m) => m.pageUrl), ["https://new.example/"], "the expired record is dropped");
  assert.deepEqual(Object.keys(session[SOCIAL_SIGN_IN_PENDING_KEY]), ["2|https://new.example"]);
});

test("lookups come from the page or the popup and mirror the setting", async () => {
  replies = [{ type: "socialSignIn", enabled: false }];
  const sender = page("https://shop.example/");
  assert.deepEqual(await request({ type: "getSocialSignIn", url: sender.url }, sender), { type: "socialSignIn", enabled: false });
  assert.equal(local[SOCIAL_SIGN_IN_ENABLED_KEY], false);

  replies = [{ type: "socialSignIn", enabled: true, provider: "google" }];
  const reply = await request({ type: "getSocialSignIn", url: "https://shop.example/" }, popup);
  assert.equal(reply.provider, "google");
  assert.deepEqual(nativeCalls.map((m) => m.url), ["https://shop.example/", "https://shop.example/"]);

  assert.deepEqual(await request({ type: "getSocialSignIn", url: "http://shop.example/" }, popup),
    { type: "error", message: "invalid_request" });
  assert.equal(await request({ type: "getSocialSignIn", url: "https://shop.example/" }, { id: "other", url: "https://x" }), "refused");
  assert.equal(nativeCalls.length, 2);
});

test("a page learns only a method recorded on its own exact host", async () => {
  const sender = page("https://www.shop.example/login");
  replies = [{ type: "socialSignIn", enabled: true, provider: "google", host: "www.shop.example", lastUsed: "2026-11-22T10:00:00Z" }];
  assert.deepEqual(await request({ type: "getSocialSignIn", url: sender.url }, sender),
    { type: "socialSignIn", enabled: true, provider: "google", host: "www.shop.example" });

  // Recorded on a sibling host under the same registrable domain: the page sees no method.
  replies = [{ type: "socialSignIn", enabled: true, provider: "google", host: "accounts.shop.example" }];
  assert.deepEqual(await request({ type: "getSocialSignIn", url: sender.url }, sender), { type: "socialSignIn", enabled: true });

  // Picked in the app (no host): the page sees no method either.
  replies = [{ type: "socialSignIn", enabled: true, provider: "apple" }];
  assert.deepEqual(await request({ type: "getSocialSignIn", url: sender.url }, sender), { type: "socialSignIn", enabled: true });

  // The popup is not the page: it still shows the method recorded elsewhere.
  replies = [{ type: "socialSignIn", enabled: true, provider: "google", host: "accounts.shop.example" }];
  const reply = await request({ type: "getSocialSignIn", url: sender.url }, popup);
  assert.equal(reply.provider, "google");
});

// ---- the pending queue: one promise chain, exact removal, lock transitions -----------------
// Storage whose callbacks land on a later turn, like chrome.storage: an unserialized
// read-modify-write would lose records here.
function asyncArea(store) {
  const later = (fn) => setTimeout(fn, 0);
  return {
    get: (keys, callback) => {
      const list = Array.isArray(keys) ? keys : [keys];
      const snapshot = structuredClone(Object.fromEntries(list.filter((k) => k in store).map((k) => [k, store[k]])));
      later(() => callback(snapshot));
    },
    set: (value, callback) => later(() => { Object.assign(store, structuredClone(value)); callback && callback(); }),
    remove: (keys, callback) => later(() => { for (const k of [].concat(keys)) delete store[k]; callback && callback(); }),
  };
}

test("concurrent clicks while locked are all queued", async () => {
  chrome.storage.session = asyncArea(session);
  replies = Array.from({ length: 6 }, () => ({ type: "locked" }));
  const replied = await Promise.all([1, 2, 3, 4, 5, 6].map((tab) => {
    const sender = page(`https://site${tab}.example/login`, { tab: { id: tab } });
    return request({ type: "recordSocialSignIn", pageUrl: sender.url, provider: "github", source: "click" }, sender);
  }));
  assert.deepEqual(replied.map((r) => r.type), Array(6).fill("queued"));
  assert.deepEqual(Object.keys(session[SOCIAL_SIGN_IN_PENDING_KEY]).sort(),
    [1, 2, 3, 4, 5, 6].map((tab) => `${tab}|https://site${tab}.example`));
});

test("a flush removes only the records it delivered", async () => {
  chrome.storage.session = asyncArea(session);
  const now = Date.now();
  session[SOCIAL_SIGN_IN_PENDING_KEY] = {
    "1|https://a.example": { pageUrl: "https://a.example/", provider: "google", at: now - 1000, source: "click" },
    "2|https://b.example": { pageUrl: "https://b.example/", provider: "apple", at: now - 1000, source: "click" },
  };
  // The app answers slowly; meanwhile tab 1 clicks again (another provider) and tab 3 clicks.
  let release;
  const gate = new Promise((resolve) => { release = resolve; });
  chrome.runtime.sendNativeMessage = (_, message, callback) => {
    nativeCalls.push(message);
    gate.then(() => callback(message.pageUrl === "https://b.example/" ? { type: "error", message: "x" } : { type: "socialSignIn", enabled: true }));
  };
  const flushing = flushSocialSignInQueue();
  await settle(); await settle(); await settle();
  await queueSocialSignIn(1, "https://a.example/again", "github", "click");
  await queueSocialSignIn(3, "https://c.example/", "facebook", "frame");
  release();
  await flushing;
  const left = session[SOCIAL_SIGN_IN_PENDING_KEY];
  assert.deepEqual(Object.keys(left).sort(), ["1|https://a.example", "2|https://b.example", "3|https://c.example"]);
  assert.equal(left["1|https://a.example"].provider, "github", "the newer click of tab 1 survives the flush of the older one");
  assert.equal(left["2|https://b.example"].provider, "apple", "the record the app did not take stays");
  assert.deepEqual(nativeCalls.map((m) => m.pageUrl).sort(), ["https://a.example/", "https://b.example/"]);
});

test("a lock after an unlock clears the queue; a first locked answer keeps it", async () => {
  const now = Date.now();
  const kept = { "1|https://a.example": { pageUrl: "https://a.example/", provider: "google", at: now - 1000, source: "click" } };
  session[SOCIAL_SIGN_IN_PENDING_KEY] = structuredClone(kept);
  // The service worker restarted while the vault was locked: the queue survives.
  replies = [{ type: "locked" }];
  await request({ type: "getSocialSignIn", url: "https://a.example/" }, popup);
  await settle();
  assert.deepEqual(session[SOCIAL_SIGN_IN_PENDING_KEY], kept);

  // Unlocked, then locked again without a flush getting through: the queue is dropped, so a
  // later unlock (possibly of another vault) never replays it.
  replies = [{ type: "unlocked" }, { type: "locked" }];
  await request({ type: "unlockWithPassword", password: "x" }, popup);
  await settle(); await settle();
  assert.equal(SOCIAL_SIGN_IN_PENDING_KEY in session, false);
});

test("queue actions per lock change", () => {
  const { queueActionForLockChange } = require("../shared/social-sign-in.js");
  assert.equal(queueActionForLockChange(null, "unlocked"), "flush");
  assert.equal(queueActionForLockChange("locked", "unlocked"), "flush");
  assert.equal(queueActionForLockChange("unlocked", "unlocked"), "keep");
  assert.equal(queueActionForLockChange(null, "locked"), "keep");
  assert.equal(queueActionForLockChange("locked", "locked"), "keep");
  assert.equal(queueActionForLockChange("unlocked", "locked"), "clear");
});
