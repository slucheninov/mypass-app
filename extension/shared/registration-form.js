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
// A MyPass Person entry fills the person kinds too (middle name, a whole or split birthday) and,
// with the person's names and email, the person's linked address: `personProfile` merges the two
// into one fill profile. A birthday goes into a date input as an ISO date, into a text input in
// the format its placeholder shows (DD.MM.YYYY, MM/DD/YYYY, TT.MM.JJJJ, the Cyrillic DD.MM.YYYY...) or else
// ISO, and into a split day, month or year select by number or (months) by name in en/de/uk/ru.
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
  // How many levels up two form-less controls may look for a shared container. Component
  // libraries nest each field deeply: on goldcar.es's booking page the first name and the phone
  // number meet seven and six levels up, the email and the phone eight and five, with no <form>
  // around them. The boundary rule (same section, dialog, fieldset...) is what keeps unrelated
  // controls apart; this only has to reach a field's own row of the layout.
  const PROXIMITY_LEVELS = 10;

  const AUTOCOMPLETE_KINDS = {
    name: 'name', 'given-name': 'givenName', 'family-name': 'familyName', email: 'email',
    tel: 'phone', 'tel-national': 'phone', bday: 'birthday', username: 'username', nickname: 'username',
    organization: 'organization', 'street-address': 'addressLine1', 'address-line1': 'addressLine1',
    'address-line2': 'addressLine2', 'address-level2': 'city', 'address-level1': 'region',
    'postal-code': 'postalCode', country: 'country', 'country-name': 'country',
    'additional-name': 'middleName', 'bday-day': 'bdayDay', 'bday-month': 'bdayMonth', 'bday-year': 'bdayYear',
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

  // The kinds a <textarea> may have: the address parts address-form.js fills into one (a street
  // address textarea takes both address lines). Any other textarea (a bio, a comment) is left out.
  const TEXTAREA_KINDS = new Set(['organization', 'addressLine1', 'addressLine2', 'city', 'region', 'postalCode', 'country']);
  // The kinds only a person fills, from the profile's `middleName` and `birthDate`.
  const PERSON_KINDS = new Set(['middleName', 'birthday', 'bdayDay', 'bdayMonth', 'bdayYear']);
  // The kinds of a person's own values: a form with one of these can take a person.
  const PERSON_FORM_KINDS = new Set(['name', 'givenName', 'familyName', 'middleName', 'email', ...PERSON_KINDS]);
  // Kinds the bridge's ExtraFieldKind does not know: a capture sends them as 'other' with their label.
  const CAPTURE_AS_OTHER = new Set(['middleName', 'bdayDay', 'bdayMonth', 'bdayYear']);
  const LOCALES = ['en', 'de', 'uk', 'ru'];

  function isFillKind(kind) {
    return Object.hasOwn(FILL_TOKENS, kind) || PERSON_KINDS.has(kind);
  }

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
    } else if (tag !== 'TEXTAREA' && (tag !== 'INPUT' || !FIELD_TYPES.has(typeOf(node)))) {
      return false;
    }
    if (typeof node.closest === 'function' && node.closest('[hidden], [inert]')) return false;
    return editable(node, isVisible);
  }

  // A field that takes a password: type=password, a text field marked autocomplete=new-password,
  // or one the caller remembers as a password (`options.wasPassword`: content.js keeps every field
  // it saw as type=password, so a field a show-password toggle turned into text still counts).
  function isPasswordLike(node, options) {
    if (tagOf(node) !== 'INPUT') return false;
    const type = typeOf(node);
    if (type === 'password') return true;
    if (type !== 'text' && type !== '') return false;
    if (autocompleteToken(node) === 'new-password') return true;
    return !!(options && typeof options.wasPassword === 'function' && options.wasPassword(node));
  }

  function usablePassword(node, isVisible, options) {
    return isPasswordLike(node, options) && editable(node, isVisible)
      && autocompleteToken(node) !== SENSITIVE && !formDetection.isSensitivePasswordHint(node);
  }

  function cleanText(text) {
    return Array.from(String(text || '').replace(/\s+/g, ' ').trim()).filter((c) => !UNSAFE_CHAR.test(c)).join('');
  }

  // A readable name for a field: its label, aria-label, placeholder or name, at most 64 characters.
  function readableLabel(node) {
    const labels = formDetection.labelTexts(node);
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

  // Every registration form of `doc`, each `{ owner, fields, newPassword, passwords, actions }`
  // where `fields` is `[{ node, kind, label }]` in document order, `newPassword` the field the new
  // password is typed into (null for a form without one) and `passwords` every password field
  // (the new one, its confirm field). `owner` is the <form> element, or an
  // opaque key of a proximity group (only stable within one call).
  // `options.isVisible` replaces the visibility check; `options.wasPassword` names fields to treat
  // as passwords although they are text now (see isPasswordLike); `options.submitter` is the control that
  // submitted (a submit event's submitter, or the clicked script control): its own text is then
  // the only "Sign up" evidence of its form.
  function find(doc, options) {
    const isVisible = (options && options.isVisible) || formDetection.isControlShown;
    const submitter = (options && options.submitter) || null;
    const entries = [];
    for (const node of doc.querySelectorAll('input, select, textarea')) {
      if (usablePassword(node, isVisible, options)) {
        entries.push({ node, role: 'password' });
        continue;
      }
      if (!usable(node, isVisible)) continue;
      const kind = classify(node);
      if (kind && (tagOf(node) !== 'TEXTAREA' || TEXTAREA_KINDS.has(kind))) {
        entries.push({ node, role: 'field', kind, label: readableLabel(node) });
      }
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
        forms.push({ owner: group.owner, fields: group.fields, newPassword, passwords: group.passwords, actions: group.actions });
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

  // The fields a password generated for `input` goes into: `input` and the other new-password
  // fields of its form (its "confirm" or "repeat" field), so the two always match. Grouped like
  // `find` (the same <form>, or form-less fields under one boundary close together), so another
  // form on the page is never touched, and a field for the current password never is either: a
  // current-password `input` gets the value alone. A field is a new-password one when it says so
  // (autocomplete=new-password, confirm wording) or by its place (form-detection.js
  // `isNewPasswordCandidate`: both of two unhinted fields, all but the first of three or more).
  // `options` takes `isVisible` and `wasPassword` as in `find`. Node references only.
  function newPasswordTargets(doc, input, options) {
    if (!input) return [];
    const isVisible = (options && options.isVisible) || formDetection.isControlShown;
    const entries = [];
    for (const node of doc.querySelectorAll('input')) {
      if (node === input || usablePassword(node, isVisible, options)) entries.push({ node, role: 'password' });
    }
    if (!entries.some((entry) => entry.node === input)) return [input];
    assignOwners(entries);
    const owner = entries.find((entry) => entry.node === input).owner;
    const group = entries.filter((entry) => entry.owner === owner).map((entry) => entry.node);
    if (isCurrentPassword(input)) return [input];
    return group.filter((node) => node === input || (!isCurrentPassword(node)
      && (autocompleteToken(node) === 'new-password' || formDetection.isConfirmPasswordHint(node)
        || formDetection.isNewPasswordCandidate(node, group))));
  }

  function contains(form, node) {
    return form.owner === node || form.newPassword === node || form.actions.includes(node)
      || (form.passwords || []).includes(node)
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

  // The fields of `form` an Address or Person entry can fill (the inline identity icon goes on these).
  function fillableFields(form) {
    return form.fields.filter((field) => isFillKind(field.kind));
  }

  function tokenFor(field) {
    // A street address textarea takes both lines, one per row (address-form.js `valueFor`),
    // unless its autocomplete names the first line alone.
    if (field.kind === 'addressLine1' && tagOf(field.node) === 'TEXTAREA'
      && autocompleteToken(field.node) !== 'address-line1') return 'street-address';
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

  // The registration forms of `doc` a person can fill (one of the person's own kinds), each as a
  // selection like `selectionFor` anchored at its first fillable field. Node references only.
  function personSelections(doc, options) {
    return find(doc, options)
      .map((form) => fillableFields(form))
      .filter((fields) => fields.some((field) => PERSON_FORM_KINDS.has(field.kind)))
      .map((fields) => ({
        anchor: fields[0].node,
        formElement: fields[0].node.form || null,
        controls: fields.map((field) => ({ node: field.node, kind: field.kind })),
      }));
  }

  function text(value) {
    return typeof value === 'string' ? value : '';
  }

  // One fill profile from a person (the bridge's ExtensionPerson) and its address (or null): the
  // address values, with the person's names, email, middle name and birth date on top. A person
  // with a given or family name replaces all three name values, so names never mix.
  function personProfile(person, address) {
    const base = address && typeof address === 'object' ? { ...address } : {};
    if (!person || typeof person !== 'object') return base;
    const given = text(person.givenName).trim();
    const family = text(person.familyName).trim();
    const named = !!(given || family);
    return {
      ...base,
      fullName: named ? (text(person.fullName).trim() || [given, family].filter(Boolean).join(' ')) : text(base.fullName),
      givenName: named ? given : text(base.givenName),
      familyName: named ? family : text(base.familyName),
      email: text(person.email).trim() ? text(person.email) : text(base.email),
      middleName: text(person.middleName),
      birthDate: text(person.birthDate),
    };
  }

  // `{ year, month, day }` of a real ISO `YYYY-MM-DD` date, or null.
  function parseIsoDate(value) {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text(value));
    if (!match) return null;
    const [year, month, day] = match.slice(1).map(Number);
    // Checked by hand: Date.UTC maps the years 0 to 99 to 1900 to 1999.
    const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
    const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
    if (year < 1 || !days || day < 1 || day > days) return null;
    return { year, month, day };
  }

  function foldName(value) {
    return String(value || '').trim().toLowerCase().replace(/\.$/, '');
  }

  // The names of month `month` (1-12) in the four languages: long, short and the genitive form
  // (the uk and ru month names after a day), lower-cased without a trailing dot.
  const monthNameCache = new Map();
  function monthNames(month) {
    if (monthNameCache.has(month)) return monthNameCache.get(month);
    const names = new Set();
    const date = new Date(Date.UTC(2000, month - 1, 15));
    for (const locale of LOCALES) {
      try {
        for (const style of ['long', 'short']) {
          names.add(foldName(new Intl.DateTimeFormat(locale, { month: style, timeZone: 'UTC' }).format(date)));
          const part = new Intl.DateTimeFormat(locale, { day: 'numeric', month: style, timeZone: 'UTC' })
            .formatToParts(date).find((item) => item.type === 'month');
          if (part) names.add(foldName(part.value));
        }
      } catch {
        // A runtime without that locale: the numeric match still works.
      }
    }
    names.delete('');
    monthNameCache.set(month, names);
    return names;
  }

  // The one enabled option of `node` that stands for `part` (a day, month or year number); null
  // when none or several do. A menu whose values are all numbers is matched by value only (a month
  // menu numbered 0 to 11, with no 12, is zero-based); any other menu by its text, a number or a
  // month name, or by a month name in its values.
  function partOption(node, kind, part) {
    const number = (value) => (/^\d+$/.test(String(value).trim()) ? Number(String(value).trim()) : NaN);
    const options = Array.from(node.options || []).filter((option) => !option.disabled
      && String(option.value || '').trim() !== '');
    const values = options.map((option) => number(option.value));
    let matches;
    if (options.length && values.every((value) => !Number.isNaN(value))) {
      const zeroBased = kind === 'bdayMonth' && values.includes(0) && !values.includes(12);
      const wanted = zeroBased ? part - 1 : part;
      matches = options.filter((option, index) => values[index] === wanted);
    } else {
      const names = kind === 'bdayMonth' ? monthNames(part) : new Set();
      matches = options.filter((option) => number(option.textContent || '') === part
        || names.has(foldName(option.textContent)) || names.has(foldName(option.value)));
    }
    return matches.length === 1 ? matches[0].value : null;
  }

  // The one enabled option whose value or text is `value` (case-insensitive), or null.
  function textOption(node, value) {
    const wanted = foldName(value);
    const matches = Array.from(node.options || []).filter((option) => !option.disabled
      && (foldName(option.value) === wanted || foldName(option.textContent) === wanted));
    return matches.length === 1 ? matches[0].value : null;
  }

  // The date format a text input shows in its placeholder, title or label, as `{ order, separator,
  // widths }`, or null. Letters: d/t (and Cyrillic de) day, m (and em) month,
  // y/j (and Cyrillic ge, er) year.
  function dateFormatOf(node) {
    const labels = Array.from(node.labels || [], (label) => label.textContent || '');
    for (const hint of [attr(node, 'placeholder'), attr(node, 'title'), attr(node, 'aria-label'), ...labels]) {
      // The separator may carry spaces ("DD / MM / YYYY"); it is written back exactly as shown,
      // which a masked input reformats from the digits anyway.
      const found = /(?:^|[^\p{L}])(([dt\u0434m\u043cyj\u0433\u0440])\2*)(\s*[./-]\s*|\s+)(([dt\u0434m\u043cyj\u0433\u0440])\5*)\3(([dt\u0434m\u043cyj\u0433\u0440])\7*)(?:$|[^\p{L}])/iu
        .exec(String(hint || ''));
      if (!found) continue;
      const partOf = (letter) => ({ d: 'day', t: 'day', '\u0434': 'day', m: 'month', '\u043c': 'month' })[letter.toLowerCase()] || 'year';
      const order = [found[2], found[5], found[7]].map(partOf);
      if (new Set(order).size !== 3) continue;
      return { order, separator: found[3], widths: [found[1], found[4], found[6]].map((group) => group.length) };
    }
    return null;
  }

  function birthdayText(node, date) {
    const iso = [String(date.year).padStart(4, '0'), String(date.month).padStart(2, '0'), String(date.day).padStart(2, '0')].join('-');
    if (typeOf(node) === 'date') return iso;
    const format = dateFormatOf(node);
    if (!format) return iso;
    return format.order.map((part, index) => {
      const width = format.widths[index];
      if (part === 'year') return width === 2 ? String(date.year % 100).padStart(2, '0') : String(date.year).padStart(4, '0');
      const value = String(date[part]);
      return width >= 2 ? value.padStart(2, '0') : value;
    }).join(format.separator);
  }

  // The value of a person kind for `node` from `profile`, or null when there is none.
  function personValueFor(node, kind, profile) {
    if (kind === 'middleName') {
      const value = text(profile.middleName).trim();
      if (!value) return null;
      return tagOf(node) === 'SELECT' ? textOption(node, value) : value;
    }
    const date = parseIsoDate(profile.birthDate);
    if (!date) return null;
    if (kind === 'birthday') return tagOf(node) === 'SELECT' ? null : birthdayText(node, date);
    const part = kind === 'bdayDay' ? date.day : kind === 'bdayMonth' ? date.month : date.year;
    if (tagOf(node) === 'SELECT') return partOption(node, kind, part);
    if (typeOf(node) === 'number' || kind === 'bdayYear') return String(part);
    return String(part).padStart(2, '0');
  }

  // Fills the empty controls of `selection` from `address` (the bridge's ExtensionAddress, or a
  // person's `personProfile`) through
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
      if (!field || field.kind !== control.kind || !isFillKind(field.kind)) continue;
      if (!addressForm.fillable(node)) continue;
      let value = PERSON_KINDS.has(field.kind)
        ? personValueFor(node, field.kind, profile)
        : addressForm.valueFor({ node, token: tokenFor(field) }, profile);
      if (value !== null && field.kind === 'phone') value = withoutShownDialCode(node, value);
      if (value === null || value === node.value) continue;
      setValue(node, value);
      filled += 1;
    }
    return filled;
  }

  // The dial code ("48" of "+48") a phone field's own country-code menu shows, or ''. The menu is
  // the one <select> within four wrapper levels of the field; a wrapper holding a second select or
  // another text field ends the search, so a menu of some other field is never read.
  function shownDialCode(node) {
    let scope = node.parentElement;
    for (let level = 0; scope && level < 4; level += 1, scope = scope.parentElement) {
      if (typeof scope.querySelectorAll !== 'function') return '';
      const selects = Array.from(scope.querySelectorAll('select'));
      if (selects.length > 1) return '';
      if (selects.length === 1) {
        const option = Array.from(selects[0].options || []).find((candidate) => candidate.selected);
        const match = option && /\+\s?(\d{1,4})(?!\d)/.exec(String(option.textContent || '') + ' ' + String(option.value || ''));
        return match ? match[1] : '';
      }
      if (scope.querySelectorAll('input, textarea').length > 1) return '';
    }
    return '';
  }

  // A phone number written next to a country-code menu that already shows its dial code loses
  // that code, so "+48 123 456 789" under "+48" becomes "123 456 789". Any other number is kept.
  function withoutShownDialCode(node, value) {
    const code = shownDialCode(node);
    if (!code) return value;
    const match = new RegExp('^\\s*(?:\\+|00)\\s?' + code + '(?!\\d)[\\s().-]*').exec(value);
    const rest = match ? value.slice(match[0].length).trim() : '';
    return rest ? rest : value;
  }

  function valueOf(node) {
    if (tagOf(node) === 'SELECT') {
      const option = Array.from(node.options || []).find((candidate) => candidate.selected);
      if (!option || option.disabled || !String(option.value || '').trim()) return '';
      return cleanText(option.textContent || option.value);
    }
    // A textarea's rows are one line for the vault: no control characters in a captured value.
    if (tagOf(node) === 'TEXTAREA') {
      return String(node.value || '').split(/\r\n|\r|\n/).map((line) => line.trim()).filter(Boolean).join(', ');
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
      if ((field.kind === 'other' || CAPTURE_AS_OTHER.has(field.kind)) && !label) continue;
      const key = field.kind === 'other' ? '' : field.kind + '\u0000' + folded(field.value);
      if (key && seen.has(key)) continue;
      if (key) seen.add(key);
      fields.push({ kind: CAPTURE_AS_OTHER.has(field.kind) ? 'other' : field.kind, label, value: field.value });
    }
    return { username, usernameIsEmail: !!login && login.kind === 'email', password, fields };
  }

  return {
    MAX_FIELDS, MAX_LABEL, MAX_VALUE,
    classify, find, formFor, fillableFields, selectionFor, personSelections, fillSelection, capture,
    newPasswordTargets, isPasswordLike,
    personProfile, parseIsoDate,
    isSubmitControl, isScriptSubmitControl, isNativeSubmitControl, readableLabel,
  };
})();
if (typeof module !== 'undefined') module.exports = mypassRegistrationForm;
