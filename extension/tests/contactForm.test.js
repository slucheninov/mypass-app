// Contact and checkout forms without a password (a car rental's "Driver details"): the person
// fill must find them from their labels however deeply a component library nests each field,
// write the birthday in the format the field shows, and drop the dial code the page already
// shows in its own country-code menu. The fixture copies the markup shape of goldcar.es's
// review-and-book page (no <form>, each input seven levels under the shared wrapper).
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { parseHtml } = require('./miniDom.js');
const registration = require('../shared/registration-form.js');
const formDetection = require('../shared/form-detection.js');

function field(id, name, label, attrs = '') {
  return `<div class="os-form-input"><label for="${id}">${label}</label>`
    + '<div class="os-form-input__input"><div class="os-input"><div class="os-form-input__placeholder-area">'
    + '<div class="os-input__input-wrapper"><div class="os-input__text-line">'
    + `<input type="text" name="${name}" id="${id}" autocomplete="off" ${attrs}>`
    + '</div></div></div></div></div></div>';
}

const DRIVER_DETAILS = '<header class="funnel-header"><div class="step">RENTAL LOCATION</div></header>'
  + '<div class="review-and-book-page__forms-section"><div class="review-and-book-page__driver-info">'
  + '<section class="driver-details-review-section"><h2>Driver details</h2>'
  + '<div class="driver-details-review-section__form">'
  + field('firstname-review-input-id', 'firstname-review', 'First name*')
  + field('lastname-review-input-id', 'lastname-review', 'Last name*')
  + field('birthdate-review-input-id', 'birthdate-review', 'Date of birth*', 'placeholder="DD / MM / YYYY"')
  + '<div class="driver-details-review-section__contact-inputs">'
  + '<div class="driver-details-review-section__email-newsletters__container">'
  + field('email-review-input-id', 'email-review', 'Email*').replace('type="text"', 'type="email"')
  + '<div class="newsletters-section"><div class="os-checkbox"><input type="checkbox" id="commercial">'
  + '<label for="commercial">I consent to receive news and special offers.</label></div></div></div>'
  + '<div class="os-phone-number-input"><label for="phoneNumber">Phone number*</label>'
  + '<div class="os-phone-number-input__field"><div class="os-select">'
  + '<select id="country-code" aria-label="select"><option value="ES">+34</option><option value="PL" selected>+48</option></select>'
  + '</div></div><div class="os-phone-number-input__field"><div class="os-input"><div class="os-input__text-line">'
  + '<input type="tel" id="phoneNumber" placeholder="Phone number"></div></div></div></div>'
  + '</div></div></section>'
  + '<section class="flight-information"><h2>Flight information</h2>'
  + field('flight-infos-review-input-id', 'flight-infos-review', 'Flight number')
  + '</section></div></div>';

const person = {
  givenName: 'Olena', familyName: 'Example', fullName: '', middleName: '',
  email: 'olena@example.test', birthDate: '1990-02-05',
};
const address = { phone: '+48 123 456 789', countryCode: 'PL', countryName: 'Poland', addressLine1: '', city: '' };
const write = (node, value) => { node.value = value; };

test('the driver details of a booking page are one person form, whatever the nesting', () => {
  const doc = parseHtml(DRIVER_DETAILS);
  const forms = registration.find(doc);
  const driver = forms.find((form) => form.fields.some((f) => f.node.id === 'firstname-review-input-id'));
  assert.ok(driver, 'the driver details are found as a form');
  const kinds = Object.fromEntries(driver.fields.map((f) => [f.node.id, f.kind]));
  assert.equal(kinds['firstname-review-input-id'], 'givenName');
  assert.equal(kinds['lastname-review-input-id'], 'familyName');
  assert.equal(kinds['birthdate-review-input-id'], 'birthday');
  assert.equal(kinds['email-review-input-id'], 'email');
  assert.equal(kinds.phoneNumber, 'phone');
  assert.equal(kinds['flight-infos-review-input-id'], undefined, 'another section is another group');
  const people = registration.personSelections(doc);
  assert.equal(people.length, 1, 'the popup and the panel offer a person fill');
  assert.ok(people[0].controls.some((control) => control.node.id === 'email-review-input-id'));
});

test('a person fill writes the names, the birthday as the mask shows it, the email and the phone', () => {
  const doc = parseHtml(DRIVER_DETAILS);
  const email = doc.getElementById('email-review-input-id');
  const selection = registration.selectionFor(doc, email);
  assert.ok(selection, 'the email field belongs to the person form');
  const filled = registration.fillSelection(doc, selection, registration.personProfile(person, address), write);
  assert.ok(filled >= 5);
  assert.equal(doc.getElementById('firstname-review-input-id').value, 'Olena');
  assert.equal(doc.getElementById('lastname-review-input-id').value, 'Example');
  assert.equal(doc.getElementById('birthdate-review-input-id').value, '05 / 02 / 1990');
  assert.equal(email.value, 'olena@example.test');
  assert.equal(doc.getElementById('country-code').value, 'PL');
  assert.equal(doc.getElementById('phoneNumber').value, '123 456 789', 'the +48 the menu shows is not typed twice');
});

