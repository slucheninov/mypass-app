// Field scoring and login-form discovery for the inline icon and for applyFill.
// Moved out of content.js so the heuristics have one home. This file does not
// attach icons, fill fields, or talk to the extension. Callers pass a document
// or a form; `document` is only the fallback when a caller omits the root.
//
// Loaded as a classic script before content.js (manifest.json) and under
// require() from node. Do not add a top-level binding whose name another
// shared script already declares — those scripts share one global scope.

// Minimum OTP field score threshold for selection. The weakest intentional OTP signal
// (inputmode=numeric + maxlength ∈ [6,7,8]) scores 4. A bare type=text input scores 1
// and must not be selected on a login-only page. Score >= 4 ensures only deliberate OTP
// candidates are picked.
const MIN_OTP_FIELD_SCORE = 4;

// Minimum username score, applied ONLY on pages with no password field (see
// findFillableFields). A bare `<input type="text">` scores 1, so without a floor every
// search box and comment field on the web would get a key icon. 3 is the weakest
// intentional signal: `name`/`id` matching user|login|email (2) plus the type bonus (1),
// or type=email (3+1).
const MIN_USERNAME_FIELD_SCORE = 3;

// Text of the <label> associated with `input`, lowercased, or '' if none is found.
// Tries input.closest('label') first (wrapped case: <label>Email <input></label>), then
// falls back to label[for="<id>"] queried against `scope` — the SAME scope the caller
// (findFillableFields) is already using for its other queries, so this never reaches
// outside the form/page region the caller has scoped itself to. `scope` is optional:
// callers that only have a bare input (e.g. direct unit tests) simply skip the [for]
// lookup rather than throwing.
function labelTextFor(input, idAttr, scope) {
  const wrapping = typeof input.closest === 'function' ? input.closest('label') : null;
  if (wrapping) return (wrapping.textContent || '').toLowerCase();
  if (idAttr && scope && typeof scope.querySelector === 'function') {
    const forLabel = scope.querySelector('label[for="' + idAttr + '"]');
    if (forLabel) return (forLabel.textContent || '').toLowerCase();
  }
  return '';
}

// Same phone signals page-identifier.js uses when offering to save a typed number.
// `\btel\b` matches "tel" and "tel-national" without matching "intel" or "hotel".
function mentionsPhone(value) {
  const text = String(value || '').toLowerCase();
  return /phone|mobile|телефон|мобільн|номер\s*телеф/.test(text) || /\btel\b/.test(text);
}

function isShippingOrBillingAutocomplete(autocomplete) {
  return /(^|\s)(shipping|billing)(\s|$)/.test(autocomplete);
}

function autocompleteHasTelToken(autocomplete) {
  return autocomplete.split(/\s+/).some((token) => token === 'tel' || token.startsWith('tel-'));
}

// A checkout phone is filled by address autofill. Login fill must not take it.
const ADDRESS_FIELD_SELECTOR = [
  'input[autocomplete*="street-address" i]',
  'input[autocomplete*="address-line" i]',
  'input[autocomplete*="address-level" i]',
  'input[autocomplete*="postal-code" i]',
  'textarea[autocomplete*="street-address" i]'
].join(', ');

function scopeHasAddressField(scope) {
  if (!scope || typeof scope.querySelectorAll !== 'function') return false;
  const found = scope.querySelectorAll(ADDRESS_FIELD_SELECTOR);
  return !!(found && found.length);
}

function isAddressContactField(input, scope) {
  const autocomplete = (input.getAttribute('autocomplete') || '').toLowerCase();
  if (isShippingOrBillingAutocomplete(autocomplete)) return true;
  const type = (input.getAttribute('type') || '').toLowerCase();
  const inputMode = (input.getAttribute('inputmode') || '').toLowerCase();
  const looksLikePhone = type === 'tel' || inputMode === 'tel'
    || mentionsPhone(autocomplete)
    || mentionsPhone(input.getAttribute('name'))
    || mentionsPhone(input.getAttribute('placeholder'))
    || mentionsPhone(input.getAttribute('aria-label'));
  if (!looksLikePhone) return false;
  const form = input.closest && input.closest('form');
  return scopeHasAddressField(form || scope);
}

