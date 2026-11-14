// The standalone prompt window (prompt.html). Opened by background.js when the toolbar popup
// cannot open (a toolbarless window, or openPopup refused). It is NOT popup.html: that page finds
// "its" tab with currentWindow, and this window is a window of its own, so it would look up the
// wrong tab. Here the background hands the prompt over by id and by recency, and the record it
// returns is already bound to the tab, frame and document that raised it.

function createPromptPage(env) {
  const doc = env.document;
  const chromeApi = env.chrome;
  const root = doc.getElementById('prompt-card-root');
  const promptCard = env.promptCard;
  const autofillSettings = env.autofillSettings;
  const wantedId = env.promptId || null;
  let controller = null;
  let loadRevision = 0;

  const api = {
    send: (message, cb) => chromeApi.runtime.sendMessage(message, (response) => {
      void chromeApi.runtime.lastError;
      cb(response);
    }),
    settings: promptCard.chromeSettingsStore(chromeApi, autofillSettings),
    setTimer: (fn, ms) => env.setTimeout(fn, ms),
    clearTimer: (id) => env.clearTimeout(id),
    now: () => Date.now(),
    done: () => load(true),
    reload: () => load(false)
  };

  function showMessage(text) {
    if (controller) { controller.dispose(); controller = null; }
    while (root.firstChild) root.removeChild(root.firstChild);
    const p = doc.createElement('p');
    p.className = 'panel-helper';
    p.textContent = text;
    root.appendChild(p);
  }

  // closeWhenEmpty: after an answer, an empty queue closes the window; on first load and on
  // "Try again" it says so instead, because the user has not answered anything yet.
  function load(closeWhenEmpty) {
    const revision = ++loadRevision;
    chromeApi.runtime.sendMessage({ type: 'getPrompt', promptId: wantedId }, (response) => {
      void chromeApi.runtime.lastError;
      if (revision !== loadRevision) return;
      if (response && response.type === 'prompt') {
        if (controller) controller.dispose();
        controller = promptCard.render(doc, root, response.prompt, api);
        return;
      }
      if (closeWhenEmpty) { env.closeWindow(); return; }
      showMessage('Nothing to confirm. This prompt may have expired; trigger it on the page again.');
    });
  }

  // A prompt added, replaced or removed elsewhere (another tab, expiry, tab closed). A card in
  // the middle of a save is left alone.
  function onSessionChange(changes, areaName) {
    if (areaName !== 'session' || !changes.pendingPrompts) return;
    if (controller && controller.busy()) return;
    const list = changes.pendingPrompts.newValue;
    if (controller && Array.isArray(list) && list.some((r) => r && r.id === controller.id)) return;
    load(true);
  }
  if (chromeApi.storage && chromeApi.storage.onChanged) chromeApi.storage.onChanged.addListener(onSessionChange);

  load(false);
  return { load, onSessionChange };
}

if (typeof module !== 'undefined') {
  module.exports = { createPromptPage };
}
if (typeof window !== 'undefined' && typeof module === 'undefined') {
  createPromptPage({
    document,
    chrome,
    promptCard: mypassPromptCard,
    autofillSettings: mypassAutofillSettings,
    promptId: new URLSearchParams(window.location.search).get('prompt'),
    closeWindow: () => window.close(),
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (id) => window.clearTimeout(id)
  });
}
