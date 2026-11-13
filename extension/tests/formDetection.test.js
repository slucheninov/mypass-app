const assert = require('node:assert');
const path = require('node:path');
const { scoreUsernameLikeness, findBestUsernameInput, hostnameMatches, scoreOtpLikeness, findOtpField } = require(path.join(__dirname, '..', 'content.js'));

function fakeInput(attrs, extra = {}) {
  return {
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(attrs, name) ? attrs[name] : null;
    },
    closest(sel) {
      return sel === 'label' ? extra.label || null : null;
    }
  };
}

function fakeLabel(text) {
  return { textContent: text };
}

const usernameField = fakeInput({ type: 'text', autocomplete: 'username', name: 'user' });
const genericTextField = fakeInput({ type: 'text', name: 'promo-code' });
assert.ok(
  scoreUsernameLikeness(usernameField) > scoreUsernameLikeness(genericTextField),
  'autocomplete=username field should score higher than an unrelated text field'
);

const emailField = fakeInput({ type: 'email', name: 'email' });
assert.ok(
  scoreUsernameLikeness(emailField) > scoreUsernameLikeness(genericTextField),
  'email field should score higher than an unrelated text field'
);

const best = findBestUsernameInput([genericTextField, usernameField, emailField]);
assert.strictEqual(best, usernameField, 'expected the autocomplete=username field to win');

assert.strictEqual(findBestUsernameInput([]), null, 'empty candidate list must return null, not throw');

// --- placeholder / aria-label / label hints (identifier-first fields with no name/id/type clue) ---

const placeholderOnly = fakeInput({ type: 'text', placeholder: 'Email' });
assert.strictEqual(scoreUsernameLikeness(placeholderOnly), 3, 'placeholder="Email" alone scores 3 (hint 2 + type 1)');

const ariaLabelOnly = fakeInput({ type: 'text', 'aria-label': 'Username' });
assert.strictEqual(scoreUsernameLikeness(ariaLabelOnly), 3, 'aria-label="Username" alone scores 3 (hint 2 + type 1)');

const wrappedInLabel = fakeInput({ type: 'text' }, { label: fakeLabel('Email ') });
assert.strictEqual(scoreUsernameLikeness(wrappedInLabel), 3, 'input.closest("label") text alone scores 3 (hint 2 + type 1)');

const idField = fakeInput({ type: 'text', id: 'signin-id' });
const scopeWithForLabel = { querySelector: (sel) => (sel === 'label[for="signin-id"]' ? fakeLabel('Email') : null) };
assert.strictEqual(
  scoreUsernameLikeness(idField, scopeWithForLabel),
  3,
  'label[for="<id>"] looked up in the caller scope alone scores 3 (hint 2 + type 1)'
);

const searchPlaceholder = fakeInput({ type: 'text', placeholder: 'Search' });
assert.strictEqual(scoreUsernameLikeness(searchPlaceholder), 1, 'placeholder="Search" does not match user|login|email — scores only the type bonus');

// Phone-first login (YASNO: placeholder "Номер телефону", often type=tel). The hint
// counts the same way "Email" does. A bare type=tel does not, so an SMS code box is
// not treated as the login. shipping/billing autocomplete belongs to address fill.
const phonePlaceholder = fakeInput({ type: 'text', placeholder: 'Номер телефону' });
assert.strictEqual(scoreUsernameLikeness(phonePlaceholder), 3, 'placeholder "Номер телефону" scores 3 (hint 2 + type 1)');

const telWithPlaceholder = fakeInput({ type: 'tel', placeholder: 'Номер телефону' });
assert.strictEqual(scoreUsernameLikeness(telWithPlaceholder), 3, 'type=tel with a phone placeholder scores 3, enough to clear the identifier-first floor');

const bareTel = fakeInput({ type: 'tel' });
assert.strictEqual(scoreUsernameLikeness(bareTel), 1, 'a bare type=tel scores only the type bonus — an SMS field must not clear the floor by itself');