function scoreUsernameLikeness(input, scope) {
  let score = 0;
  const autocomplete = (input.getAttribute('autocomplete') || '').toLowerCase();
  const name = (input.getAttribute('name') || '').toLowerCase();
  const idAttr = input.getAttribute('id') || '';
  const id = idAttr.toLowerCase();
  const type = (input.getAttribute('type') || 'text').toLowerCase();
  const inputMode = (input.getAttribute('inputmode') || '').toLowerCase();
  const placeholder = (input.getAttribute('placeholder') || '').toLowerCase();
  const ariaLabel = (input.getAttribute('aria-label') || '').toLowerCase();
  const labelText = labelTextFor(input, idAttr, scope);

  // shipping/billing contact fields belong to address autofill.
  if (isShippingOrBillingAutocomplete(autocomplete)) return 0;

  if (autocomplete.includes('username')) score += 5;
  if (autocompleteHasTelToken(autocomplete)) score += 5;
  if (type === 'email') score += 3;
  // inputmode=tel is an explicit phone field. A bare type=tel is not: SMS codes use
  // type=tel with no other phone hint, and those must stay under the identifier floor.
  if (inputMode === 'tel') score += 3;
  // name and id keep their original, independent +2 each — a field matching BOTH still
  // outscores one matching only name, which matters because findBestUsernameInput picks
  // the single highest scorer among candidates, not just whichever clears the floor.
  const nameMatches = /user|login|email/.test(name) || mentionsPhone(name);
  const idMatches = /user|login|email/.test(id) || mentionsPhone(id);
  if (nameMatches) score += 2;
  if (idMatches) score += 2;
  // placeholder / aria-label / an associated <label>'s text are weaker hints than name/id
  // (a placeholder can describe almost anything), so they only contribute when name/id gave
  // NO signal at all, and only once even if more than one of the three matches — otherwise
  // a field whose placeholder AND aria-label both say "email" would outscore one whose name
  // alone says "user", and a field already identified via name/id would get a second,
  // redundant bonus.
  // Known trade-off, accepted deliberately: a search box with placeholder="Search by
  // email" now also matches this pattern when no name/id hint exists. It under-triggers
  // far less often than the coverage gap this closes (identifier-first login fields hinted
  // only by placeholder/aria-label/label), and the icon stays inert until clicked.
  if (!nameMatches && !idMatches) {
    const hasNewHint = [placeholder, ariaLabel, labelText].some((value) =>
      /user|login|email/.test(value) || mentionsPhone(value)
    );
    if (hasNewHint) score += 2;
  }
  if (type === 'text' || type === 'email' || type === 'tel') score += 1;
  return score;
}

function findBestUsernameInput(candidates, scope) {
  if (candidates.length === 0) return null;
  let best = candidates[0];
  let bestScore = scoreUsernameLikeness(best, scope);
  for (let i = 1; i < candidates.length; i++) {
    const score = scoreUsernameLikeness(candidates[i], scope);
    if (score > bestScore) {
      best = candidates[i];
      bestScore = score;
    }
  }
  return best;
}

function scoreOtpLikeness(input) {
  if (!input) return 0;
  const type = (input.getAttribute('type') || 'text').toLowerCase();
  const isDisabled = input.getAttribute('disabled') !== null || input.disabled === true;
  const isReadOnly = input.getAttribute('readonly') !== null || input.readOnly === true;
  if (type === 'password' || type === 'hidden' || isDisabled || isReadOnly) {
    return 0;
  }

  let score = 0;
  const autocomplete = (input.getAttribute('autocomplete') || '').toLowerCase();
  if (autocomplete === 'one-time-code') {
    // autocomplete=one-time-code is a decisive browser signal for OTP fields, not stacked
    // with other bonuses. Scores 10 alone, not 11 (avoiding +1 type bonus).
    return 10;
  }

  const nameOrId = `${input.getAttribute('name') || ''} ${input.getAttribute('id') || ''}`.toLowerCase();
  if (/otp|totp|2fa|two.?factor|verification.?code|auth.?code|security.?code/.test(nameOrId)) {
    score += 6;
  }

  const inputMode = (input.getAttribute('inputmode') || '').toLowerCase();
  const maxLength = parseInt(input.getAttribute('maxlength') || '', 10);
  if (inputMode === 'numeric' && [6, 7, 8].includes(maxLength)) {
    score += 4;
  }

  if (type === 'tel' || type === 'text') {
    score += 1;
  }

  return score;
}

