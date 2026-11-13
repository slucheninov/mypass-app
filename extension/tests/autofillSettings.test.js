const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const autofill = require('../shared/autofill-settings.js');
const { chooseInlinePanel } = require('../content.js');

const defaults = autofill.defaultAutofillSettings();
assert.strictEqual(defaults.offerSaveLogin, true);
assert.strictEqual(defaults.offerSaveTotp, true);
assert.strictEqual(defaults.showFieldIcon, true);
assert.strictEqual(defaults.showLogins, true);
assert.strictEqual(defaults.showCards, true);
assert.strictEqual(defaults.showAddresses, true);
assert.strictEqual(defaults.autoSubmit, false, 'a fill must not click Sign in until the user opts in');

assert.deepStrictEqual(autofill.normalizeAutofillSettings(null), defaults);
assert.deepStrictEqual(autofill.normalizeAutofillSettings('on'), defaults);
assert.strictEqual(
  autofill.normalizeAutofillSettings({ offerSaveLogin: false, autoSubmit: 'yes', showCards: false }).offerSaveLogin,
  false
);
assert.strictEqual(
  autofill.normalizeAutofillSettings({ offerSaveLogin: false, autoSubmit: 'yes' }).autoSubmit,
  false,
  'a non-boolean does not turn on a new click'
);
assert.strictEqual(
  autofill.normalizeAutofillSettings({ offerSaveLogin: false }).showFieldIcon,
  true,
  'a partial record keeps the other current behaviors'
);

const login = { id: 'login', title: 'Bank', entryType: 'Login', hasTotp: true };
const untyped = { id: 'plain', title: 'Plain', hasTotp: false };
const card = { id: 'card', title: 'Visa', entryType: 'Card' };
const address = { id: 'home', title: 'Home', entryType: 'Address' };
const note = { id: 'note', title: 'Wifi', entryType: 'Notes' };
const mixed = { type: 'credentialList', credentials: [login, untyped, card, address, note] };

assert.deepStrictEqual(
  chooseInlinePanel(mixed, 'password').entries.map((entry) => entry.id),
  ['login', 'plain', 'card', 'home'],
  'defaults keep logins, cards, and addresses, and drop notes'
);
assert.deepStrictEqual(
  chooseInlinePanel(mixed, 'password', { showLogins: false }).entries.map((entry) => entry.id),
  ['card', 'home']
);
assert.deepStrictEqual(
  chooseInlinePanel(mixed, 'password', { showCards: false, showAddresses: false }).entries.map((entry) => entry.id),
  ['login', 'plain']
);
assert.deepStrictEqual(
  chooseInlinePanel(mixed, 'otp', { showLogins: true }).entries.map((entry) => entry.id),
  ['login'],
  'an otp field still lists only logins that have a code'
);
assert.strictEqual(chooseInlinePanel(mixed, 'password', { showLogins: false, showCards: false, showAddresses: false }).panel, 'empty');
console.log('autofillSettings.test.js: suggestion filters OK');

assert.strictEqual(autofill.shouldAutoSubmitLogin(null, true, undefined), false);
assert.strictEqual(autofill.shouldAutoSubmitLogin({ autoSubmit: true }, false, undefined), false);
assert.strictEqual(autofill.shouldAutoSubmitLogin({ autoSubmit: true }, true, 'newPassword'), false);
assert.strictEqual(autofill.shouldAutoSubmitLogin({ autoSubmit: true }, true, undefined), true);

function control(tag, type, extra) {
  return Object.assign({
    tagName: tag,
    type: type,
    disabled: false,
    clicked: false,
    click() { this.clicked = true; }
  }, extra);
}
function formOf(controls) {
  return { querySelectorAll() { return controls; } };
}

const disabledSubmit = control('BUTTON', 'submit', { disabled: true });
const plainButton = control('BUTTON', 'button');
const signIn = control('BUTTON', '');
assert.strictEqual(autofill.loginSubmitControl(formOf([disabledSubmit, plainButton, signIn])), signIn);
assert.strictEqual(autofill.loginSubmitControl(formOf([plainButton])), null, 'type=button is not a sign-in control');
assert.strictEqual(autofill.loginSubmitControl(null), null);

const passwordInput = { form: formOf([signIn]) };
assert.strictEqual(autofill.maybeSubmitFilledLogin(passwordInput, { autoSubmit: false }, true, undefined), false);
assert.strictEqual(signIn.clicked, false);
assert.strictEqual(autofill.maybeSubmitFilledLogin(passwordInput, { autoSubmit: true }, true, 'newPassword'), false);
assert.strictEqual(signIn.clicked, false);
assert.strictEqual(autofill.maybeSubmitFilledLogin({ form: null }, { autoSubmit: true }, true, undefined), false);
assert.strictEqual(autofill.maybeSubmitFilledLogin(passwordInput, { autoSubmit: true }, true, undefined), true);
assert.strictEqual(signIn.clicked, true, 'an opted-in login fill clicks the form submit control');
console.log('autofillSettings.test.js: auto-submit stays opt-in OK');

const contentSource = fs.readFileSync(path.join(__dirname, '..', 'content.js'), 'utf8');
assert.ok(contentSource.includes('chooseInlinePanel(response, kind, pageAutofillSettings)'));
assert.ok(contentSource.includes('pageAutofillSettings.offerSaveLogin'));
assert.ok(contentSource.includes('pageAutofillSettings.offerSaveTotp'));
assert.ok(contentSource.includes('pageAutofillSettings.showFieldIcon'));
assert.ok(contentSource.includes('pageAutofillSettings.showAddresses'));
assert.ok(contentSource.includes('maybeSubmitFilledLogin'));
const popupSource = fs.readFileSync(path.join(__dirname, '..', 'app', 'app.js'), 'utf8');
assert.ok(popupSource.includes("type: 'setPasskeyProxyEnabled'"));
assert.ok(popupSource.includes('mypassAutofillSettings'));
assert.ok(!popupSource.includes('passkeyCredentialId'));
console.log('autofillSettings.test.js: page wiring OK');
