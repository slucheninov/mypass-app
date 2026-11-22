// Registration forms a person fills: the person kinds from autocomplete tokens and from English,
// Ukrainian, Russian and German hints (middle name, birthday, the parts of a split birthday,
// bare part names inside a birthday fieldset), the person profile, the birth date in date
// inputs, formatted text inputs and split selects, and what a submit captures of those kinds.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const registration = require('../shared/registration-form.js');

// A small fake DOM, as in registrationForm.test.js, plus an optional fieldset around a control.
function page() {
  const fields = [];
  const buttons = [];
  const doc = {
    querySelectorAll(selector) {
      return selector.startsWith('input, select') ? fields.slice() : buttons.slice();
    },
  };
  const form = { tagName: 'FORM' };
  function node(tag, attrs, extra) {
    const created = {
      tagName: tag, value: '', isConnected: true, disabled: false, readOnly: false, multiple: false,
      form, labels: [], attrs: { ...attrs },
      getAttribute(name) { return Object.hasOwn(this.attrs, name) ? this.attrs[name] : null; },
      closest(selector) {
        if (selector === 'form') return this.form;
        if (selector === 'fieldset') return this.fieldset || null;
        return null;
      },
      ...extra,
    };
    created.type = tag === 'INPUT' ? (attrs.type || 'text') : '';
    created.name = attrs.name || '';
    created.id = attrs.id || '';
    if (extra && extra.label) created.labels = [{ textContent: extra.label, contains: () => false }];
    return created;
  }
  return {
    doc, form,
    input(attrs = {}, extra = {}) {
      const created = node('INPUT', attrs, extra);
      fields.push(created);
      return created;
    },
    select(attrs, options, extra = {}) {
      const created = node('SELECT', attrs, { options, size: 0, ...extra });
      const chosen = options.find((option) => option.selected);
      created.value = chosen ? chosen.value : '';
      fields.push(created);
      return created;
    },
    password() {
      return this.input({ type: 'password', autocomplete: 'new-password' });
    },
  };
}

const visible = { isVisible: (node) => !node.invisible };
const kinds = (found) => found.fields.map((field) => field.kind);
const write = (node, value) => { node.value = value; };
const option = (value, textContent = value) => ({ value, textContent, selected: false, defaultSelected: false, disabled: false });
const placeholder = (text) => ({ value: '', textContent: text, selected: true, defaultSelected: true, disabled: false });
const legendSet = (text) => ({ tagName: 'FIELDSET', children: [{ tagName: 'LEGEND', textContent: text }] });

const ADA = { fullName: 'Ada Lovelace', givenName: 'Ada', middleName: 'King', familyName: 'Lovelace',
  email: 'ada@example.test', birthDate: '1815-12-10' };
const HOME = { fullName: 'Home Owner', givenName: 'Home', familyName: 'Owner', city: 'London',
  addressLine1: '1 Main St', email: 'home@example.test' };

function fill(p, profile) {
  const [selection] = registration.personSelections(p.doc, visible);
  assert.ok(selection, 'the form takes a person');
  return registration.fillSelection(p.doc, selection, profile, write, visible);
}

test('the autocomplete tokens of a person', () => {
  const p = page();
  p.input({ autocomplete: 'given-name' });
  p.input({ autocomplete: 'additional-name' });
  p.input({ autocomplete: 'family-name' });
  p.input({ autocomplete: 'bday' });
  p.input({ autocomplete: 'bday-day' });
  p.input({ autocomplete: 'bday-month' });
  p.input({ autocomplete: 'bday-year' });
  p.password();
  const [found] = registration.find(p.doc, visible);
  assert.deepEqual(kinds(found), ['givenName', 'middleName', 'familyName', 'birthday', 'bdayDay', 'bdayMonth', 'bdayYear']);
});