function findOtpField(root) {
  const scope = root || document;
  const candidates = Array.from(scope.querySelectorAll('input, textarea'));
  let best = null;
  let bestScore = 0;
  for (const candidate of candidates) {
    if (candidate.offsetParent === null) continue;
    const score = scoreOtpLikeness(candidate);
    if (score > bestScore) {
      bestScore = score;
      best = candidate;
    }
  }
  return bestScore >= MIN_OTP_FIELD_SCORE ? best : null;
}

// offsetParent is null both for display:none AND for position:fixed elements, so on its own it
// wrongly excluded rendered fixed-position fields (a pinned login bar, a sticky modal form),
// which then never got an icon. The fast path keeps today's behaviour for every field that has an
// offsetParent; only when it is null do we ask whether the element still has a layout box that is
// not visibility:hidden. Elements without those APIs (test doubles) stay "not rendered".
function isFieldRendered(input) {
  if (input.offsetParent !== null && input.offsetParent !== undefined) return true;
  if (typeof input.getClientRects !== 'function' || input.getClientRects().length === 0) return false;
  const view = input.ownerDocument && input.ownerDocument.defaultView;
  const style = view && typeof view.getComputedStyle === 'function' ? view.getComputedStyle(input) : null;
  return !(style && style.visibility === 'hidden');
}

function isFieldVisible(input) {
  if (!input) return false;
  if (!isFieldRendered(input)) return false;
  const disabled = input.getAttribute('disabled') !== null || input.disabled === true;
  const readOnly = input.getAttribute('readonly') !== null || input.readOnly === true;
  return !disabled && !readOnly;
}

// A password field's autocomplete attribute is a decisive signal when present — the
// same trust level scoreOtpLikeness already gives autocomplete=one-time-code. When it
// is absent, fall back to field count and position:
//   - exactly two unhinted password fields is the signup shape ("new password" +
//     "confirm password"): both count as new-password.
//   - three or more unhinted password fields is treated as a change-password form
//     ("current password", "new password", "confirm new password"): everything
//     except the first counts as new-password, because the first field in that
//     layout is overwhelmingly the box holding the user's existing password.
//     Marking it new-password would offer to generate over the exact field this
//     classification exists to protect.
//   - a single unhinted password field is a login field, not new-password.
// This gives up correct classification for the rare page that stacks three or more
// password fields without following either the signup or change-password
// convention (for example two confirm fields after a first field that is not the
// current password). That is an acceptable loss: failing there only withholds the
// "Generate password" button on an unusual layout, whereas failing the other way —
// as the plain "two or more" rule this replaces did — offers to overwrite a real
// credential in the ordinary change-password case, which is the outcome this
// classification exists to prevent.
function isNewPasswordCandidate(input, allPasswordInputs) {
  const autocomplete = (input.getAttribute('autocomplete') || '').toLowerCase();
  if (autocomplete === 'new-password') return true;
  if (autocomplete === 'current-password') return false;
  if (allPasswordInputs.length === 2) return true;
  if (allPasswordInputs.length >= 3) return allPasswordInputs.indexOf(input) > 0;
  return false;
}

