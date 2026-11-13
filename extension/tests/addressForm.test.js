const assert = require('node:assert/strict');
const addressForm = require('../shared/address-form.js');

function fixture() {
  const doc = { activeElement: null, nodes: [], defaultView: { getComputedStyle: () => ({ visibility: 'visible', display: 'block', opacity: '1' }) },
    querySelectorAll() { return this.nodes; } };
  function field(autocomplete, options = {}) {
    const node = { autocomplete, tagName: 'INPUT', type: 'text', name: '', id: '', value: '',
      isConnected: true, disabled: false, readOnly: false, ownerDocument: doc, form: null,
      getAttribute(name) { return this[name] || null; }, closest() { return null; }, matches() { return false; },
      getBoundingClientRect() { return { width: 100, height: 30 }; }, ...options };
    doc.nodes.push(node);
    return node;
  }
  return { doc, field };
}
const address = { fullName: 'Example Person', givenName: '', familyName: '',
  addressLine1: '12 Example Street', addressLine2: 'Unit 4', city: 'Sample City',
  countryCode: 'PL', countryName: 'Poland', email: 'demo@example.test', postalCode: '00-001' };
function fill(doc, groups, id = '0', profile = address, write = (node, value) => { node.value = value; }) {
  return addressForm.apply(doc, groups, id, profile, write);
}
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
  assert.equal(name.value, 'Keep name', 'full name is never split into a given name');
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
  fill(doc, addressForm.collect(doc)); assert.equal(country.value, 'custom');
  country.options = [{ value: 'US', textContent: 'United States' }];
  country.value = 'unchanged';
  fill(doc, addressForm.collect(doc)); assert.equal(country.value, 'unchanged', 'no guessed country');
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
  const country = field('country', { tagName: 'SELECT', value: 'DE', options: [
    { value: 'PL', textContent: 'Poland' }, { value: 'DE', textContent: 'Germany' }
  ] });
  const result = fill(doc, addressForm.collect(doc), '0', address, (node, value) => {
    node.value = value;
    country.options = [{ value: 'DE', textContent: 'Germany' }];
  });
  assert.equal(result.filled, 1);
  assert.equal(country.value, 'DE', 'removed option never clears an existing selection');
}
{
  const { doc, field } = fixture();
  const street = field('street-address');
  fill(doc, addressForm.collect(doc));
  assert.equal(street.value, '12 Example Street, Unit 4', 'single-line inputs do not glue multiline addresses');
}
console.log('addressForm.test.js: conservative scope, sparse values, select matching and stale forms OK');
