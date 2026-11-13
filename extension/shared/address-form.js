// Conservative address autofill: only standard autocomplete tokens opt a field in.
// Snapshots retain node identities, never field values. A changed form must be
// chosen again so a delayed native reply cannot fill a replacement or new scope.
var mypassAddressForm = (() => {
  const fields = {
    name: 'fullName', 'given-name': 'givenName', 'family-name': 'familyName',
    organization: 'organization', 'address-line1': 'addressLine1',
    'address-line2': 'addressLine2', 'address-level2': 'city',
    'address-level1': 'region', 'postal-code': 'postalCode',
    country: 'countryCode', 'country-name': 'countryName', tel: 'phone', email: 'email'
  };
  const addressTokens = new Set(['street-address', 'address-line1', 'address-line2',
    'address-level2', 'address-level1', 'postal-code']);

  function parse(control) {
    const tokens = (control.getAttribute('autocomplete') || '').trim().toLowerCase().split(/\s+/);
    let section = '', purpose = '';
    if (/^section-\S+$/.test(tokens[0])) section = tokens.shift();
    if (tokens[0] === 'shipping' || tokens[0] === 'billing') purpose = tokens.shift();
    if (tokens.length !== 1 || !(tokens[0] in fields || tokens[0] === 'street-address')) return null;
    return { token: tokens[0], section, purpose };
  }

  function editable(control) {
    if (!control.isConnected || control.disabled || control.readOnly || control.multiple
      || control.closest('[hidden], [inert]') || control.matches(':disabled')) return false;
    const tag = control.tagName.toLowerCase();
    if (!['input', 'textarea', 'select'].includes(tag)) return false;
    if (tag === 'input' && !['text', 'email', 'tel'].includes(control.type)) return false;
    const view = control.ownerDocument.defaultView;
    for (let ancestor = control; ancestor; ancestor = ancestor.parentElement) {
      const style = view.getComputedStyle(ancestor);
      if (style.visibility !== 'visible' || style.display === 'none' || Number(style.opacity) === 0) return false;
    }
    const box = control.getBoundingClientRect();
    return box.width > 0 && box.height > 0;
  }

  function collect(doc) {
    const owners = new Map();
    const groups = [];
    for (const node of doc.querySelectorAll('input[autocomplete], textarea[autocomplete], select[autocomplete]')) {
      const parsed = parse(node);
      if (!parsed || !editable(node)) continue;
      const owner = node.form || doc;
      if (!owners.has(owner)) owners.set(owner, new Map());
      const key = parsed.section + '|' + parsed.purpose;
      const scopes = owners.get(owner);
      if (!scopes.has(key)) {
        const group = { owner, key, purpose: parsed.purpose, section: parsed.section, controls: [] };
        scopes.set(key, group);
        groups.push(group);
      }
      scopes.get(key).controls.push({ node, token: parsed.token,
        signature: [node.tagName, node.type, node.name, node.id, node.getAttribute('autocomplete')].join('|') });
    }
    return groups.filter((group) => {
      const tokens = group.controls.map((item) => item.token);
      // Repeated unnamed sections cannot be safely disambiguated. Neither can a
      // street-address control and separate lines in the same scope.
      return tokens.some((token) => addressTokens.has(token))
        && new Set(tokens).size === tokens.length
        && !(tokens.includes('street-address') && (tokens.includes('address-line1') || tokens.includes('address-line2')));
    });
  }

  function describe(groups, doc) {
    return groups.map((group, index) => ({
      id: String(index),
      label: (group.purpose ? group.purpose[0].toUpperCase() + group.purpose.slice(1) : 'Address')
        + ' form ' + (index + 1),
      fields: group.controls.length,
      focused: group.controls.some((control) => control.node === doc.activeElement)
    }));
  }

  function sameGroup(a, b) {
    return !!(a && b && a.owner === b.owner && a.key === b.key
      && a.controls.length === b.controls.length
      && a.controls.every((item, index) => item.node === b.controls[index].node
        && item.token === b.controls[index].token && item.signature === b.controls[index].signature));
  }

  function valueFor(control, address) {
    const value = control.token === 'street-address'
      ? [address.addressLine1, address.addressLine2].filter((part) => typeof part === 'string' && part.trim()).join(control.node.tagName.toLowerCase() === 'textarea' ? '\n' : ', ')
      : address[fields[control.token]];
    if (typeof value !== 'string' || !value.trim()) return null;
    if (control.node.tagName.toLowerCase() !== 'select') return value;
    const candidates = [value];
    if (control.token === 'country' && address.countryName) candidates.push(address.countryName);
    if (control.token === 'country-name' && address.countryCode) candidates.push(address.countryCode);
    const normalize = (text) => String(text).trim().toLowerCase();
    const matches = Array.from(control.node.options).filter((option) => !option.disabled
      && !(option.parentElement && option.parentElement.disabled)
      && candidates.some((candidate) => normalize(candidate) === normalize(option.value)
        || normalize(candidate) === normalize(option.textContent)));
    return matches.length === 1 ? matches[0].value : null;
  }

  function setNativeAddressValue(node, value) {
    const view = node.ownerDocument.defaultView;
    const prototypes = { INPUT: view.HTMLInputElement, TEXTAREA: view.HTMLTextAreaElement, SELECT: view.HTMLSelectElement };
    const constructor = prototypes[node.tagName];
    const setter = constructor && Object.getOwnPropertyDescriptor(constructor.prototype, 'value').set;
    // Use the platform setter rather than a framework's per-instance tracker,
    // then dispatch input/change so controlled forms observe the explicit fill.
    if (setter) setter.call(node, value);
    else node.value = value;
    node.dispatchEvent(new view.Event('input', { bubbles: true }));
    node.dispatchEvent(new view.Event('change', { bubbles: true }));
  }

  function apply(doc, snapshot, groupId, address, setValue, stillCurrent) {
    const chosen = snapshot[Number(groupId)];
    if (!chosen || String(Number(groupId)) !== groupId) return { type: 'error', code: 'address_form_changed' };
    const valid = () => (!stillCurrent || stillCurrent()) && collect(doc).some((group) => sameGroup(chosen, group));
    if (!valid()) return { type: 'error', code: 'address_form_changed' };
    let filled = 0;
    for (const control of chosen.controls) {
      // input/change handlers may synchronously replace other controls. Recheck
      // before each write instead of following stale references into a new form.
      if (!valid()) return { type: 'error', code: 'address_form_changed', filled };
      // Earlier change handlers can repopulate a dependent select without
      // replacing its node. Match the current options now, never a cached value
      // that could clear the select after its option was removed.
      const value = valueFor(control, address);
      if (value === null) continue;
      setValue(control.node, value);
      filled += 1;
    }
    return filled ? { type: 'addressFilled', filled } : { type: 'error', code: 'address_no_values' };
  }

  return { parse, editable, collect, describe, sameGroup, valueFor, apply, setNativeAddressValue };
})();
if (typeof module !== 'undefined') module.exports = mypassAddressForm;