// Returns every field the inline icon may attach to. Deliberately looser than the old
// findLoginForm: a password field no longer has to sit inside a <form>, and a username
// field is emitted even when no password field exists, so identifier-first login screens
// (username on step 1, password on step 2) are covered.
function findFillableFields(root) {
  const scope = root || document;
  const fields = [];

  const passwords = Array.from(scope.querySelectorAll('input[type="password"]')).filter(isFieldVisible);
  for (const password of passwords) {
    fields.push({ input: password, kind: 'password', isNewPassword: isNewPasswordCandidate(password, passwords) });
  }

  const formScope = (passwords[0] && passwords[0].closest && passwords[0].closest('form')) || scope;
  const usernameCandidates = Array.from(
    formScope.querySelectorAll(
      'input[type="text"], input[type="email"], input[type="tel"], input[inputmode="tel" i], input[autocomplete*="username"]'
    )
  ).filter(isFieldVisible).filter((input) => !isAddressContactField(input, formScope));
  const username = findBestUsernameInput(usernameCandidates, formScope);
  // The score floor applies ONLY when there is no password field on the page. When a
  // password field IS present, its presence is itself strong evidence that the nearby text
  // input is the username, so today's floor-free behaviour is kept — otherwise a bare
  // `<input type="text">` login form would silently lose username autofill, which would be
  // a regression of applyFill. Without a password field there is no such evidence, and the
  // identifier-first relaxation above would otherwise put an icon on every search box.
  if (username && (passwords.length > 0 || scoreUsernameLikeness(username, formScope) >= MIN_USERNAME_FIELD_SCORE)) {
    fields.push({ input: username, kind: 'username' });
  }

  // An OTP input is type=text or type=tel, so it also satisfies the username selector
  // above and can win findBestUsernameInput on a 2FA-only page. Classify it as otp and
  // drop the duplicate rather than letting one element carry two icons.
  const otp = findOtpField(scope);
  if (otp) {
    const duplicate = fields.findIndex((field) => field.input === otp);
    if (duplicate >= 0) fields.splice(duplicate, 1);
    fields.push({ input: otp, kind: 'otp' });
  }

  return fields;
}

// Kept for applyFill's existing contract. Rebuilt on findFillableFields so the two
// cannot drift apart. `form` is now nullable: a password field outside any <form> used
// to make this return null, which meant applyFill silently did nothing on such pages.
function findLoginForm(root) {
  const fields = findFillableFields(root || document);
  const passwordField = fields.find((field) => field.kind === 'password');
  if (!passwordField) return null;
  const usernameField = fields.find((field) => field.kind === 'username');
  const form = passwordField.input.closest ? passwordField.input.closest('form') : null;
  return {
    form,
    passwordInput: passwordField.input,
    usernameInput: usernameField ? usernameField.input : null,
  };
}

// Match whole normalized hints so fields such as username, email confirmation,
// and cardholder name do not get an address value from a shared word.
const ADDRESS_FIELD_HINTS = Object.entries({
  name: /^(?:name|full name|contact name|vollständiger name|vor und nachname|повне ім я|ім я та прізвище|полное имя|имя и фамилия|фио)$/,
  'given-name': /^(?:first name|firstname|given name|forename|vorname|ім я|имя)$/,
  'family-name': /^(?:last name|lastname|family name|surname|nachname|familienname|прізвище|фамилия)$/,
  organization: /^(?:company(?: name)?|organization|organisation|business name|firma|firmenname|unternehmen|компанія|компания|організація|организация)$/,
  'street-address': /^full address$/,
  'address-line1': /^(?:address|address line ?1|address ?1|street(?: name| address)?|street 1|street address 1|straße(?: und hausnummer)?|strasse(?: und hausnummer)?|anschrift|адреса|адрес|вулиця|улица)$/,
  'address-line2': /^(?:address line ?2|address ?2|street 2|street address 2|apt|apartment|suite|unit|adresszusatz|wohnung|квартира)$/,
  'address-level2': /^(?:city|town|locality|stadt|ort|wohnort|місто|город|населений пункт|населенный пункт)$/,
  'address-level1': /^(?:state(?: province)?|province|region|county|bundesland|область|регіон|регион|штат)$/,
  'postal-code': /^(?:zip(?: code)?|zipcode|postal ?code|post ?code|postleitzahl|plz|поштовий індекс|почтовый индекс|індекс|индекс)$/,
  'country-name': /^(?:country(?: name| region)?|land(?: region)?|staat|країна|страна)$/,
  country: /^(?:country code|ländercode|код країни|код страны)$/,
  tel: /^(?:phone(?: number)?|telephone(?: number)?|tel|mobile(?: phone| number)?|telefon(?:nummer)?|handynummer|телефон|номер телефону|номер телефона)$/,
  email: /^(?:e ?mail(?: address| adresse)?|електронна пошта|электронная почта|пошта|почта)$/,
});

function addressLabelText(node, control) {
  if (node === control) return '';
  if (!node.contains?.(control)) return node.textContent;
  // A wrapping label's textContent also includes all the options of its select.
  return Array.from(node.childNodes, (child) => addressLabelText(child, control)).join(' ');
}