test('middle name and birthday hints in four languages', () => {
  const cases = [
    ['Middle name', 'middleName'], ['Patronymic', 'middleName'], ['Zweiter Vorname', 'middleName'],
    ['\u041f\u043e \u0431\u0430\u0442\u044c\u043a\u043e\u0432\u0456', 'middleName'], ['\u041e\u0442\u0447\u0435\u0441\u0442\u0432\u043e', 'middleName'],
    ['Date of birth', 'birthday'], ['Geburtsdatum', 'birthday'], ['\u0414\u0430\u0442\u0430 \u043d\u0430\u0440\u043e\u0434\u0436\u0435\u043d\u043d\u044f', 'birthday'],
    ['\u0414\u0430\u0442\u0430 \u0440\u043e\u0436\u0434\u0435\u043d\u0438\u044f', 'birthday'],
    ['Birth month', 'bdayMonth'], ['Geburtsjahr', 'bdayYear'], ['\u0420\u0456\u043a \u043d\u0430\u0440\u043e\u0434\u0436\u0435\u043d\u043d\u044f', 'bdayYear'],
    ['\u041c\u0435\u0441\u044f\u0446 \u0440\u043e\u0436\u0434\u0435\u043d\u0438\u044f', 'bdayMonth'], ['Day of birth', 'bdayDay'],
  ];
  for (const [label, kind] of cases) {
    const p = page();
    p.input({ type: 'email', name: 'email' });
    p.input({}, { label });
    p.password();
    const [found] = registration.find(p.doc, visible);
    assert.equal(kinds(found)[1], kind, label);
  }
  // name attributes in the usual spellings
  const p = page();
  p.input({ name: 'dob_day' });
  p.input({ name: 'dobMonth' });
  p.input({ name: 'birthDateYear' });
  p.password();
  assert.deepEqual(kinds(registration.find(p.doc, visible)[0]), ['bdayDay', 'bdayMonth', 'bdayYear']);
});

test('bare Day, Month and Year are a split birthday only inside a birthday fieldset', () => {
  for (const [legend, labels] of [
    ['Date of birth', ['Day', 'Month', 'Year']],
    ['Geburtsdatum', ['Tag', 'Monat', 'Jahr']],
    ['\u0414\u0430\u0442\u0430 \u043d\u0430\u0440\u043e\u0434\u0436\u0435\u043d\u043d\u044f', ['\u0414\u0435\u043d\u044c', '\u041c\u0456\u0441\u044f\u0446\u044c', '\u0420\u0456\u043a']],
  ]) {
    const p = page();
    const fieldset = legendSet(legend);
    p.input({ type: 'email', name: 'email' });
    for (const label of labels) p.input({}, { label, fieldset });
    p.password();
    assert.deepEqual(kinds(registration.find(p.doc, visible)[0]), ['email', 'bdayDay', 'bdayMonth', 'bdayYear'], legend);
  }
  const p = page();
  p.input({ type: 'email', name: 'email' });
  p.input({}, { label: 'Day', fieldset: legendSet('Delivery') });
  p.password();
  assert.deepEqual(kinds(registration.find(p.doc, visible)[0]), ['email', 'other'], 'a delivery day is no birthday');
});

test('the person profile: person names win over the address, the address fills the rest', () => {
  const profile = registration.personProfile(ADA, HOME);
  assert.equal(profile.givenName, 'Ada');
  assert.equal(profile.familyName, 'Lovelace');
  assert.equal(profile.fullName, 'Ada Lovelace');
  assert.equal(profile.email, 'ada@example.test');
  assert.equal(profile.city, 'London');
  assert.equal(profile.middleName, 'King');
  const nameless = registration.personProfile({ ...ADA, givenName: '', familyName: '', fullName: '', email: '' }, HOME);
  assert.equal(nameless.givenName, 'Home');
  assert.equal(nameless.email, 'home@example.test');
  assert.equal(registration.personProfile(ADA, null).city, undefined);
  assert.deepEqual(registration.parseIsoDate('1815-12-10'), { year: 1815, month: 12, day: 10 });
  assert.equal(registration.parseIsoDate('2023-02-30'), null);
  assert.equal(registration.parseIsoDate('10.12.1815'), null);
});

test('a person and its address fill names, email, a date input and the address fields', () => {
  const p = page();
  const given = p.input({ autocomplete: 'given-name' });
  const middle = p.input({}, { label: '\u041f\u043e \u0431\u0430\u0442\u044c\u043a\u043e\u0432\u0456' });
  const family = p.input({ autocomplete: 'family-name' });
  const email = p.input({ type: 'email', name: 'email' });
  const birthday = p.input({ type: 'date', name: 'birthday' });
  const city = p.input({ autocomplete: 'address-level2' });
  p.password();
  assert.equal(fill(p, registration.personProfile(ADA, HOME)), 6);
  assert.deepEqual([given.value, middle.value, family.value, email.value, birthday.value, city.value],
    ['Ada', 'King', 'Lovelace', 'ada@example.test', '1815-12-10', 'London']);
});