const telAutocomplete = fakeInput({ type: 'tel', autocomplete: 'tel' });
assert.strictEqual(scoreUsernameLikeness(telAutocomplete), 6, 'autocomplete=tel is decisive (5) plus the type bonus (1)');

const inputModeTel = fakeInput({ type: 'text', inputmode: 'tel' });
assert.strictEqual(scoreUsernameLikeness(inputModeTel), 4, 'inputmode=tel scores 3 plus the text type bonus');

const billingTel = fakeInput({ type: 'tel', autocomplete: 'billing tel', placeholder: 'Phone' });
assert.strictEqual(scoreUsernameLikeness(billingTel), 0, 'a billing phone belongs to address fill, not login fill');

// The placeholder/aria-label/label hint bonus is a single +2 that only applies when name/id
// gave NO signal — a field matching in BOTH name and placeholder must score the same as one
// matching only name (the new source adds nothing on top of an existing name/id match).
const nameOnly = fakeInput({ type: 'text', name: 'user-email' });
const nameAndPlaceholder = fakeInput({ type: 'text', name: 'user-email', placeholder: 'Email' });
assert.strictEqual(
  scoreUsernameLikeness(nameAndPlaceholder),
  scoreUsernameLikeness(nameOnly),
  'matching in both name and placeholder must not stack — same score as matching only name'
);

// name and id, however, keep their ORIGINAL independent scoring — this is not a "hint group"
// merge. A field matching in both must still score strictly higher than one matching only
// name, because findBestUsernameInput picks the single highest scorer among candidates, not
// just whichever clears MIN_USERNAME_FIELD_SCORE. (This is the regression a prior draft of
// this feature introduced by wrongly folding name/id into the same capped group as the new
// placeholder/aria-label/label sources — pinned here so it fails loudly if reintroduced.)
const nameAndId = fakeInput({ type: 'text', name: 'username', id: 'username' });
const nameField = fakeInput({ type: 'text', name: 'username' });
assert.ok(
  scoreUsernameLikeness(nameAndId) > scoreUsernameLikeness(nameField),
  'a field matching in both name and id must score strictly higher than one matching only name'
);

console.log('formDetection.test.js: placeholder/aria-label/label hint scoring OK');

assert.strictEqual(
  hostnameMatches('https://example.com/login', 'https://example.com/account'),
  true,
  'same hostname (different path) should match'
);

assert.strictEqual(
  hostnameMatches('https://evil.example/login', 'https://example.com/account'),
  false,
  'different hostname must not match'
);

assert.strictEqual(
  hostnameMatches('https://example.com', 'https://sub.example.com'),
  false,
  'subdomain vs bare domain must not match (exact hostname comparison, no loose matching)'
);

assert.strictEqual(
  hostnameMatches(null, 'https://example.com'),
  false,
  'missing message URL must be rejected, not treated as a match'
);

assert.strictEqual(
  hostnameMatches('not a url', 'https://example.com'),
  false,
  'malformed message URL must be rejected rather than throwing'
);

console.log('content.js form-detection heuristics OK');

// --- OTP field scoring ---

const otpAutocomplete = fakeInput({ type: 'text', autocomplete: 'one-time-code' });
assert.strictEqual(scoreOtpLikeness(otpAutocomplete), 10, 'autocomplete=one-time-code scores 10');

const otpByName = fakeInput({ type: 'text', name: 'otp-code' });
assert.strictEqual(scoreOtpLikeness(otpByName), 7, 'name matching otp pattern scores 6, plus 1 for type=text');

const otpByNumericMode = fakeInput({ type: 'text', inputmode: 'numeric', maxlength: '6' });
assert.strictEqual(scoreOtpLikeness(otpByNumericMode), 5, 'numeric inputmode + maxlength 6 scores 4, plus 1 for type=text');

const genericText = fakeInput({ type: 'text' });
assert.strictEqual(scoreOtpLikeness(genericText), 1, 'plain text input scores 1 (fallback signal)');