function addressFieldHint(control) {
  const labels = Array.from(control.labels || [], (label) => addressLabelText(label, control));
  const hints = [control.name, control.id, control.getAttribute('aria-label'), ...labels];
  for (const hint of hints) {
    const text = String(hint || '').normalize('NFKC')
      .replace(/([\p{Ll}\d])(\p{Lu})/gu, '$1 $2').toLowerCase()
      .replace(/[^\p{L}\p{N}]+/gu, ' ').trim()
      .replace(/ (?:optional|required|pflichtfeld|необов язково|необязательно)$/, '');
    const scope = /^(?:checkout )?(shipping|billing)\b ?/.exec(text);
    const purpose = scope ? scope[1] : '';
    const addressHint = text.slice(scope ? scope[0].length : 0).replace(/^checkout /, '');
    const unprefixed = addressHint.replace(/^address /, '');
    for (const [token, pattern] of ADDRESS_FIELD_HINTS) {
      if (pattern.test(addressHint) || pattern.test(unprefixed)) {
        const field = token === 'address-line1' && control.tagName === 'TEXTAREA' ? 'street-address' : token;
        return { token: field, purpose };
      }
    }
  }
  return null;
}

// Registration (sign-up) forms. The identity kinds below are the `kind` values of the bridge's
// `extraFields` (protocol.rs `ExtraFieldKind`); address parts come from addressFieldHint above.
// Hints are normalized like addressFieldHint (NFKC, camelCase split, lowercase, punctuation to
// spaces), so "firstName", "first_name" and "First name:" are all "first name", and an apostrophe
// ("ім'я") becomes a space ("ім я"). Patterns match whole words of that text, in order: the
// first kind that matches wins, so "user name" is a username before it is a name.
function normalizedHint(value) {
  return String(value || '').normalize('NFKC')
    .replace(/([\p{Ll}\d])(\p{Lu})/gu, '$1 $2').toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
}

const WORD = (alternatives) => new RegExp('(?:^| )(?:' + alternatives + ')(?: |$)');

// A field that holds a payment card, a one-time, verification or authentication code, a PIN, a
// captcha, a token, a security answer, a bank or national id, a passport, a search query or a
// coupon is never captured or filled, whatever else its hints or its autocomplete attribute say:
// registration-form.js checks this before any positive hint.
const SENSITIVE_FIELD_HINT = WORD([
  'card(?: number| holder| name)?', 'credit ?card', 'cc(?: number| num| name| exp\\w*)?', 'cvv2?', 'cvc2?', 'csc', 'cid',
  'exp(?:iry|iration)?(?: date| month| year)?', 'security code', 'verification(?: code)?', 'verify code',
  'one ?time(?: code| password| passcode| pin)?', 'otp(?: code)?', 'totp', 'passcode',
  'auth(?:entication|entificator)?(?: code)?', '2fa(?: code)?', 'mfa(?: code)?', 'two ?factor(?: code)?',
  'sms code', 'confirmation code', 'captcha', 'recaptcha', 'pin(?: code| number)?', 'token', 'secret',
  'security answer', 'answer', 'security question', 'iban', 'bic', 'swift', 'routing number', 'account number',
  'ssn', 'social security(?: number)?', 'tax id', 'passport(?: number| no)?', 'search', 'query', 'coupon(?: code)?',
  'promo(?: code)?', 'gift card', 'voucher(?: code)?', 'discount code',
  'karte', 'kartennummer', 'kreditkarte', 'karteninhaber', 'prüfnummer', 'prüfziffer', 'sicherheitscode',
  'gültig bis', 'bestätigungscode', 'verifizierungscode', 'authentifizierungscode', 'einmalcode',
  'einmalpasswort', 'einmalkennwort', 'sicherheitsfrage', 'sicherheitsantwort', 'antwort', 'suche',
  'gutschein(?:code)?', 'rabattcode', 'steuer ?id', 'steuernummer', 'reisepass(?:nummer)?',
  'sozialversicherungsnummer', 'kontonummer', 'tan',
  'картка', 'картки', 'номер картки', 'кредитна картка', 'код безпеки', 'код підтвердження',
  'код автентифікації', 'код аутентифікації', 'одноразовий(?: код| пароль)?', 'пін(?: код)?', 'капча',
  'токен', 'секретне питання', 'контрольне питання', 'відповідь', 'пошук', 'промокод', 'купон',
  'паспорт', 'іпн', 'номер рахунку',
  'карта', 'карты', 'номер карты', 'кредитная карта', 'код безопасности', 'код подтверждения',
  'код аутентификации', 'одноразовый(?: код| пароль)?', 'пин(?: код)?', 'секретный вопрос',
  'контрольный вопрос', 'ответ', 'поиск', 'инн', 'снилс', 'номер счета', 'номер счёта',
].join('|'));

