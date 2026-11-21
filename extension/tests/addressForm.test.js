const assert = require('node:assert/strict');
const { test } = require('node:test');
const addressForm = require('../shared/address-form.js');

function fixture() {
  const doc = { activeElement: null, nodes: [], defaultView: { getComputedStyle: () => ({ visibility: 'visible', display: 'block', opacity: '1' }) },
    querySelectorAll() { return this.nodes; } };
  const container = { tagName: 'DIV' };
  function field(autocomplete, options = {}) {
    const node = { autocomplete, tagName: 'INPUT', type: 'text', name: '', id: '', value: '', size: 0,
      isConnected: true, disabled: false, readOnly: false, ownerDocument: doc, form: null, parentElement: container,
      getAttribute(name) { return this[name] || null; }, closest() { return null; }, matches() { return false; },
      getBoundingClientRect() { return { width: 100, height: 30 }; }, ...options };
    doc.nodes.push(node);
    return node;
  }
  return { doc, field };
}
const address = { fullName: 'Example Person', givenName: '', familyName: '', organization: 'Example Ltd',
  addressLine1: '12 Example Street', addressLine2: 'Unit 4', city: 'Sample City',
  region: 'Some Region', countryCode: 'PL', countryName: 'Poland', phone: '+48 123456789',
  email: 'demo@example.test', postalCode: '00-001' };
function fill(doc, groups, id = '0', profile = address, write = (node, value) => { node.value = value; }) {
  return addressForm.apply(doc, groups, id, profile, write);
}

for (const defaultSelected of [true, false]) {
  test(`fills the ${defaultSelected ? 'explicit' : 'implicit'} country default before matching the region`, () => {
    const { doc, field } = fixture();
    field('address-line1');
    const region = field('address-level1', { tagName: 'SELECT', value: 'CA', options: [
      { value: 'CA', textContent: 'California', selected: true, defaultSelected: false }
    ] });
    const country = field('country', { tagName: 'SELECT', value: 'US', options: [
      { value: 'US', textContent: 'United States', selected: true, defaultSelected },
      { value: 'PL', textContent: 'Poland', selected: false, defaultSelected: false }
    ] });
    const result = fill(doc, addressForm.collect(doc), '0', address, (node, value) => {
      node.value = value;
      if (node === country) {
        region.value = 'XX';
        region.options = [
          { value: 'XX', textContent: 'Other', selected: true, defaultSelected: false },
          { value: 'SR', textContent: 'Some Region', selected: false, defaultSelected: false }
        ];
      }
    });
    assert.equal(country.value, 'PL');
    assert.equal(region.value, 'SR');
    assert.equal(result.filled, 3);
  });
}

for (const defaultSelected of [true, false]) {
  test(`preserves a user selection that differs from the ${defaultSelected ? 'explicit' : 'implicit'} default`, () => {
    const { doc, field } = fixture();
    field('address-line1');
    const country = field('country', { tagName: 'SELECT', value: 'DE', options: [
      { value: 'US', textContent: 'United States', selected: false, defaultSelected },
      { value: 'DE', textContent: 'Germany', selected: true, defaultSelected: false },
      { value: 'PL', textContent: 'Poland', selected: false, defaultSelected: false }
    ] });
    assert.equal(fill(doc, addressForm.collect(doc)).filled, 1);
    assert.equal(country.value, 'DE');
  });
}

test('fills a selected disabled placeholder with a nonempty value', () => {
  const { doc, field } = fixture();
  field('address-line1');
  const country = field('country', { tagName: 'SELECT', value: 'choose', options: [
    { value: 'choose', textContent: 'Choose a country', selected: true, defaultSelected: false, disabled: true },
    { value: 'PL', textContent: 'Poland', selected: false, defaultSelected: true }
  ] });
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 2);
  assert.equal(country.value, 'PL');
});

test('excludes a guessed newsletter email outside the address section', () => {
  const { doc, field } = fixture();
  const page = { tagName: 'DIV' };
  const checkout = { tagName: 'SECTION', parentElement: page };
  const newsletter = { tagName: 'SECTION', parentElement: page };
  field('address-line1', { parentElement: checkout });
  const email = field('', { name: 'email', type: 'email', parentElement: newsletter });
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 1);
  assert.equal(email.value, '');
});