test('a phone whose dial code differs from the country menu keeps its full number', () => {
  const doc = parseHtml(DRIVER_DETAILS.replace('<option value="PL" selected>+48</option>', '<option value="PL">+48</option>'));
  const selection = registration.selectionFor(doc, doc.getElementById('phoneNumber'));
  registration.fillSelection(doc, selection, registration.personProfile(person, { ...address, countryCode: '', countryName: '' }), write);
  assert.equal(doc.getElementById('country-code').value, 'ES');
  assert.equal(doc.getElementById('phoneNumber').value, '+48 123 456 789');
});

test('fields named only by aria-labelledby or by the text right before them are classified', () => {
  const doc = parseHtml('<section>'
    + '<div><span id="fn-label">First name</span><div><div><input id="a" aria-labelledby="fn-label"></div></div></div>'
    + '<div><p>Last name</p><div><div><input id="b"></div></div></div>'
    + '<div><span>Date of birth</span><div><input id="c" placeholder="DD.MM.YYYY"></div></div>'
    + '<div><span>E-mail</span><div><input id="d"></div></div>'
    + '</section>');
  assert.equal(formDetection.identityFieldHint(doc.getElementById('a')), 'givenName');
  assert.equal(formDetection.identityFieldHint(doc.getElementById('b')), 'familyName');
  assert.equal(formDetection.identityFieldHint(doc.getElementById('c')), 'birthday');
  assert.equal(formDetection.identityFieldHint(doc.getElementById('d')), 'email');
  const [form] = registration.find(doc);
  assert.deepEqual(form.fields.map((f) => f.kind), ['givenName', 'familyName', 'birthday', 'email']);
  assert.equal(registration.readableLabel(doc.getElementById('b')), 'Last name');
});

test('text before a field is never borrowed from a neighbouring field', () => {
  const doc = parseHtml('<section><div><span>First name</span><input id="a" name="given"><input id="b"></div></section>');
  assert.equal(formDetection.identityFieldHint(doc.getElementById('b')), null);
});

test('spaced and plain date masks both give the shown format', () => {
  for (const [placeholder, expected] of [['DD / MM / YYYY', '05 / 02 / 1990'], ['MM/DD/YYYY', '02/05/1990'],
    ['TT.MM.JJJJ', '05.02.1990'], ['YYYY-MM-DD', '1990-02-05']]) {
    const doc = parseHtml(`<form><label for="n">First name</label><input id="n"><label for="e">Email</label><input id="e" type="email">
      <label for="b">Date of birth</label><input id="b" placeholder="${placeholder}"></form>`);
    const selection = registration.selectionFor(doc, doc.getElementById('b'));
    registration.fillSelection(doc, selection, registration.personProfile(person, null), write);
    assert.equal(doc.getElementById('b').value, expected, placeholder);
  }
});

test('the inline icon hides while its field is scrolled under a sticky header', () => {
  const { isFieldSpotCovered } = require('../content.js');
  const doc = parseHtml(DRIVER_DETAILS);
  const email = doc.getElementById('email-review-input-id');
  const header = doc.querySelector('header');
  const wrapper = email.parentElement;
  const ownHost = doc.createElement('mypass-icon');
  const rect = { left: 280, right: 930, top: 180, bottom: 240, width: 650, height: 60 };
  const isOwn = (node) => node === ownHost;
  // Under the header: the topmost page element at the icon's spot is the header.
  assert.equal(isFieldSpotCovered(email, rect, () => [ownHost, header, doc.body], isOwn, null), true);
  // In view: the field, its wrapper (padding) or its visual box is on top.
  assert.equal(isFieldSpotCovered(email, rect, () => [ownHost, email, wrapper], isOwn, null), false);
  assert.equal(isFieldSpotCovered(email, rect, () => [wrapper, doc.body], isOwn, null), false);
  const box = email.closest('.os-input');
  const sibling = doc.createElement('span');
  box.appendChild(sibling);
  assert.equal(isFieldSpotCovered(email, rect, () => [sibling], isOwn, box), false, 'a clear button inside the box');
  // No hit test available, or nothing reported: never hide on a guess.
  assert.equal(isFieldSpotCovered(email, rect, null, isOwn, null), false);
  assert.equal(isFieldSpotCovered(email, rect, () => [], isOwn, null), false);
});