// A password-type field that holds a card code, a PIN or a one-time code, not an account password.
// Narrower than SENSITIVE_FIELD_HINT: "auth" or "secret" in a password field's name is common.
const SENSITIVE_PASSWORD_HINT = WORD([
  'card(?: number| code)?', 'credit ?card', 'cvv2?', 'cvc2?', 'csc', 'cid', 'security code', 'verification code',
  'one ?time(?: code| password| passcode| pin)?', 'otp(?: code)?', 'pin(?: code| number)?', 'tan',
  'prüfnummer', 'sicherheitscode', 'einmalcode', 'einmalpasswort',
  'пін(?: код)?', 'код безпеки', 'одноразовий(?: код| пароль)?',
  'пин(?: код)?', 'код безопасности', 'одноразовый(?: код| пароль)?',
].join('|'));

// A password field that holds the user's existing password (a change-password or re-auth form):
// "current", "old" or "existing" password in the four languages. Registration forms never have one.
const CURRENT_PASSWORD_HINT = WORD([
  'current(?: password)?', 'old(?: password)?', 'existing(?: password)?', 'previous(?: password)?',
  'present password', 'cur(?:r)? pass(?:word)?',
  'aktuelles(?: passwort| kennwort)?', 'altes(?: passwort| kennwort)?', 'bisheriges(?: passwort| kennwort)?',
  'derzeitiges(?: passwort| kennwort)?',
  'поточний(?: пароль)?', 'старий(?: пароль)?', 'чинний(?: пароль)?', 'теперішній(?: пароль)?',
  'текущий(?: пароль)?', 'старый(?: пароль)?', 'действующий(?: пароль)?', 'прежний(?: пароль)?',
].join('|'));

// Birthday words, used alone (a whole birthday) and as the context of a split day, month or year.
const BIRTHDAY_WORDS = [
  'birthday', 'birth ?date', 'date of birth', 'dob', 'bday',
  'geburtsdatum', 'geburtstag',
  'дата народження', 'день народження',
  'дата рождения', 'день рождения',
];
// The part words of a split birthday (day, month, year), in four languages.
const BIRTHDAY_PARTS = {
  bdayDay: ['day', 'dd', 'tag', 'день'],
  bdayMonth: ['month', 'mm', 'monat', 'місяць', 'месяц'],
  bdayYear: ['year', 'yyyy', 'yy', 'jahr', 'рік', 'год'],
};
const BIRTHDAY_WORD = BIRTHDAY_WORDS.join('|');
// Whole-word names of one part: "birth year", "Geburtsjahr", "рік народження".
const BIRTHDAY_PART_NAMES = {
  bdayDay: [],
  bdayMonth: ['birth ?month', 'geburtsmonat', 'місяць народження', 'месяц рождения'],
  bdayYear: ['birth ?year', 'geburtsjahr', 'рік народження', 'год рождения'],
};
// "bday day", "dob_month", "birthDateYear", "day of birth", "geburtstag tag". Never "birth day",
// which names a whole birthday as often as its day.
const SPLIT_BIRTHDAY_HINTS = Object.entries(BIRTHDAY_PARTS).map(([kind, parts]) => {
  const part = parts.join('|');
  return [kind, WORD([
    '(?:' + BIRTHDAY_WORD + ') (?:' + part + ')',
    '(?:' + part + ') of (?:' + BIRTHDAY_WORD + '|birth)',
    ...BIRTHDAY_PART_NAMES[kind],
  ].join('|'))];
});
// A bare part word ("Day", "Monat", "Рік"), a split birthday only inside a birthday group.
const BARE_BIRTHDAY_PARTS = Object.entries(BIRTHDAY_PARTS)
  .map(([kind, parts]) => [kind, new RegExp('^(?:' + parts.join('|') + ')$')]);
