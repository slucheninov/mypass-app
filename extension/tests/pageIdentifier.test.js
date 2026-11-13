const assert = require('node:assert');
const pageIdentifier = require('../shared/page-identifier.js');

const phone = '+38 066 183 74 99';
const yasno = { id: '1', title: 'Yasno', username: 'ada@example.com', identifiers: [] };

assert.equal(pageIdentifier.unsavedPageIdentifier(null, [yasno]), null);
assert.equal(pageIdentifier.unsavedPageIdentifier({ value: '   ', kind: 'phone' }, [yasno]), null);
assert.equal(pageIdentifier.unsavedPageIdentifier({ value: phone, kind: 'phone' }, []), null);

const offer = pageIdentifier.unsavedPageIdentifier({ value: '  ' + phone + ' ', kind: 'phone' }, [yasno]);
assert.deepEqual(offer, { value: phone, kind: 'phone', fieldKey: 'Phone' });
assert.equal(
  pageIdentifier.unsavedPageIdentifier({ value: 'Ada@Example.com', kind: 'email' }, [yasno]),
  null,
  'a saved login matches ignoring spaces and case'
);
assert.equal(
  pageIdentifier.unsavedPageIdentifier({ value: '+380661837499', kind: 'phone' }, [
    { username: 'ada@example.com', identifiers: ['+38 066 183 74 99'] }
  ]),
  null,
  'a saved extra field matches ignoring spaces'
);
assert.equal(
  pageIdentifier.unsavedPageIdentifier({ value: phone, kind: 'phone' }, [
    yasno,
    { username: '+380661837499', identifiers: [] }
  ]),
  null,
  'any saved item for the site suppresses the offer'
);
assert.equal(
  pageIdentifier.fieldKeyForKind('email'),
  'Email'
);
assert.deepEqual(
  pageIdentifier.unsavedPageIdentifier({ value: 'bob', kind: 'nope' }, [{ username: '', identifiers: [] }]),
  { value: 'bob', kind: 'username', fieldKey: 'Login' }
);

function input(attrs, value) {
  return {
    value,
    getAttribute(name) { return Object.prototype.hasOwnProperty.call(attrs, name) ? attrs[name] : null; },
    closest() { return null; }
  };
}

function page(inputs, active) {
  return {
    activeElement: active || null,
    querySelectorAll(selector) { return selector === 'input' ? inputs : []; },
    querySelector() { return null; }
  };
}

const helpers = { isFieldVisible: () => true, findOtpField: () => null, scoreUsernameLikeness: () => 0 };
const tel = input({ type: 'tel', placeholder: 'Номер телефону' }, phone);
assert.deepEqual(
  pageIdentifier.readPageIdentifier(page([tel]), helpers),
  { value: phone, kind: 'phone' }
);
assert.equal(
  pageIdentifier.readPageIdentifier(page([input({ type: 'tel', placeholder: 'Номер телефону' }, '  ')]), helpers),
  null,
  'an empty phone field is not an unsaved value'
);
assert.equal(
  pageIdentifier.readPageIdentifier(page([input({ type: 'search', placeholder: 'Search' }, 'yasno')]), helpers),
  null
);

const email = input({ type: 'email' }, 'bob@example.com');
assert.deepEqual(
  pageIdentifier.readPageIdentifier(page([tel, email], email), helpers),
  { value: 'bob@example.com', kind: 'email' },
  'the focused identifier wins when several are filled'
);

const otp = input({ type: 'tel' }, '123456');
assert.equal(
  pageIdentifier.readPageIdentifier(page([otp]), { ...helpers, findOtpField: () => otp }),
  null,
  'a one-time code is not a login identifier'
);

const username = input({ type: 'text', name: 'username' }, 'carol');
assert.deepEqual(
  pageIdentifier.readPageIdentifier(page([username]), {
    ...helpers,
    scoreUsernameLikeness: () => 5
  }),
  { value: 'carol', kind: 'username' }
);

console.log('pageIdentifier.test.js: unsaved phone, email, and username OK');