test('a text birthday follows the format its placeholder or label shows', () => {
  for (const [hint, expected] of [
    [{ placeholder: 'DD.MM.YYYY' }, '10.12.1815'],
    [{ placeholder: 'MM/DD/YYYY' }, '12/10/1815'],
    [{ placeholder: 'TT.MM.JJJJ' }, '10.12.1815'],
    [{ placeholder: '\u0414\u0414.\u041c\u041c.\u0420\u0420\u0420\u0420' }, '10.12.1815'],
    [{ placeholder: '\u0414\u0414.\u041c\u041c.\u0413\u0413\u0413\u0413' }, '10.12.1815'],
    [{ placeholder: 'D/M/YY' }, '10/12/15'],
    [{}, '1815-12-10'],
  ]) {
    const p = page();
    p.input({ type: 'email', name: 'email' });
    const birthday = p.input({ name: 'birthdate', ...hint });
    p.password();
    fill(p, registration.personProfile(ADA, null));
    assert.equal(birthday.value, expected, JSON.stringify(hint));
  }
});

test('split birthday selects match numbers and month names; inputs take the numbers', () => {
  const days = [placeholder('Day'), ...Array.from({ length: 31 }, (_, i) => option(String(i + 1)))];
  for (const months of [
    ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'],
    ['\u0441\u0456\u0447\u043d\u044f', '\u043b\u044e\u0442\u043e\u0433\u043e', '\u0431\u0435\u0440\u0435\u0437\u043d\u044f', '\u043a\u0432\u0456\u0442\u043d\u044f', '\u0442\u0440\u0430\u0432\u043d\u044f', '\u0447\u0435\u0440\u0432\u043d\u044f', '\u043b\u0438\u043f\u043d\u044f', '\u0441\u0435\u0440\u043f\u043d\u044f', '\u0432\u0435\u0440\u0435\u0441\u043d\u044f', '\u0436\u043e\u0432\u0442\u043d\u044f', '\u043b\u0438\u0441\u0442\u043e\u043f\u0430\u0434\u0430', '\u0433\u0440\u0443\u0434\u043d\u044f'],
    ['Jan.', 'Feb.', 'M\u00e4r.', 'Apr.', 'Mai', 'Juni', 'Juli', 'Aug.', 'Sept.', 'Okt.', 'Nov.', 'Dez.'],
  ]) {
    const p = page();
    p.input({ type: 'email', name: 'email' });
    const day = p.select({ autocomplete: 'bday-day' }, days.map((item) => ({ ...item })));
    const month = p.select({ autocomplete: 'bday-month' }, [placeholder('Month'),
      ...months.map((name, index) => option('m' + (index + 1), name))]);
    const year = p.input({ autocomplete: 'bday-year' });
    p.password();
    assert.equal(fill(p, registration.personProfile(ADA, null)), 4, months[0]);
    assert.deepEqual([day.value, month.value, year.value], ['10', 'm12', '1815'], months[0]);
  }
  const p = page();
  p.input({ type: 'email', name: 'email' });
  const day = p.input({ autocomplete: 'bday-day' });
  const month = p.input({ autocomplete: 'bday-month', type: 'number' });
  p.password();
  fill(p, registration.personProfile({ ...ADA, birthDate: '1990-03-07' }, null));
  assert.deepEqual([day.value, month.value], ['07', '3']);
});

test('no birth date or middle name leaves those fields alone; an address-only form takes no person', () => {
  const p = page();
  p.input({ type: 'email', name: 'email' });
  const middle = p.input({ autocomplete: 'additional-name' });
  const birthday = p.input({ autocomplete: 'bday' });
  p.password();
  fill(p, registration.personProfile({ ...ADA, middleName: '', birthDate: '' }, null));
  assert.deepEqual([middle.value, birthday.value], ['', '']);
  const q = page();
  q.input({ autocomplete: 'address-level2' });
  q.input({ autocomplete: 'postal-code' });
  q.password();
  assert.deepEqual(registration.personSelections(q.doc, visible), []);
});

test('a submit captures the middle name and split birthday parts as labelled other fields', () => {
  const p = page();
  p.input({ type: 'email', name: 'email' }, { value: 'ada@example.test' });
  p.input({ autocomplete: 'additional-name' }, { label: 'Middle name', value: 'King' });
  p.input({ autocomplete: 'bday-day' }, { value: '10' });
  p.input({ autocomplete: 'bday' }, { label: 'Birthday', value: '1815-12-10' });
  const password = p.password();
  password.value = 'pw-1';
  const captured = registration.capture(registration.find(p.doc, visible)[0]);
  assert.deepEqual(captured.fields, [
    { kind: 'other', label: 'Middle name', value: 'King' },
    { kind: 'birthday', label: 'Birthday', value: '1815-12-10' },
  ], 'an unlabelled day part is left out');
});