const BIRTHDAY_CONTEXT = WORD(BIRTHDAY_WORD);

const IDENTITY_FIELD_HINTS = [
  ...SPLIT_BIRTHDAY_HINTS,
  ['username', WORD([
    'user ?name', 'user id', 'userid', 'login(?: name| id)?', 'nick ?name', 'screen name', 'account name', 'display name',
    'benutzer(?: ?name)?', 'anmeldename', 'spitzname',
    'логін', 'ім я користувача', 'нікнейм', 'псевдонім',
    'логин', 'имя пользователя', 'никнейм', 'псевдоним',
  ].join('|'))],
  ['email', WORD([
    'e ?mail(?: address| adresse)?', 'mail(?: address| adresse)', 'e ?mail ?adresse',
    'електронна пошта', 'ел пошта', 'е пошта', 'пошта',
    'электронная почта', 'эл почта', 'почта',
  ].join('|'))],
  ['phone', WORD([
    'phone(?: number)?', 'telephone(?: number)?', 'tel', 'mobile(?: phone| number)?', 'cell(?: phone)?',
    'telefon(?:nummer)?', 'handy(?:nummer)?', 'mobil(?:nummer|telefon)?', 'rufnummer',
    'телефон', 'номер телефону', 'мобільний(?: телефон)?',
    'номер телефона', 'мобильный(?: телефон)?',
  ].join('|'))],
  ['birthday', WORD(BIRTHDAY_WORD)],
  // Before the given and family name: "middle name" and "ім'я по батькові" hold a name word too.
  ['middleName', WORD([
    'middle ?name', 'middle initial', 'mname', 'additional name', 'patronymic',
    'zweiter vorname', 'weitere vornamen', 'zweitname',
    'по батькові', 'ім я по батькові',
    'отчество', 'имя отчество',
  ].join('|'))],
  ['familyName', WORD([
    'last ?name', 'family ?name', 'surname', 'lname',
    'nachname', 'familienname',
    'прізвище', 'фамилия',
  ].join('|'))],
  ['givenName', WORD([
    'first ?name', 'given ?name', 'forename', 'fname',
    'vorname',
    'ім я', 'имя',
  ].join('|'))],
  ['name', /^(?:name|your name|full ?name|vollständiger name|ihr name|vor und nachname|ваше ім я|повне ім я|піб|ваше имя|полное имя|фио)$/],
  ['organization', WORD([
    'company(?: name)?', 'organi[sz]ation', 'business name',
    'firma', 'firmenname', 'unternehmen',
    'компанія', 'організація', 'компания', 'организация',
  ].join('|'))],
];

// The words of a button (or input value) that creates an account: a registration form's submit.
const REGISTRATION_ACTION_TEXT = WORD([
  'sign ?up', 'register', 'registration', 'create(?: an| your| my)? account', 'join(?: now| free)?', 'get started',
  'open(?: an)? account',
  'registrieren', 'registrierung', 'jetzt registrieren', 'konto (?:erstellen|anlegen|eröffnen)', 'kostenlos registrieren',
  'зареєструватися', 'зареєструватись', 'реєстрація', 'створити (?:акаунт|обліковий запис)',
  'зарегистрироваться', 'регистрация', 'создать (?:аккаунт|учетную запись|учётную запись)',
].join('|'));

// The texts that describe a control: name, id, aria-label, placeholder, associated labels.
function controlHints(control) {
  const labels = Array.from(control.labels || [], (label) => addressLabelText(label, control));
  return [
    control.getAttribute('name'), control.getAttribute('id'), control.getAttribute('aria-label'),
    control.getAttribute('placeholder'), ...labels,
  ].map(normalizedHint).filter(Boolean);
}

function isSensitiveFieldHint(control) {
  return controlHints(control).some((hint) => SENSITIVE_FIELD_HINT.test(hint));
}

function isSensitivePasswordHint(control) {
  return controlHints(control).some((hint) => SENSITIVE_PASSWORD_HINT.test(hint));
}

