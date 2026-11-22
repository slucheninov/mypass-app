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
  handleRuntimeMessage, flushSocialSignInQueue, resetActionIconStateForTests,
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
