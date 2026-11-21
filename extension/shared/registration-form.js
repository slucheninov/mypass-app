// Registration (sign-up) forms: which forms create an account, what each field holds, filling the
// identity and address fields from a MyPass Address entry, and capturing the filled form when the
// user submits it. DOM functions only, no chrome.*: content.js wires them to the inline icon, the
// panel and the save prompt.
//
// A form is a registration form when it has a new-password field and no field for the user's
// current password, or, without any password, at least two identity kinds in three or more
// fields. The new password is a field with autocomplete=new-password, the first of an unhinted
// "password + confirm" pair, or a single unhinted password whose form is submitted by a control
// that says "Sign up", "Register", "Create account" (localized): the control that really submits
// (the submitter, or the form's default button), never just any button somewhere in the form. A
// password field hinted as the current, old or existing password (autocomplete or text) makes the
// form a change-password or re-authentication form, which is never a registration form.
//
// Field kinds: sensitive hints first (card, CVV, one-time, verification or authentication code,
// PIN, captcha, token, bank and national ids, passport, security answer, search, coupon; four
// languages), so no autocomplete token or positive hint can classify such a field; then the
// autocomplete attribute; then the localized hints in form-detection.js. Hidden, invisible,
// disabled (also through a disabled fieldset) and read-only controls, password fields other than
// the one new password, card, one-time-code and other sensitive fields are never classified,
// captured or filled.
//
// Grouping: controls of a <form> belong to that form. Controls outside any form are grouped by
// proximity, never document-wide: two such controls share a group when they sit under the same
// boundary element (a section, dialog, fieldset...) and have a common ancestor within
// PROXIMITY_LEVELS levels, transitively.
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
  const ACTION_SELECTOR = 'button, input[type="submit"], input[type="button"], input[type="image"], [role="button"]';
  // Elements that bound a group of form-less controls, like address-form.js `addressBoundaries`.
  const BOUNDARY_SELECTOR = 'form, dialog, [role="dialog"], [role="form"], section, fieldset, article, aside, main, header, footer, nav';
  // How many levels up two form-less controls may look for a shared container.
  const PROXIMITY_LEVELS = 4;

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

  function attr(node, name) {
    return node && typeof node.getAttribute === 'function' ? node.getAttribute(name) || '' : '';
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

  function editable(node, isVisible) {
    return !!node && node.isConnected !== false && !node.readOnly
      && !formDetection.isEffectivelyDisabled(node) && isVisible(node);
  }

  function usable(node, isVisible) {
    const tag = tagOf(node);
    if (tag === 'SELECT') {
      if (node.multiple) return false;
    } else if (tag !== 'INPUT' || !FIELD_TYPES.has(typeOf(node))) {
      return false;
    }
    if (typeof node.closest === 'function' && node.closest('[hidden], [inert]')) return false;
    return editable(node, isVisible);
  }

  function usablePassword(node, isVisible) {
    return tagOf(node) === 'INPUT' && typeOf(node) === 'password' && editable(node, isVisible)
      && autocompleteToken(node) !== SENSITIVE && !formDetection.isSensitivePasswordHint(node);
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

  // The kind of one usable field, or null when it must be left alone. Sensitive hints are checked
  // before anything that could classify the field, the autocomplete attribute included.
  function classify(node) {
    if (formDetection.isSensitiveFieldHint(node)) return null;
    const token = autocompleteToken(node);
    if (token === SENSITIVE || token === 'new-password' || token === 'current-password') return null;
    const explicit = Object.hasOwn(AUTOCOMPLETE_KINDS, token) ? AUTOCOMPLETE_KINDS[token] : null;
    if (explicit) return explicit;
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
    if (tag === 'INPUT') return type === 'submit' || type === 'button' || type === 'image';
    return tag === 'BUTTON' || attr(node, 'role') === 'button';
  }

  function actionText(node) {
    return [node.textContent, tagOf(node) === 'INPUT' ? node.value : '', attr(node, 'aria-label'), attr(node, 'title')]
      .filter(Boolean).join(' ');
  }

  function hasRegistrationText(node) {
    return !!node && formDetection.isRegistrationActionText(actionText(node));
  }

  // A control whose activation submits its form natively (and so runs the browser's constraint
  // validation before the `submit` event): a submit button or a submit/image input of a form.
  function isNativeSubmitControl(node) {
    if (!node || !node.form) return false;
    const tag = tagOf(node);
    const type = typeOf(node);
    if (tag === 'BUTTON') return type === 'submit' || type === '';
    return tag === 'INPUT' && (type === 'submit' || type === 'image');
  }

  // Whether `node` submits a registration: a native submit control, or a button whose text says
  // "Sign up", "Register", "Create account" (localized).
  function isSubmitControl(node) {
    if (!node || !isActionControl(node)) return false;
    return isNativeSubmitControl(node) || hasRegistrationText(node);
  }

  // A click on `node` that may stand for a registration submit: only a script-driven control that
  // says "Sign up" (localized). A native submit control is never one: its form's `submit` event
  // follows only once the browser's validation passed, and that event is what counts.
  function isScriptSubmitControl(node) {
    return !!node && isActionControl(node) && !isNativeSubmitControl(node) && hasRegistrationText(node);
  }

  function formOf(node) {
    if (node.form) return node.form;
    if (['INPUT', 'SELECT', 'TEXTAREA', 'BUTTON'].includes(tagOf(node))) return null;
    return typeof node.closest === 'function' ? node.closest('form') : null;
  }

  function parentsOf(node, levels) {
    const path = [];
    for (let parent = node.parentElement; parent && path.length < levels; parent = parent.parentElement) {
      if (tagOf(parent) === 'BODY' || tagOf(parent) === 'HTML') break;
      path.push(parent);
    }
    return path;
  }

  function boundaryOf(node) {
    return typeof node.closest === 'function' ? node.closest(BOUNDARY_SELECTOR) : null;
  }

  // The owner of every entry: its form, or for form-less controls a proximity group key shared by
  // the controls under the same boundary with a common ancestor within PROXIMITY_LEVELS levels.
  function assignOwners(entries) {
    const loose = [];
    for (const entry of entries) {
      entry.owner = formOf(entry.node);
      if (!entry.owner) loose.push(entry);
    }
    const parent = loose.map((_, i) => i);
    const root = (i) => {
      while (parent[i] !== i) {
        parent[i] = parent[parent[i]];
        i = parent[i];
      }
      return i;
    };
    const facts = loose.map((entry) => ({ boundary: boundaryOf(entry.node), path: parentsOf(entry.node, PROXIMITY_LEVELS) }));
    for (let i = 0; i < loose.length; i += 1) {
      for (let j = i + 1; j < loose.length; j += 1) {
        if (facts[i].boundary !== facts[j].boundary) continue;
        if (!facts[i].path.some((node) => facts[j].path.includes(node))) continue;
        parent[root(j)] = root(i);
      }
    }
    const keys = new Map();
    loose.forEach((entry, i) => {
      const key = root(i);
      if (!keys.has(key)) keys.set(key, { proximityGroup: true });
      entry.owner = keys.get(key);
    });
  }

  // Every registration form of `doc`, each `{ owner, fields, newPassword, actions }` where
  // `fields` is `[{ node, kind, label }]` in document order and `newPassword` the field the new
  // password is typed into (null for a form without one). `owner` is the <form> element, or an
  // opaque key of a proximity group (only stable within one call).
  // `options.isVisible` replaces the visibility check; `options.submitter` is the control that
  // submitted (a submit event's submitter, or the clicked script control): its own text is then
  // the only "Sign up" evidence of its form.
  function find(doc, options) {
    const isVisible = (options && options.isVisible) || formDetection.isControlShown;
    const submitter = (options && options.submitter) || null;
    const entries = [];
    for (const node of doc.querySelectorAll('input, select')) {
      if (usablePassword(node, isVisible)) {
        entries.push({ node, role: 'password' });
        continue;
      }
      if (!usable(node, isVisible)) continue;
      const kind = classify(node);
      if (kind) entries.push({ node, role: 'field', kind, label: readableLabel(node) });
    }
    for (const node of doc.querySelectorAll(ACTION_SELECTOR)) {
      if (node === submitter || isActionControl(node)) entries.push({ node, role: 'action' });
    }
    if (submitter && !entries.some((entry) => entry.node === submitter)) entries.push({ node: submitter, role: 'action' });
    assignOwners(entries);
    const groups = new Map();
    for (const entry of entries) {
      if (!groups.has(entry.owner)) groups.set(entry.owner, { owner: entry.owner, fields: [], passwords: [], actions: [] });
      const group = groups.get(entry.owner);
      if (entry.role === 'password') group.passwords.push(entry.node);
      else if (entry.role === 'field') group.fields.push({ node: entry.node, kind: entry.kind, label: entry.label });
      else group.actions.push(entry.node);
    }
    const forms = [];
    for (const group of groups.values()) {
      const newPassword = newPasswordOf(group, registrationEvidence(group, submitter, isVisible));
      const kinds = new Set(group.fields.map((field) => field.kind).filter((kind) => kind !== 'other'));
      const passwordless = group.passwords.length === 0 && kinds.size >= 2 && group.fields.length >= 3;
      if (newPassword || passwordless) {
        forms.push({ owner: group.owner, fields: group.fields, newPassword, actions: group.actions });
      }
    }
    return forms;
  }

  // Whether the control that submits `group` says "Sign up": the submitter when it belongs to the
  // group; otherwise a form's default button (its first native submit control), or, for a
  // proximity group, which has no native submit, one of its visible buttons.
  function registrationEvidence(group, submitter, isVisible) {
    if (submitter && group.actions.includes(submitter)) return hasRegistrationText(submitter);
    if (group.owner && group.owner.proximityGroup) {
      return group.actions.some((node) => isVisible(node) && hasRegistrationText(node));
    }
    const defaultButton = group.actions.find(isNativeSubmitControl);
    return !!defaultButton && hasRegistrationText(defaultButton);
  }

  function isCurrentPassword(node) {
    return autocompleteToken(node) === 'current-password' || formDetection.isCurrentPasswordHint(node);
  }

  function newPasswordOf(group, signUpAction) {
    const passwords = group.passwords;
    // A field for the current password: a change-password or sign-in form, whatever else it has.
    if (passwords.some(isCurrentPassword)) return null;
    const explicit = passwords.find((node) => autocompleteToken(node) === 'new-password');
    if (explicit) return explicit;
    // Two unhinted fields are "password + confirm"; three or more are a change-password form.
    if (passwords.length === 2) return passwords[0];
    if (passwords.length !== 1) return null;
    // One unhinted password is a login unless the control that submits says "Sign up".
    return signUpAction ? passwords[0] : null;
  }

  function contains(form, node) {
    return form.owner === node || form.newPassword === node || form.actions.includes(node)
      || form.fields.some((field) => field.node === node);
  }

  // The registration form `node` belongs to (a field, a password, a button, or the form element
  // itself), with the same options as `find`.
  function formFor(doc, node, options) {
    if (!node) return null;
    if (tagOf(node) !== 'FORM' && !node.form && typeof node.closest === 'function' && node.closest('form')
      && !['INPUT', 'SELECT', 'TEXTAREA', 'BUTTON'].includes(tagOf(node))) {
      node = node.closest('form');
    }
    return find(doc, options).find((form) => contains(form, node)) || null;
  }

  // The fields of `form` an Address entry can fill (the inline identity icon goes on these).
  function fillableFields(form) {
    return form.fields.filter((field) => Object.hasOwn(FILL_TOKENS, field.kind));
  }

  function tokenFor(field) {
    return field.kind === 'country' && autocompleteToken(field.node) === 'country' ? 'country' : FILL_TOKENS[field.kind];
  }

  // What the user chose to fill, taken when they click an Address entry in the panel of `input`:
  // the exact controls of the registration form `input` belongs to, with their kinds. Null when
  // `input` is in no registration form. Node references only, never values.
  function selectionFor(doc, input, options) {
    const form = formFor(doc, input, options);
    if (!form || !form.fields.some((field) => field.node === input)) return null;
    return {
      anchor: input,
      formElement: input.form || null,
      controls: fillableFields(form).map((field) => ({ node: field.node, kind: field.kind })),
    };
  }

  // Fills the empty controls of `selection` from `address` (the bridge's ExtensionAddress) through
  // `setValue`, which dispatches input and change events. Every control is checked again right
  // before its write, because a page's handlers may move, replace, hide or retype controls while
  // the fill runs: it must still be connected, in the same form (or the same proximity group as
  // the clicked field), visible and enabled, of the same kind, and empty. A control that is not is
  // skipped; `options.stillCurrent()` false stops the fill. A country goes first, so a region
  // select it repopulates is matched afterwards. Returns how many controls were written.
  function fillSelection(doc, selection, address, setValue, options) {
    if (!selection || !address || typeof address !== 'object') return 0;
    const stillCurrent = options && options.stillCurrent;
    const findOptions = options && options.isVisible ? { isVisible: options.isVisible } : undefined;
    const profile = addressForm.withNameParts(address);
    const region = (control) => control.kind === 'region' && tagOf(control.node) === 'SELECT';
    const order = [...selection.controls.filter((c) => !region(c)), ...selection.controls.filter(region)];
    let filled = 0;
    for (const control of order) {
      if (stillCurrent && !stillCurrent()) break;
      const node = control.node;
      if (node.isConnected === false || (node.form || null) !== selection.formElement) continue;
      if (selection.anchor.isConnected === false) break;
      const current = formFor(doc, selection.anchor, findOptions);
      const field = current && current.fields.find((candidate) => candidate.node === node);
      if (!field || field.kind !== control.kind || !Object.hasOwn(FILL_TOKENS, field.kind)) continue;
      if (!addressForm.fillable(node)) continue;
      const value = addressForm.valueFor({ node, token: tokenFor(field) }, profile);
      if (value === null || value === node.value) continue;
      setValue(node, value);
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

  // What a submit of `form` proposes to save: `{ username, usernameIsEmail, password, fields }`,
  // or null when the new password is empty. The username is the username field, else the email
  // (then `usernameIsEmail` is true, so an Address entry made from the form still gets the
  // email); that field is not repeated in `fields`. Values that break a limit, and a second field
  // with the same kind and value (an "email" and its "confirm email"), are left out.
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
    return { username, usernameIsEmail: !!login && login.kind === 'email', password, fields };
  }

  return {
    MAX_FIELDS, MAX_LABEL, MAX_VALUE,
    classify, find, formFor, fillableFields, selectionFor, fillSelection, capture,
    isSubmitControl, isScriptSubmitControl, isNativeSubmitControl, readableLabel,
  };
})();
if (typeof module !== 'undefined') module.exports = mypassRegistrationForm;
