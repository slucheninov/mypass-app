// Registration (sign-up) forms: which forms create an account, what each field holds, filling the
// identity and address fields from a MyPass Address entry, and capturing the filled form when the
// user submits it. DOM functions only, no chrome.*: content.js wires them to the inline icon, the
// panel and the save prompt.
//
// A form is a registration form when it has a new-password field (autocomplete=new-password, the
// "password + confirm" pair, or a single password next to a "Sign up" button or real identity
// fields) and no current-password field, or, without any password, at least two identity kinds
// in three or more fields. Field kinds come from the autocomplete attribute first, then from the
// localized hints in form-detection.js. Hidden inputs, password fields other than the one new
// password, card, one-time-code and other sensitive fields are never classified, captured or
// filled.
//
// What a capture sends is checked again by the background (prompt-record.js) and by the bridge,
// which refuses the whole proposal when a field breaks a limit, so the limits here match theirs:
// at most 24 fields, labels of at most 64 and values of at most 512 characters, one clean line
// each, and no label starting with a reserved MyPass, KeePassXC or KP2A prefix.
//
// One var bag, like the other shared scripts (they share one classic-script scope).
var mypassRegistrationForm = (() => {
  const formDetection = typeof require === 'function' ? require('./form-detection.js') : mypassFormDetection;
  const addressForm = typeof require === 'function' ? require('./address-form.js') : mypassAddressForm;

  const MAX_FIELDS = 24;
  const MAX_LABEL = 64;
  const MAX_VALUE = 512;
  const RESERVED_LABEL_PREFIXES = ['mypass.', 'kpex_', 'kp2a_'];
  const UNSAFE_CHAR = /[\p{Cc}\p{Cf}]/u;
  // Input types a registration field may have. Never hidden, password, file, checkbox or radio.
  const FIELD_TYPES = new Set(['text', 'email', 'tel', 'date', 'number', '']);
  const SENSITIVE = 'sensitive';

  const AUTOCOMPLETE_KINDS = {
    name: 'name', 'given-name': 'givenName', 'family-name': 'familyName', email: 'email',
    tel: 'phone', 'tel-national': 'phone', bday: 'birthday', username: 'username', nickname: 'username',
    organization: 'organization', 'street-address': 'addressLine1', 'address-line1': 'addressLine1',
    'address-line2': 'addressLine2', 'address-level2': 'city', 'address-level1': 'region',
    'postal-code': 'postalCode', country: 'country', 'country-name': 'country',
  };
  // addressFieldHint tokens (exact, localized hints) as kinds.
  const ADDRESS_HINT_KINDS = {
    name: 'name', 'given-name': 'givenName', 'family-name': 'familyName', organization: 'organization',
    'street-address': 'addressLine1', 'address-line1': 'addressLine1', 'address-line2': 'addressLine2',
    'address-level2': 'city', 'address-level1': 'region', 'postal-code': 'postalCode',
    'country-name': 'country', country: 'country', tel: 'phone', email: 'email',
  };
  // The kinds an Address entry can fill, as the address-form token that names the value.
  const FILL_TOKENS = {
    name: 'name', givenName: 'given-name', familyName: 'family-name', email: 'email', phone: 'tel',
    organization: 'organization', addressLine1: 'address-line1', addressLine2: 'address-line2',
    city: 'address-level2', region: 'address-level1', postalCode: 'postal-code', country: 'country-name',
  };
  // Kinds that say a single-password form is about a person, not a login.
  const PERSON_KINDS = new Set([
    'name', 'givenName', 'familyName', 'phone', 'birthday', 'organization',
    'addressLine1', 'addressLine2', 'city', 'region', 'postalCode', 'country',
  ]);

  function attr(node, name) {
    return typeof node.getAttribute === 'function' ? node.getAttribute(name) || '' : '';
  }

  function tagOf(node) {
    return String(node && node.tagName || '').toUpperCase();
  }

  function typeOf(node) {
    return String(attr(node, 'type') || node.type || '').toLowerCase();
  }

  // The autocomplete detail token without section, purpose and phone/email qualifiers: a kind,
  // SENSITIVE (cc-*, one-time-code), 'new-password', 'current-password', or '' (no usable token).
  function autocompleteToken(node) {
    const tokens = attr(node, 'autocomplete').trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (tokens.length && /^section-/.test(tokens[0])) tokens.shift();
    if (tokens[0] === 'shipping' || tokens[0] === 'billing') tokens.shift();
    if (['home', 'work', 'mobile', 'fax', 'pager'].includes(tokens[0]) && tokens.length > 1) tokens.shift();
    const token = tokens.length === 1 ? tokens[0] : '';
    if (/^cc-/.test(token) || token === 'one-time-code' || /^transaction-/.test(token)) return SENSITIVE;
    return token;
  }

  function usable(node, isVisible) {
    if (!node || node.isConnected === false || node.disabled || node.readOnly) return false;
    const tag = tagOf(node);
    if (tag === 'SELECT') {
      if (node.multiple) return false;
    } else if (tag !== 'INPUT' || !FIELD_TYPES.has(typeOf(node))) {
      return false;
    }
    if (typeof node.closest === 'function' && node.closest('[hidden], [inert]')) return false;
    return isVisible(node);
  }

  function usablePassword(node, isVisible) {
    return !!node && tagOf(node) === 'INPUT' && typeOf(node) === 'password' && node.isConnected !== false
      && !node.disabled && !node.readOnly && isVisible(node);
  }

  function cleanText(text) {
    return Array.from(String(text || '').replace(/\s+/g, ' ').trim()).filter((c) => !UNSAFE_CHAR.test(c)).join('');
  }

  // A readable name for a field: its label, aria-label, placeholder or name, at most 64 characters.
  function readableLabel(node) {
    const labels = Array.from(node.labels || [], (label) => label.textContent || '');
    for (const text of [...labels, attr(node, 'aria-label'), attr(node, 'placeholder'), attr(node, 'name')]) {
      const clean = cleanText(text).replace(/[\s:*]+$/, '').trim();
      if (clean) return Array.from(clean).slice(0, MAX_LABEL).join('').trim();
    }
    return '';
  }

  function hasReservedPrefix(label) {
    const lowered = label.toLowerCase();
    return RESERVED_LABEL_PREFIXES.some((prefix) => lowered.startsWith(prefix));
  }

  // The kind of one usable field, or null when it must be left alone.
  function classify(node) {
    const token = autocompleteToken(node);
    if (token === SENSITIVE || token === 'new-password' || token === 'current-password') return null;
    const explicit = Object.hasOwn(AUTOCOMPLETE_KINDS, token) ? AUTOCOMPLETE_KINDS[token] : null;
    if (explicit) return explicit;
    if (formDetection.isSensitiveFieldHint(node)) return null;
    const type = typeOf(node);
    if (type === 'email') return 'email';
    if (type === 'tel') return 'phone';
    const hint = formDetection.addressFieldHint(node);
    if (hint && Object.hasOwn(ADDRESS_HINT_KINDS, hint.token)) return ADDRESS_HINT_KINDS[hint.token];
    const identity = formDetection.identityFieldHint(node);
    if (identity) return identity;
    return readableLabel(node) ? 'other' : null;
  }

  function isActionControl(node) {
    const tag = tagOf(node);
    const type = typeOf(node);
    if (tag === 'INPUT') return type === 'submit' || type === 'button';
    return tag === 'BUTTON' || attr(node, 'role') === 'button';
  }

  function actionText(node) {
    return [node.textContent, tagOf(node) === 'INPUT' ? node.value : '', attr(node, 'aria-label'), attr(node, 'title')]
      .filter(Boolean).join(' ');
  }

  // Whether a click on `node` submits a registration: a submit button, or any button whose text
  // says "Sign up", "Register", "Create account" (localized).
  function isSubmitControl(node) {
    if (!node || !isActionControl(node)) return false;
    const type = typeOf(node);
    if (tagOf(node) === 'BUTTON' && (type === 'submit' || (type === '' && node.form))) return true;
    if (tagOf(node) === 'INPUT' && type === 'submit') return true;
    return formDetection.isRegistrationActionText(actionText(node));
  }

  function ownerOf(node, doc) {
    return node.form || doc;
  }

  // Every registration form of `doc`, each `{ owner, fields, newPassword }` where `fields` is
  // `[{ node, kind, label }]` in document order and `newPassword` the field the new password is
  // typed into (null for a form without one). `options.isVisible` replaces the visibility check.
  function find(doc, options) {
    const isVisible = (options && options.isVisible) || formDetection.isFieldVisible;
    const groups = new Map();
    const groupOf = (owner) => {
      if (!groups.has(owner)) groups.set(owner, { owner, fields: [], passwords: [], action: false });
      return groups.get(owner);
    };
    for (const node of doc.querySelectorAll('input, select')) {
      if (usablePassword(node, isVisible)) {
        groupOf(ownerOf(node, doc)).passwords.push(node);
        continue;
      }
      if (!usable(node, isVisible)) continue;
      const kind = classify(node);
      if (!kind) continue;
      groupOf(ownerOf(node, doc)).fields.push({ node, kind, label: readableLabel(node) });
    }
    for (const node of doc.querySelectorAll('button, input[type="submit"], input[type="button"], [role="button"]')) {
      const owner = ownerOf(node, doc);
      if (groups.has(owner) && formDetection.isRegistrationActionText(actionText(node))) groups.get(owner).action = true;
    }
    const forms = [];
    for (const group of groups.values()) {
      const newPassword = newPasswordOf(group);
      const kinds = new Set(group.fields.map((field) => field.kind).filter((kind) => kind !== 'other'));
      const passwordless = group.passwords.length === 0 && kinds.size >= 2 && group.fields.length >= 3;
      if (newPassword || passwordless) forms.push({ owner: group.owner, fields: group.fields, newPassword });
    }
    return forms;
  }

  function newPasswordOf(group) {
    const passwords = group.passwords;
    if (passwords.some((node) => autocompleteToken(node) === 'current-password')) return null;
    const explicit = passwords.find((node) => autocompleteToken(node) === 'new-password');
    if (explicit) return explicit;
    // Two unhinted fields are "password + confirm"; three or more are a change-password form.
    if (passwords.length === 2) return passwords[0];
    if (passwords.length !== 1) return null;
    const person = group.fields.some((field) => PERSON_KINDS.has(field.kind));
    return group.action || person ? passwords[0] : null;
  }

  // The registration form `node` belongs to (a field, a button, or the form element itself).
  function formFor(doc, node, options) {
    if (!node) return null;
    const owner = tagOf(node) === 'FORM' ? node : (node.form || (typeof node.closest === 'function' && node.closest('form')) || doc);
    return find(doc, options).find((form) => form.owner === owner) || null;
  }

  // The fields of `form` an Address entry can fill (the inline identity icon goes on these).
  function fillableFields(form) {
    return form.fields.filter((field) => Object.hasOwn(FILL_TOKENS, field.kind));
  }

  // Fills every empty fillable field of `form` from `address` (the bridge's ExtensionAddress),
  // with input and change events through `setValue`. A country goes first, so a region select it
  // repopulates is matched afterwards. Returns how many fields were written.
  function fill(form, address, setValue) {
    if (!form || !address || typeof address !== 'object') return 0;
    const profile = addressForm.withNameParts(address);
    const controls = fillableFields(form).map((field) => ({
      node: field.node,
      token: field.kind === 'country' && autocompleteToken(field.node) === 'country' ? 'country' : FILL_TOKENS[field.kind],
    }));
    const region = (control) => control.token === 'address-level1' && tagOf(control.node) === 'SELECT';
    let filled = 0;
    for (const control of [...controls.filter((c) => !region(c)), ...controls.filter(region)]) {
      if (control.node.isConnected === false || !addressForm.fillable(control.node)) continue;
      const value = addressForm.valueFor(control, profile);
      if (value === null || value === control.node.value) continue;
      setValue(control.node, value);
      filled += 1;
    }
    return filled;
  }

  function valueOf(node) {
    if (tagOf(node) === 'SELECT') {
      const option = Array.from(node.options || []).find((candidate) => candidate.selected);
      if (!option || option.disabled || !String(option.value || '').trim()) return '';
      return cleanText(option.textContent || option.value);
    }
    return String(node.value || '').trim();
  }

  function cleanLine(value, max) {
    return typeof value === 'string' && Array.from(value).length <= max && !UNSAFE_CHAR.test(value);
  }

  function folded(value) {
    return value.replace(/\s+/g, ' ').toLowerCase();
  }

  // What a submit of `form` proposes to save: `{ username, password, fields }`, or null when the
  // new password is empty. The username is the username field, else the email; that field is
  // not repeated in `fields`. Values that break a limit, and a second field with the same kind
  // and value (an "email" and its "confirm email"), are left out.
  function capture(form) {
    if (!form || !form.newPassword) return null;
    const password = String(form.newPassword.value || '');
    if (!password) return null;
    const filled = form.fields.map((field) => ({ ...field, value: valueOf(field.node) }))
      .filter((field) => field.value);
    const login = filled.find((field) => field.kind === 'username') || filled.find((field) => field.kind === 'email');
    const username = login ? login.value : '';
    const seen = new Set(login ? [login.kind + '\u0000' + folded(login.value)] : []);
    const fields = [];
    for (const field of filled) {
      if (field === login || fields.length >= MAX_FIELDS) continue;
      if (!cleanLine(field.value, MAX_VALUE)) continue;
      let label = cleanLine(field.label, MAX_LABEL) ? field.label : '';
      if (hasReservedPrefix(label)) label = '';
      if (field.kind === 'other' && !label) continue;
      const key = field.kind === 'other' ? '' : field.kind + '\u0000' + folded(field.value);
      if (key && seen.has(key)) continue;
      if (key) seen.add(key);
      fields.push({ kind: field.kind, label, value: field.value });
    }
    return { username, password, fields };
  }

  return {
    MAX_FIELDS, MAX_LABEL, MAX_VALUE,
    classify, find, formFor, fillableFields, fill, capture, isSubmitControl, readableLabel,
  };
})();
if (typeof module !== 'undefined') module.exports = mypassRegistrationForm;
