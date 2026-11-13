// A value typed into a login identifier (phone, email, or username) that is
// not already stored on a matching item. Comparison ignores spaces and case.
// This file only decides; it does not write to the vault or the page.
//
// Classic script. The only top-level name is the `mypassPageIdentifier` bag.

var mypassPageIdentifier = (function () {
  function hintText(input, scope) {
    const id = input.getAttribute('id') || '';
    let label = '';
    const wrapping = typeof input.closest === 'function' ? input.closest('label') : null;
    if (wrapping && wrapping.textContent) label = wrapping.textContent;
    else if (id && scope && typeof scope.querySelector === 'function' && !id.includes('"')) {
      const forLabel = scope.querySelector('label[for="' + id + '"]');
      if (forLabel && forLabel.textContent) label = forLabel.textContent;
    }
    return [
      input.getAttribute('name'), input.getAttribute('placeholder'), input.getAttribute('aria-label'),
      input.getAttribute('autocomplete'), label
    ].filter(Boolean).join(' ').toLowerCase();
  }

  function kindFor(input, scope, scoreUsername) {
    const type = (input.getAttribute('type') || 'text').toLowerCase();
    if (['password', 'hidden', 'search', 'checkbox', 'radio', 'submit', 'button', 'file', 'image',
      'range', 'color', 'date', 'datetime-local', 'month', 'week', 'time', 'number', 'url'].includes(type)) {
      return null;
    }
    const autocomplete = (input.getAttribute('autocomplete') || '').toLowerCase();
    const inputMode = (input.getAttribute('inputmode') || '').toLowerCase();
    const hints = hintText(input, scope);
    const phone = type === 'tel' || inputMode === 'tel' || /\btel\b/.test(autocomplete)
      || /phone|mobile|телефон|мобільн|номер\s*телеф/.test(hints);
    if (phone) return 'phone';
    if (type === 'email' || autocomplete.includes('email') || /e-?mail|пошта/.test(hints)) return 'email';
    if (typeof scoreUsername === 'function' && scoreUsername(input, scope) >= 3) return 'username';
    return null;
  }

  function normalizeIdentifier(value) {
    return String(value || '').replace(/\s+/g, '').toLowerCase();
  }

  function fieldKeyForKind(kind) {
    if (kind === 'phone') return 'Phone';
    if (kind === 'email') return 'Email';
    return 'Login';
  }

  return {
    normalizeIdentifier: normalizeIdentifier,
    fieldKeyForKind: fieldKeyForKind,

    entryHasIdentifier(entry, value) {
      const needle = normalizeIdentifier(value);
      if (!needle || !entry) return false;
      const values = [entry.username].concat(entry.identifiers || []);
      return values.some((item) => normalizeIdentifier(item) === needle);
    },

    // Null when there is nothing new to offer: no page value, no saved items,
    // or any saved item already holds the value in its login or an extra field.
    unsavedPageIdentifier(page, entries) {
      const value = page && String(page.value || '').trim();
      if (!value || !Array.isArray(entries) || entries.length === 0) return null;
      if (!normalizeIdentifier(value)) return null;
      if (entries.some((entry) => this.entryHasIdentifier(entry, value))) return null;
      const kind = page.kind === 'phone' || page.kind === 'email' ? page.kind : 'username';
      return { value: value, kind: kind, fieldKey: fieldKeyForKind(kind) };
    },

    // The filled identifier on this page, or null. `helpers` supplies the form
    // detectors from form-detection.js so this file does not redeclare them.
    readPageIdentifier(root, helpers) {
      if (!root || typeof root.querySelectorAll !== 'function') return null;
      const visible = helpers && helpers.isFieldVisible ? helpers.isFieldVisible : function () { return true; };
      const otp = helpers && helpers.findOtpField ? helpers.findOtpField(root) : null;
      const scoreUsername = helpers && helpers.scoreUsernameLikeness;
      const found = [];
      for (const input of Array.from(root.querySelectorAll('input'))) {
        if (!input || input === otp || !visible(input)) continue;
        const value = String(input.value || '').trim();
        if (!value) continue;
        const kind = kindFor(input, root, scoreUsername);
        if (!kind) continue;
        found.push({ input: input, value: value, kind: kind });
      }
      if (!found.length) return null;
      const focused = found.find((item) => item.input === root.activeElement);
      const best = focused || found.slice().sort((a, b) => {
        const rank = { phone: 3, email: 2, username: 1 };
        return (rank[b.kind] || 0) - (rank[a.kind] || 0);
      })[0];
      return { value: best.value, kind: best.kind };
    }
  };
})();

if (typeof module !== 'undefined') module.exports = mypassPageIdentifier;
