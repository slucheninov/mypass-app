// Renders one pending prompt (save login, save 2FA, autofill settings) into a container. Shared by
// popup.html (toolbar popup) and prompt.html (the window used when the toolbar popup cannot open),
// so both surfaces look and behave the same.
//
// The card only ever sees mypassPromptRecord.redactedView(): a username, a site, an account
// label. The password or seed stays in the background; Save sends `{ promptId, action }` (plus an
// entryId the user picked) and nothing else. DOM is built with createElement/textContent only,
// never innerHTML, because every string here ultimately came from a web page.
//
// One var bag. A top-level const or function here would share the classic-script scope with the
// other extension scripts and can throw at load.

var mypassPromptCard = (function () {
  var i18n = typeof require === 'function' ? require('./i18n.js') : mypassI18n;

  var SETTING_ROWS = [
    { name: 'offerSaveLogin', labelKey: 'promptOfferSaveLogin' },
    { name: 'offerSaveTotp', labelKey: 'promptOfferSaveTotp' },
    { name: 'showFieldIcon', labelKey: 'promptShowFieldIcon' },
    { name: 'showLogins', labelKey: 'promptShowLogins' },
    { name: 'showCards', labelKey: 'promptShowCards' },
    { name: 'showAddresses', labelKey: 'promptShowAddresses' },
    { name: 'autoSubmit', labelKey: 'promptAutoSubmit' }
  ];

  function el(doc, tag, className, text) {
    var node = doc.createElement(tag);
    if (className) node.className = className;
    if (typeof text === 'string') node.textContent = text;
    return node;
  }

  function clear(node) {
    if (typeof node.replaceChildren === 'function') { node.replaceChildren(); return; }
    while (node.firstChild) node.removeChild(node.firstChild);
  }

  function field(doc, list, name, value) {
    if (!value) return;
    list.appendChild(el(doc, 'dt', '', name));
    list.appendChild(el(doc, 'dd', '', value));
  }

  // The switch that turns this kind of prompt off. It is the SAME setting the popup's Autofill
  // settings use (offerSaveLogin / offerSaveTotp): one source of truth, no second copy.
  function settingToggle(doc, api, name, label, onChange) {
    var row = el(doc, 'label', 'pc-toggle');
    var input = el(doc, 'input');
    input.type = 'checkbox';
    input.checked = true;
    input.disabled = true;
    row.appendChild(input);
    row.appendChild(el(doc, 'span', '', label));
    if (api.settings) {
      api.settings.read(function (settings) {
        input.checked = settings[name] !== false;
        input.disabled = false;
      });
      input.addEventListener('change', function () {
        api.settings.update(name, !!input.checked);
        if (onChange) onChange(!!input.checked);
      });
    }
    return row;
  }

  function messageFor(response) {
    if (!response) return i18n.t('couldntReachMyPassSentence');
    if (response.type === 'locked') return i18n.t('promptLocked');
    if (response.type === 'pairingRequired') return i18n.t('promptNotPaired');
    if (response.code === 'prompt_gone') return i18n.t('promptExpired');
    if (response.code === 'prompt_page_changed') return i18n.t('promptPageChanged');
    if (typeof response.message === 'string' && response.message) return response.message;
    return i18n.t('couldntReachMyPassSentence');
  }

  // Returns a controller: { id, busy() }. The popup consults busy() so a storage change that
  // arrives while a save is in flight does not tear the card down under the user.
  function render(doc, container, view, api) {
    clear(container);
    var controller = { id: view && view.id, kind: view && view.kind, _busy: false, busy: function () { return this._busy; }, dispose: function () {
      if (this._timer !== undefined && api.clearTimer) api.clearTimer(this._timer);
    } };
    var card = el(doc, 'div', 'pc-card');
    container.appendChild(card);

    var errorEl = el(doc, 'p', 'pc-error');
    errorEl.hidden = true;
    errorEl.setAttribute('role', 'alert');
    var saveBtn = null;
    var notNowBtn = null;
    var chosenEntry = null;
    var accounts = Array.isArray(view.accounts) ? view.accounts : [];

    function showError(text) {
      errorEl.textContent = text;
      errorEl.hidden = !text;
    }

    function setBusy(value) {
      controller._busy = value;
      if (saveBtn) saveBtn.disabled = value || (view.kind === 'saveTotp' && view.state === 'ready' && accounts.length > 1 && chosenEntry === null);
      if (notNowBtn) notNowBtn.disabled = value;
    }

    function answer(action) {
      if (controller._busy) return;
      // Several candidate accounts: never guess one for the user.
      if (action === 'save' && view.kind === 'saveTotp' && accounts.length > 1 && chosenEntry === null) return;
      showError('');
      setBusy(true);
      var message = { type: 'consumePrompt', promptId: view.id, action: action };
      if (action === 'save' && chosenEntry !== null) message.entryId = chosenEntry;
      api.send(message, function (response) {
        setBusy(false);
        var type = response && response.type;
        if (action === 'dismiss' || type === 'ok' || type === 'saveProposalAccepted') {
          api.done();
          return;
        }
        showError(messageFor(response));
        var retryable = type === 'locked' || type === 'pairingRequired' || (response && response.code === 'prompt_choice_required');
        if (!retryable && response && (response.code === 'prompt_gone' || response.code === 'prompt_page_changed')) {
          // Nothing left to retry: offer only a way out.
          if (saveBtn) saveBtn.hidden = true;
          if (notNowBtn) notNowBtn.textContent = i18n.t('close');
        }
      });
    }

    if (view.kind === 'settings') {
      card.appendChild(el(doc, 'h1', 'panel-title', i18n.t('promptAutofillSettings')));
      var list = el(doc, 'div', 'pc-toggles');
      SETTING_ROWS.forEach(function (row) {
        list.appendChild(settingToggle(doc, api, row.name, i18n.t(row.labelKey)));
      });
      card.appendChild(list);
      var done = el(doc, 'button', 'btn-primary', i18n.t('done'));
      done.type = 'button';
      done.addEventListener('click', function () { answer('dismiss'); });
      var buttons = el(doc, 'div', 'btn-row');
      buttons.appendChild(done);
      card.appendChild(buttons);
      return controller;
    }

    var isLogin = view.kind === 'saveLogin';
    card.appendChild(el(doc, 'h1', 'panel-title', i18n.t(isLogin ? 'promptSavePassword' : 'promptAdd2fa')));
    var details = el(doc, 'dl', 'passkey-fields pc-fields');
    field(doc, details, i18n.t('promptSite'), view.origin);
    if (isLogin) {
      field(doc, details, i18n.t('username'), view.username || i18n.t('promptNone'));
      field(doc, details, i18n.t('password'), '••••••••');
    } else {
      field(doc, details, i18n.t('detailAccount'), view.label);
    }
    card.appendChild(details);

    var state = isLogin ? 'ready' : view.state;
    if (!isLogin && state === 'ready') {
      if (accounts.length === 0) {
        card.appendChild(el(doc, 'p', 'panel-helper', i18n.t('promptNoSavedLogin')));
      } else if (accounts.length === 1) {
        var only = accounts[0];
        card.appendChild(el(doc, 'p', 'panel-helper',
          only.username ? i18n.t('promptAddToOnlyWithUser', [only.title, only.username]) : i18n.t('promptAddToOnly', only.title)));
      } else {
        card.appendChild(el(doc, 'p', 'panel-helper', i18n.t('promptChooseAccount')));
        var picker = el(doc, 'ul', 'pc-accounts');
        var choices = [];
        accounts.forEach(function (account) {
          var item = el(doc, 'li');
          var choice = el(doc, 'button', 'pc-account', account.title + (account.username ? ' — ' + account.username : ''));
          choice.type = 'button';
          choice.setAttribute('aria-pressed', 'false');
          choice.addEventListener('click', function () {
            chosenEntry = account.id;
            choices.forEach(function (button) {
              button.setAttribute('aria-pressed', button === choice ? 'true' : 'false');
            });
            setBusy(controller._busy);
          });
          choices.push(choice);
          item.appendChild(choice);
          picker.appendChild(item);
        });
        card.appendChild(picker);
      }
    } else if (!isLogin) {
      card.appendChild(el(doc, 'p', 'panel-helper',
        state === 'locked' ? messageFor({ type: 'locked' })
          : state === 'pairingRequired' ? messageFor({ type: 'pairingRequired' })
            : messageFor(null)));
    }

    card.appendChild(errorEl);

    var row = el(doc, 'div', 'btn-row');
    var canSave = isLogin || state === 'ready';
    if (canSave) {
      saveBtn = el(doc, 'button', 'btn-primary', i18n.t(isLogin ? 'save' : 'add'));
      saveBtn.type = 'button';
      saveBtn.addEventListener('click', function () { answer('save'); });
      row.appendChild(saveBtn);
    } else {
      var retry = el(doc, 'button', 'btn-primary', i18n.t('tryAgain'));
      retry.type = 'button';
      retry.addEventListener('click', function () { api.reload(); });
      row.appendChild(retry);
    }
    notNowBtn = el(doc, 'button', 'btn-secondary', i18n.t('notNow'));
    notNowBtn.type = 'button';
    notNowBtn.addEventListener('click', function () { answer('dismiss'); });
    row.appendChild(notNowBtn);
    card.appendChild(row);
    setBusy(false);

    card.appendChild(settingToggle(doc, api, isLogin ? 'offerSaveLogin' : 'offerSaveTotp',
      i18n.t(isLogin ? 'promptOfferSaveLogin' : 'promptOfferSaveTotp')));

    // Expiry mid-display: the background drops the record at expiresAt, so the card must stop
    // offering a button that can only fail.
    if (typeof view.expiresAt === 'number' && api.setTimer) {
      var remaining = Math.max(0, view.expiresAt - (api.now ? api.now() : Date.now()));
      controller._timer = api.setTimer(function () {
        if (controller._busy) return;
        if (saveBtn) saveBtn.hidden = true;
        if (notNowBtn) notNowBtn.textContent = i18n.t('close');
        showError(messageFor({ code: 'prompt_gone' }));
      }, remaining);
    }
    return controller;
  }

  // The settings store both surfaces use: the existing autofill settings object in
  // chrome.storage.local, rewritten whole so the other switches keep their values.
  function chromeSettingsStore(chromeApi, autofillSettings) {
    var key = autofillSettings.AUTOFILL_SETTINGS_KEY;
    function read(cb) {
      chromeApi.storage.local.get([key], function (stored) {
        cb(autofillSettings.normalizeAutofillSettings(stored && stored[key]));
      });
    }
    return {
      read: read,
      update: function (name, value) {
        read(function (settings) {
          settings[name] = value;
          var next = {};
          next[key] = settings;
          chromeApi.storage.local.set(next);
        });
      }
    };
  }

  return {
    SETTING_ROWS: SETTING_ROWS,
    render: render,
    chromeSettingsStore: chromeSettingsStore,
    messageFor: messageFor
  };
})();

if (typeof module !== 'undefined') {
  module.exports = mypassPromptCard;
}
