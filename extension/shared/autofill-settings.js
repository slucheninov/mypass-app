// What the extension offers on a page. Stored in chrome.storage.local.
// The passkey switch is not here: the MyPass app owns that setting.
//
// One var bag. A top-level const or function here would share the classic-script
// scope with the other extension scripts and can throw at load.

var mypassAutofillSettings = (function () {
  var AUTOFILL_SETTINGS_KEY = 'mypassAutofillSettings';

  function defaultAutofillSettings() {
    return {
      offerSaveLogin: true,
      offerSaveTotp: true,
      showFieldIcon: true,
      showLogins: true,
      showCards: true,
      showAddresses: true,
      // Clicking Sign in is new. Leave it off so a fill still only writes fields.
      autoSubmit: false
    };
  }

  function normalizeAutofillSettings(value) {
    var defaults = defaultAutofillSettings();
    if (!value || typeof value !== 'object') return defaults;
    function flag(name) {
      return typeof value[name] === 'boolean' ? value[name] : defaults[name];
    }
    return {
      offerSaveLogin: flag('offerSaveLogin'),
      offerSaveTotp: flag('offerSaveTotp'),
      showFieldIcon: flag('showFieldIcon'),
      showLogins: flag('showLogins'),
      showCards: flag('showCards'),
      showAddresses: flag('showAddresses'),
      autoSubmit: flag('autoSubmit')
    };
  }

  // Rows with no type are logins: getCredentials does not send entryType.
  // Notes are never a field suggestion.
  function suggestionKind(entry) {
    var type = entry && entry.entryType;
    if (type === 'Card') return 'card';
    if (type === 'Address') return 'address';
    if (type === 'Notes') return 'notes';
    return 'login';
  }

  function filterSuggestionEntries(entries, settings) {
    var prefs = normalizeAutofillSettings(settings);
    var list = Array.isArray(entries) ? entries : [];
    var allowed = {
      login: prefs.showLogins,
      card: prefs.showCards,
      address: prefs.showAddresses,
      notes: false
    };
    var filtered = [];
    for (var i = 0; i < list.length; i++) {
      if (allowed[suggestionKind(list[i])]) filtered.push(list[i]);
    }
    return filtered;
  }

  function shouldAutoSubmitLogin(settings, wrotePassword, fillTarget) {
    var prefs = normalizeAutofillSettings(settings);
    if (!prefs.autoSubmit || wrotePassword !== true) return false;
    if (fillTarget === 'newPassword') return false;
    return true;
  }

  function controlType(control) {
    if (!control) return '';
    if (typeof control.type === 'string' && control.type) return control.type.toLowerCase();
    if (typeof control.getAttribute === 'function') {
      var attr = control.getAttribute('type');
      if (typeof attr === 'string' && attr) return attr.toLowerCase();
    }
    return '';
  }

  // The first real submit control inside the filled form. A button with no type
  // is a submit button. Disabled controls and type=button are skipped. Nothing
  // outside the form is considered, and a missing control means do not submit.
  function loginSubmitControl(form) {
    if (!form || typeof form.querySelectorAll !== 'function') return null;
    var controls = form.querySelectorAll('input, button');
    if (!controls || typeof controls.length !== 'number') return null;
    for (var i = 0; i < controls.length; i++) {
      var control = controls[i];
      if (!control || control.disabled) continue;
      var tag = String(control.tagName || '').toLowerCase();
      var type = controlType(control);
      if (tag === 'input' && (type === 'submit' || type === 'image')) return control;
      if (tag === 'button' && (type === '' || type === 'submit')) return control;
    }
    return null;
  }

  function maybeSubmitFilledLogin(passwordInput, settings, wrotePassword, fillTarget) {
    if (!shouldAutoSubmitLogin(settings, wrotePassword, fillTarget)) return false;
    if (!passwordInput || !passwordInput.form) return false;
    var control = loginSubmitControl(passwordInput.form);
    if (!control || typeof control.click !== 'function') return false;
    control.click();
    return true;
  }

  return {
    AUTOFILL_SETTINGS_KEY: AUTOFILL_SETTINGS_KEY,
    defaultAutofillSettings: defaultAutofillSettings,
    normalizeAutofillSettings: normalizeAutofillSettings,
    filterSuggestionEntries: filterSuggestionEntries,
    shouldAutoSubmitLogin: shouldAutoSubmitLogin,
    loginSubmitControl: loginSubmitControl,
    maybeSubmitFilledLogin: maybeSubmitFilledLogin
  };
})();

if (typeof module !== 'undefined') {
  module.exports = mypassAutofillSettings;
}