const passwordField = fakeInput({ type: 'password', autocomplete: 'one-time-code' });
assert.strictEqual(scoreOtpLikeness(passwordField), 0, 'password inputs are excluded even with matching autocomplete');

const hiddenField = fakeInput({ type: 'hidden', name: 'otp' });
assert.strictEqual(scoreOtpLikeness(hiddenField), 0, 'hidden inputs are excluded');

const disabledField = fakeInput({ type: 'text', disabled: 'true', name: 'otp' });
assert.strictEqual(scoreOtpLikeness(disabledField), 0, 'disabled inputs are excluded');

const readonlyField = fakeInput({ type: 'text', readonly: 'true', name: 'otp' });
assert.strictEqual(scoreOtpLikeness(readonlyField), 0, 'readonly inputs are excluded');

console.log('formDetection.test.js: OTP scoring OK');

// --- findOtpField ---

const fakeOtpDoc = {
  querySelectorAll: () => [
    fakeInput({ type: 'text' }),
    fakeInput({ type: 'text', autocomplete: 'one-time-code', id: 'best-match' }),
    fakeInput({ type: 'password' }),
  ],
};
const found = findOtpField(fakeOtpDoc);
assert.strictEqual(found.getAttribute('id'), 'best-match', 'findOtpField picks the highest-scoring candidate');

const fakeLoginOnlyDoc = {
  querySelectorAll: () => [
    fakeInput({ type: 'text', name: 'username' }),
    fakeInput({ type: 'password', name: 'password' }),
  ],
};
assert.strictEqual(findOtpField(fakeLoginOnlyDoc), null, 'no OTP-like field on a login-only page');

const fakeOtpOnlyDoc = {
  querySelectorAll: () => [
    fakeInput({ type: 'text', autocomplete: 'one-time-code' }),
  ],
};
assert.notStrictEqual(findOtpField(fakeOtpOnlyDoc), null, 'OTP-only page (no password field) still finds the OTP input');

// --- Boundary tests ---

// Textarea with OTP-like attributes is scored and selected like inputs
const fakeTextarea = fakeInput({ name: 'otp-code' });
const fakeTextareaDoc = {
  querySelectorAll: () => [
    fakeInput({ type: 'text', name: 'regular-field' }),
    fakeTextarea,
  ],
};
const textareaResult = findOtpField(fakeTextareaDoc);
assert.strictEqual(textareaResult, fakeTextarea, 'textarea with name=otp-code (score 7) is found over plain text (score 1)');

// Score-exactly-4 boundary: inputmode=numeric + maxlength=6, no type bonus (type='number')
const scoreOf4Input = fakeInput({ type: 'number', inputmode: 'numeric', maxlength: '6' });
assert.strictEqual(scoreOtpLikeness(scoreOf4Input), 4, 'type=number with inputmode=numeric + maxlength=6 scores exactly 4 (no type bonus)');
const fakeScoreOf4Doc = {
  querySelectorAll: () => [
    fakeInput({ type: 'text', name: 'username' }),
    scoreOf4Input,
  ],
};
assert.notStrictEqual(findOtpField(fakeScoreOf4Doc), null, 'score-of-4 field at MIN_OTP_FIELD_SCORE boundary is selected');

// Higher combined signals beat a single strong signal: name + inputmode + maxlength + type (11) > autocomplete alone (10)
const fakeCompetingDoc = {
  querySelectorAll: () => [
    fakeInput({ type: 'text', autocomplete: 'one-time-code', id: 'strong-signal' }),
    fakeInput({ type: 'text', name: 'otp-code', inputmode: 'numeric', maxlength: '6', id: 'higher-score' }),
  ],
};
const higherScoreWins = findOtpField(fakeCompetingDoc);
assert.strictEqual(higherScoreWins.getAttribute('id'), 'higher-score', 'combined signals (name+inputmode+maxlength+type=11) beat autocomplete-alone (10); higher score always wins');

console.log('formDetection.test.js: OTP boundary tests OK');
console.log('formDetection.test.js: findOtpField OK');