function isCurrentPasswordHint(control) {
  return controlHints(control).some((hint) => CURRENT_PASSWORD_HINT.test(hint));
}

// Whether a control is really on screen, the way the address autofill decides it
// (address-form.js `editable`): connected, not under [hidden], [inert] or aria-hidden="true",
// no ancestor with display:none or opacity 0, visibility "visible", and a box of some size. No
// offsetParent shortcut: a field inside an opacity:0 or visibility:hidden wrapper still has one.
// Elements without those APIs (test doubles) are not shown.
function isControlShown(control) {
  if (!control || control.isConnected === false) return false;
  if (typeof control.closest !== 'function' || control.closest('[hidden], [inert], [aria-hidden="true"]')) return false;
  const view = control.ownerDocument && control.ownerDocument.defaultView;
  if (!view || typeof view.getComputedStyle !== 'function' || typeof control.getBoundingClientRect !== 'function') return false;
  for (let node = control; node; node = node.parentElement) {
    const style = view.getComputedStyle(node);
    if (!style || style.display === 'none' || Number(style.opacity) === 0) return false;
    if (node === control && style.visibility !== 'visible') return false;
  }
  const box = control.getBoundingClientRect();
  return !!box && box.width > 0 && box.height > 0;
}

// Disabled itself or through a disabled <fieldset> (`:disabled` matches both).
function isEffectivelyDisabled(control) {
  if (!control) return true;
  if (control.disabled === true) return true;
  return typeof control.matches === 'function' && control.matches(':disabled');
}

// The texts that name the group a control sits in: the legend of its fieldset and the label of an
// enclosing role=group or role=radiogroup (aria-label, or the text aria-labelledby points at).
function groupHints(control) {
  const texts = [];
  const fieldset = typeof control.closest === 'function' ? control.closest('fieldset') : null;
  const legend = fieldset && Array.from(fieldset.children || []).find((child) => child.tagName === 'LEGEND');
  if (legend) texts.push(legend.textContent);
  const group = typeof control.closest === 'function' ? control.closest('[role="group"], [role="radiogroup"]') : null;
  if (group) {
    texts.push(group.getAttribute('aria-label'));
    const doc = control.ownerDocument;
    for (const id of String(group.getAttribute('aria-labelledby') || '').split(/\s+/).filter(Boolean)) {
      const node = doc && typeof doc.getElementById === 'function' ? doc.getElementById(id) : null;
      if (node) texts.push(node.textContent);
    }
  }
  return texts.map(normalizedHint).filter(Boolean);
}

// The identity kind of a control from its texts, or null. Sensitive fields are never classified.
// A control named only "Day", "Month" or "Year" (localized) is a part of a split birthday when its
// fieldset legend or group label says birthday.
function identityFieldHint(control) {
  const hints = controlHints(control);
  if (hints.some((hint) => SENSITIVE_FIELD_HINT.test(hint))) return null;
  for (const [kind, pattern] of IDENTITY_FIELD_HINTS) {
    if (hints.some((hint) => pattern.test(hint))) return kind;
  }
  for (const [kind, pattern] of BARE_BIRTHDAY_PARTS) {
    if (hints.some((hint) => pattern.test(hint)) && groupHints(control).some((hint) => BIRTHDAY_CONTEXT.test(hint))) {
      return kind;
    }
  }
  return null;
}

function isRegistrationActionText(text) {
  return REGISTRATION_ACTION_TEXT.test(normalizedHint(text));
}

// Bare name for content.js. globalThis in a content script is the page, so
// globalThis.findLoginForm is not this function (LinkedIn throws
// "findLoginForm is not a function").
var mypassFormDetection = {
  scoreUsernameLikeness,
  findBestUsernameInput,
  scoreOtpLikeness,
  findOtpField,
  isFieldVisible,
  isNewPasswordCandidate,
  findFillableFields,
  findLoginForm,
  addressFieldHint,
  identityFieldHint,
  isSensitiveFieldHint,
  isSensitivePasswordHint,
  isCurrentPasswordHint,
  isControlShown,
  isEffectivelyDisabled,
  isRegistrationActionText,
};

if (typeof module !== 'undefined') {
  module.exports = mypassFormDetection;
}
