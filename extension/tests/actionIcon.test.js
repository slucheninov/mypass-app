const assert = require("node:assert/strict");
const { beforeEach, test } = require("node:test");

const extensionUrl = (file) => `chrome-extension://test/${file}`;
const popup = { id: "test", url: extensionUrl("popup.html") };
const tab = { ...popup, tab: { id: 1 } };
let nativeCalls, storageWrites, icons, reply;
global.chrome = { runtime: { id: "test", getURL: extensionUrl, onMessage: { addListener() {} } } };

const background = require("../background.js");
const {
  ACTION_ICON_PATHS, iconStateForResponse, applyActionIcon, resetActionIconStateForTests,
  handleRuntimeMessage, attachPasskeyProxyIfEligible, PASSKEY_REFRESH_ALARM,
} = background;

beforeEach(() => {
  nativeCalls = [];
  storageWrites = [];
  icons = [];
  reply = { type: "locked" };
  chrome.runtime.sendNativeMessage = (_, message, callback) => {
    nativeCalls.push(message);
    callback(reply);
  };
  chrome.action = { setIcon: async (options) => { icons.push(options.path); } };
  chrome.storage = { local: {
    get: (_, callback) => callback({ pairingSecret: "proof" }),
    set: (value, callback) => { storageWrites.push(value); callback(); },
    remove: (_, callback) => callback(),
  } };
  resetActionIconStateForTests?.();
});

function request(message, sender = popup) {
  return new Promise((resolve) => {
    if (!handleRuntimeMessage(message, sender, resolve)) resolve("refused");
  });
}

test("response icon classification", () => {
  for (const response of [
    { type: "locked" }, { type: "pairingRequired" },
    { type: "error", message: "app_not_running" },
    { type: "error", code: "connection_unavailable" },
    { type: "error", code: "native_host_not_found" },
    { type: "passkeyProxyStatus", unlocked: false, paired: true },
    { type: "passkeyProxyStatus", unlocked: true, paired: false },
  ]) assert.equal(iconStateForResponse(response), "locked");
  for (const response of [
    { type: "unlocked" }, { type: "unlockNotNeeded" }, { type: "credentialList" },
    { type: "passkeyProxyStatus", unlocked: true, paired: true },
  ]) assert.equal(iconStateForResponse(response), "unlocked");
  for (const response of [
    { type: "wrongPassword" }, { type: "totp" }, { type: "error", message: "unlock_failed" }, undefined,
  ]) assert.equal(iconStateForResponse(response), null);
});

test("repeated locked states replace the action icon once", async () => {
  await applyActionIcon("locked");
  await applyActionIcon("locked");
  assert.deepEqual(icons, [ACTION_ICON_PATHS.locked]);
  chrome.action.setIcon = async () => { throw new Error("toolbar gone"); };
  await applyActionIcon("unlocked");
});

test("a native locked reply switches the icon through the paired message path", async () => {
  assert.deepEqual(await request({ type: "getUnlockOptions" }), reply);
  assert.deepEqual(icons, [ACTION_ICON_PATHS.locked]);
});

test("the passkey refresh switches the icon to unlocked when unlocked and paired", async () => {
  reply = { type: "passkeyProxyStatus", unlocked: true, paired: true };
  await attachPasskeyProxyIfEligible();
  assert.deepEqual(icons, [ACTION_ICON_PATHS.unlocked]);
});

test("a successful lock updates the icon immediately", async () => {
  reply = { type: "ok" };
  assert.deepEqual(await request({ type: "lockVault" }), reply);
  assert.deepEqual(icons, [ACTION_ICON_PATHS.locked]);
});

test("unlock and launch requests are refused from tab senders and other pages", async () => {
  for (const type of ["unlockWithPassword", "unlockWithDevice", "getUnlockOptions", "launchApp"]) {
    assert.equal(await request({ type }, tab), "refused");
    assert.equal(await request({ type }, { ...popup, url: extensionUrl("app/app.html") }), "refused");
  }
  assert.deepEqual(nativeCalls, []);
});

test("launchApp from the popup is passed through without pairing or extra fields", async () => {
  reply = { type: "launchStarted" };
  assert.deepEqual(await request({ type: "launchApp", pairing: "ignored" }), reply);
  assert.deepEqual(nativeCalls, [{ type: "launchApp" }]);
});

test("unlockWithPassword is sent with pairing without writing the password to storage", async () => {
  reply = { type: "unlocked" };
  const password = "test-password";
  assert.deepEqual(await request({ type: "unlockWithPassword", password }), reply);
  assert.deepEqual(nativeCalls, [{ type: "unlockWithPassword", password, pairing: "proof" }]);
  assert.ok(!JSON.stringify(storageWrites).includes(password));
  assert.deepEqual(icons, [ACTION_ICON_PATHS.unlocked]);
});

test("worker startup, install and the status alarm update the icon without a popup or passkey proxy", async () => {
  const startup = [], installed = [], alarms = [], created = [];
  const callbacks = [];
  chrome.runtime.onStartup = { addListener: (listener) => startup.push(listener) };
  chrome.runtime.onInstalled = { addListener: (listener) => installed.push(listener) };
  chrome.alarms = {
    onAlarm: { addListener: (listener) => alarms.push(listener) },
    get: async () => undefined,
    create: (name, options) => created.push([name, options]),
  };
  chrome.runtime.sendNativeMessage = (_, message, callback) => {
    nativeCalls.push(message);
    callbacks.push(callback);
  };
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  const worker = require.resolve("../background.js");
  delete require.cache[worker];
  try {
    require(worker);
    await settle();
    assert.deepEqual(icons, [ACTION_ICON_PATHS.locked]);
    assert.deepEqual(nativeCalls, [{ type: "getPasskeyProxyStatus", pairing: "proof" }]);
    assert.deepEqual(created, [[PASSKEY_REFRESH_ALARM, { periodInMinutes: 0.5 }]]);

    callbacks.shift()({ type: "passkeyProxyStatus", unlocked: true, paired: true });
    await settle();
    for (const listener of alarms) listener({ name: PASSKEY_REFRESH_ALARM });
    await settle();
    callbacks.shift()({ type: "passkeyProxyStatus", unlocked: false, paired: true });
    await settle();

    assert.equal(startup.length, 1);
    startup[0]();
    await settle();
    callbacks.shift()({ type: "passkeyProxyStatus", unlocked: true, paired: true });
    await settle();
    assert.equal(installed.length, 1);
    installed[0]();
    await settle();
    callbacks.shift()({ type: "error", message: "app_not_running" });
    await settle();

    assert.deepEqual(icons, [
      ACTION_ICON_PATHS.locked, ACTION_ICON_PATHS.unlocked, ACTION_ICON_PATHS.locked,
      ACTION_ICON_PATHS.unlocked, ACTION_ICON_PATHS.locked,
    ]);
  } finally {
    delete chrome.runtime.onStartup;
    delete chrome.runtime.onInstalled;
    delete chrome.alarms;
  }
});