test('keeps the closest of two guessed fields with the same token', () => {
  const { doc, field } = fixture();
  const checkout = { tagName: 'SECTION' };
  const contact = { tagName: 'DIV', parentElement: checkout };
  const farther = field('', { name: 'phone', parentElement: contact });
  field('address-line1', { parentElement: checkout });
  const closer = field('', { name: 'phone', parentElement: checkout });
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 2);
  assert.equal(closer.value, address.phone);
  assert.equal(farther.value, '');
});

test('a distant guessed city cannot pull a newsletter field into the address', () => {
  const { doc, field } = fixture();
  const checkout = { tagName: 'SECTION' };
  const newsletter = { tagName: 'SECTION' };
  field('address-line1', { parentElement: checkout });
  const city = field('', { name: 'city', parentElement: newsletter });
  const email = field('', { name: 'email', type: 'email', parentElement: newsletter });
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 1);
  assert.equal(city.value, '');
  assert.equal(email.value, '');
});
{
  const { doc, field } = fixture();
  field('email'); field('username'); field('one-time-code');
  assert.equal(addressForm.collect(doc).length, 0, 'login/email alone is not an address form');
}
{
  const { doc, field } = fixture();
  const shipping = field('section-checkout shipping address-line1');
  const billing = field('section-checkout billing address-line1');
  const other = field('section-other shipping address-line1');
  doc.activeElement = billing;
  const groups = addressForm.collect(doc);
  assert.equal(groups.length, 3);
  assert.equal(addressForm.describe(groups, doc)[1].focused, true);
  assert.equal(fill(doc, groups, '1').filled, 1);
  assert.equal(billing.value, address.addressLine1);
  assert.equal(shipping.value, ''); assert.equal(other.value, '');
}
{
  const { doc, field } = fixture();
  field('address-line1', { form: { id: 'one' } });
  field('address-line1', { form: { id: 'two' } });
  assert.equal(addressForm.collect(doc).length, 2, 'form owners partition identical tokens');
}
{
  const { doc, field } = fixture();
  field('address-line1'); field('address-line1');
  assert.equal(addressForm.collect(doc).length, 0, 'duplicate anonymous scopes fail closed');
}
{
  const { doc, field } = fixture();
  field('address-line1'); field('street-address');
  assert.equal(addressForm.collect(doc).length, 0, 'overlapping address layouts fail closed');
}
{
  const { doc, field } = fixture();
  const street = field('street-address', { tagName: 'TEXTAREA' });
  const name = field('given-name', { value: 'Keep name' });
  const postal = field('postal-code', { value: 'Keep postal' });
  const email = field('email');
  assert.equal(fill(doc, addressForm.collect(doc), '0', { ...address, postalCode: '' }).filled, 2);
  assert.equal(street.value, '12 Example Street\nUnit 4');
  assert.equal(email.value, address.email);
  assert.equal(name.value, 'Keep name', 'an existing name is preserved even with a full name on file');
  assert.equal(postal.value, 'Keep postal', 'blank profile data does not erase fields');
}
{
  const { doc, field } = fixture();
  field('address-line1');
  const country = field('country', { tagName: 'SELECT', options: [
    { value: 'DE', textContent: 'Germany' }, { value: 'PL', textContent: 'Polska' }
  ] });
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 2);
  assert.equal(country.value, 'PL', 'country code can match even when labels are localized');
  country.options = [{ value: 'custom', textContent: 'Poland' }];
  country.value = '';
  fill(doc, addressForm.collect(doc)); assert.equal(country.value, 'custom');
  country.options = [{ value: 'US', textContent: 'United States' }];
  country.value = 'unchanged';
  fill(doc, addressForm.collect(doc)); assert.equal(country.value, 'unchanged', 'no guessed country');
}
{
  const { doc, field } = fixture();
  const street = field('shipping address-line1', { name: 'zip' });
  const secondLine = field('', { name: 'checkout[shipping_address][address2]' });
  const first = field('shipping given-name');
  const last = field('', { id: 'shippingLastName' });
  const postal = field('shipping postal-code', { value: 'Already typed' });
  const phone = field('shipping mobile tel', { type: 'tel' });
  const email = field('shipping email', { type: 'email' });
  const region = field('', { id: 'shipping-state', tagName: 'SELECT', options: [
    { value: 'SR', textContent: 'SOME REGION' }, { value: 'XX', textContent: 'Other' }
  ] });
  const billing = field('', { name: 'billing-address1' });
  field('', { name: 'billing-city' });
  const groups = addressForm.collect(doc);
  assert.equal(groups.length, 2);
  assert.equal(fill(doc, groups, '0', { ...address, fullName: 'Ada Byron Lovelace' }).filled, 7);
  assert.deepEqual([street, secondLine, first, last, postal, phone, email, region, billing].map(node => node.value),
    [address.addressLine1, address.addressLine2, 'Ada', 'Byron Lovelace', 'Already typed',
      address.phone, address.email, 'SR', '']);
}
{
  const { doc, field } = fixture();
  const street = field('', { name: 'address1' });
  const given = field('given-name');
  const family = field('family-name');
  const otp = field('one-time-code', { name: 'phone', type: 'tel' });
  fill(doc, addressForm.collect(doc), '0', { ...address, fullName: 'Cher', givenName: 'Explicit' });
  assert.equal(street.value, address.addressLine1);
  assert.equal(given.value, 'Explicit');
  assert.equal(family.value, '', 'a mononym supplies no family name');
  assert.equal(otp.value, '', 'an explicit non-address autocomplete wins over a conflicting name');
}
{
  const { doc, field } = fixture();
  const street = field('address-line1');
  const country = field('country', { tagName: 'SELECT', value: 'DE', options: [
    { value: 'pl', textContent: 'Polska' }, { value: 'DE', textContent: 'Germany' }
  ] });
  const city = field('address-level2');
  fill(doc, addressForm.collect(doc), '0', address, (node, value) => {
    node.value = value;
    if (node === street) city.value = 'Set during input';
  });
  assert.equal(city.value, 'Set during input', 'check whether each field is empty at the time of the write');
  assert.equal(country.value, 'DE', 'a selected country is preserved');
  country.value = '';
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 1);
  assert.equal(country.value, 'pl', 'option codes are matched case-insensitively');
  assert.equal(fill(doc, addressForm.collect(doc)).code, 'address_no_values');
}
{
  const { doc, field } = fixture();
  field('address-line1');
  const region = field('address-level1', { tagName: 'SELECT', options: [] });
  const country = field('', { tagName: 'SELECT', textContent: 'Select a countryPoland',
    options: [{ value: 'PL', textContent: 'Poland' }] });
  country.labels = [{ textContent: 'CountrySelect a countryPoland', contains: (node) => node === country,
    childNodes: [{ textContent: 'Country' }, country] }];
  assert.equal(fill(doc, addressForm.collect(doc), '0', address, (node, value) => {
    node.value = value;
    if (node === country) region.options = [{ value: 'some region', textContent: 'Local region name' }];
  }).filled, 3, 'country is filled before looking up dependent region options');
  assert.equal(country.value, 'PL');
  assert.equal(region.value, 'some region');
}
{
  const { doc, field } = fixture();
  const values = new WeakMap();
  class Input {
    set value(value) { values.set(this, value); }
  }
  doc.defaultView.HTMLInputElement = Input;
  doc.defaultView.Event = Event;
  const events = [];
  const street = field('address-line1', { dispatchEvent(event) {
    events.push([event.type, event.bubbles, values.get(this)]);
  } });
  assert.equal(fill(doc, addressForm.collect(doc), '0', address, addressForm.setNativeAddressValue).filled, 1);
  assert.deepEqual(events, [['input', true, address.addressLine1], ['change', true, address.addressLine1]]);
}
{
  const { doc, field } = fixture();
  const eligible = field('address-line1');
  const traps = [
    field('email', { type: 'password' }), field('email', { type: 'hidden' }),
    field('name', { disabled: true }), field('family-name', { readOnly: true }),
    field('postal-code', { isConnected: false }), field('address-level2', { closest: () => ({}) }),
    field('off'), field('billing one-time-code'), field('name', { getBoundingClientRect: () => ({ width: 0, height: 0 }) })
  ];
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 1);
  assert.equal(eligible.value, address.addressLine1);
  assert.ok(traps.every((node) => node.value === ''));
}
{
  const { doc, field } = fixture();
  const street = field('address-line1');
  const groups = addressForm.collect(doc);
  doc.nodes = []; field('address-line1');
  assert.equal(fill(doc, groups).code, 'address_form_changed', 'replacement node cannot receive delayed fill');
  assert.equal(street.value, '');
}
{
  const { doc, field } = fixture();
  const street = field('address-line1');
  const groups = addressForm.collect(doc);
  street.autocomplete = 'billing address-line1';
  assert.equal(fill(doc, groups).code, 'address_form_changed');
}
{
  const { doc, field } = fixture();
  const street = field('address-line1'); const city = field('address-level2');
  const groups = addressForm.collect(doc);
  const result = fill(doc, groups, '0', address, (node, value) => {
    node.value = value;
    city.autocomplete = 'billing address-level2';
  });
  assert.equal(result.code, 'address_form_changed');
  assert.equal(street.value, address.addressLine1); assert.equal(city.value, '');
}
{
  const { doc, field } = fixture();
  const street = field('address-line1');
  assert.equal(addressForm.apply(doc, addressForm.collect(doc), '0', address,
    () => { throw Error('must not write'); }, () => false).code, 'address_form_changed');
  assert.equal(street.value, '');
}
{
  const { doc, field } = fixture();
  const invisible = { opacity: '0', parentElement: null };
  const hidden = field('email', { parentElement: invisible });
  field('address-line1');
  doc.defaultView.getComputedStyle = (node) => ({ visibility: 'visible', display: 'block', opacity: node.opacity || '1' });
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 1);
  assert.equal(hidden.value, '', 'ancestor opacity zero is not visible');
}
for (const mutate of [node => { node.isConnected = false; },
  node => { node.parentElement = { opacity: '0' }; }]) {
  const { doc, field } = fixture();
  const street = field('address-line1'); const email = field('email');
  doc.defaultView.getComputedStyle = (node) => ({ visibility: 'visible', display: 'block', opacity: node.opacity || '1' });
  const result = fill(doc, addressForm.collect(doc), '0', address, (node, value) => {
    node.value = value; mutate(email);
  });
  assert.equal(result.code, 'address_form_changed');
  assert.equal(result.filled, 1);
  assert.equal(email.value, '', 'synchronous page mutations stop all later writes');
}
{
  const { doc, field } = fixture();
  field('address-line1');
  const country = field('country', { tagName: 'SELECT', options: [
    { value: 'PL', textContent: 'Poland' }, { value: 'DE', textContent: 'Germany' }
  ] });
  const result = fill(doc, addressForm.collect(doc), '0', address, (node, value) => {
    node.value = value;
    country.options = [{ value: 'DE', textContent: 'Germany' }];
  });
  assert.equal(result.filled, 1);
  assert.equal(country.value, '', 'removed option is skipped even when the control is empty');
}
{
  const { doc, field } = fixture();
  const street = field('street-address');
  fill(doc, addressForm.collect(doc));
  assert.equal(street.value, '12 Example Street, Unit 4', 'single-line inputs do not glue multiline addresses');
}
{
  const labels = ['Vollständiger Name', 'Firma (optional)', 'Straße und Hausnummer', 'Stadt', 'Bundesland',
    'Postleitzahl', 'Land / Region', 'Telefonnummer', 'E-Mail-Adresse'];
  const { doc, field } = fixture();
  const nodes = labels.map(textContent => field('', { labels: [{ textContent }] }));
  const groups = addressForm.collect(doc);
  assert.equal(groups.length, 1);
  assert.equal(fill(doc, groups).filled, labels.length);
  assert.deepEqual(nodes.map(node => node.value), [address.fullName, address.organization, address.addressLine1,
    address.city, address.region, address.postalCode, address.countryName, address.phone, address.email]);
}
{
  const { doc, field } = fixture();
  field('address-line1');
  const postal = field('', { name: 'zip', type: 'number' });
  const card = field('cc-number', { name: 'phone', type: 'number' });
  assert.equal(fill(doc, addressForm.collect(doc), '0', { ...address, postalCode: '01234' }).filled, 2);
  assert.equal(postal.value, '01234');
  assert.equal(card.value, '');
  postal.value = '';
  assert.equal(fill(doc, addressForm.collect(doc)).code, 'address_no_values');
  assert.equal(postal.value, '', 'an alphanumeric postal code is skipped for a number input');
}
{
  const { doc, field } = fixture();
  field('', { name: 'city' });
  field('', { name: 'email', type: 'email' });
  assert.equal(addressForm.collect(doc).length, 0, 'location and contact alone are insufficient');
  const first = field('', { name: 'address1' });
  const second = field('', { name: 'street' });
  assert.equal(fill(doc, addressForm.collect(doc)).filled, 3);
  assert.equal(first.value, address.addressLine1, 'equally close guessed fields keep the first in DOM order');
  assert.equal(second.value, '');
}
console.log('addressForm.test.js: conservative scope, sparse values, select matching and stale forms OK');
