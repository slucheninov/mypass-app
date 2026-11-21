// Standard autocomplete tokens take precedence over address-specific field hints.
// Snapshots retain node identities, never field values. A changed form must be
// chosen again so a delayed native reply cannot fill a replacement or new scope.
var mypassAddressForm = (() => {
  const formDetection = typeof require === 'function' ? require('./form-detection.js') : mypassFormDetection;
  const fields = {
    name: 'fullName', 'given-name': 'givenName', 'family-name': 'familyName',
    organization: 'organization', 'address-line1': 'addressLine1',
    'address-line2': 'addressLine2', 'address-level2': 'city',
    'address-level1': 'region', 'postal-code': 'postalCode',
    country: 'countryCode', 'country-name': 'countryName', tel: 'phone', email: 'email'
  };
  const streetTokens = new Set(['street-address', 'address-line1', 'address-line2']);
  const placeTokens = new Set(['address-level2', 'address-level1', 'postal-code']);
  const countryTokens = new Set(['country', 'country-name']);
  const addressBoundaries = new Set(['SECTION', 'FIELDSET', 'MAIN', 'ASIDE', 'HEADER', 'FOOTER', 'NAV']);
  const isAnchor = ({ token }) => streetTokens.has(token) || placeTokens.has(token);

  function parse(autocomplete, control) {
    if (['', 'on', 'off'].includes(autocomplete)) {
      const hint = formDetection.addressFieldHint(control);
      return hint && { ...hint, section: '', explicit: false };
    }
    const tokens = autocomplete.split(/\s+/);
    let section = '', purpose = '';
    if (/^section-\S+$/.test(tokens[0])) section = tokens.shift();
    if (tokens[0] === 'shipping' || tokens[0] === 'billing') purpose = tokens.shift();
    if (['home', 'work', 'mobile', 'fax', 'pager'].includes(tokens[0])
      && ['tel', 'email'].includes(tokens[1])) tokens.shift();
    if (tokens.length !== 1 || !(Object.hasOwn(fields, tokens[0]) || tokens[0] === 'street-address')) return null;
    return { token: tokens[0], section, purpose, explicit: true };
  }

  function editable(control, token) {
    if (!control.isConnected || control.disabled || control.readOnly || control.multiple
      || control.closest('[hidden], [inert]') || control.matches(':disabled')) return false;
    const tag = control.tagName.toLowerCase();
    if (!['input', 'textarea', 'select'].includes(tag)) return false;
    if (tag === 'input' && !['text', 'email', 'tel'].includes(control.type)
      && !(control.type === 'number' && token === 'postal-code')) return false;
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
    for (const node of doc.querySelectorAll('input, textarea, select')) {
      const autocomplete = (node.getAttribute('autocomplete') || '').trim().toLowerCase();
      const parsed = parse(autocomplete, node);
      if (!parsed || !editable(node, parsed.token)) continue;
      const owner = node.form || doc;
      if (!owners.has(owner)) owners.set(owner, []);
      owners.get(owner).push({ ...parsed, node,
        signature: JSON.stringify([node.tagName, node.type, node.name, node.id, autocomplete]) });
    }
    return Array.from(owners).flatMap(([owner, controls]) => addressGroups(owner, controls));
  }

  function ancestors(node) {
    const path = [];
    for (let parent = node.parentElement; parent; parent = parent.parentElement) {
      if (parent.tagName === 'BODY' || parent.tagName === 'HTML') break;
      path.push(parent);
      // Outside forms, a shared page wrapper is weak evidence. Stay within
      // the local section, allowing up to three levels for field wrappers.
      if (!node.form && (path.length === 3 || addressBoundaries.has(parent.tagName))) break;
    }
    return path;
  }

  function closestHints(controls, anchors) {
    const anchorPaths = anchors.map((item) => ({ token: item.token, path: ancestors(item.node) }));
    const closest = new Map();
    for (const item of controls) {
      if (item.explicit) continue;
      const path = ancestors(item.node);
      let distance = Infinity;
      let hasOtherAnchor = false;
      for (const anchor of anchorPaths) {
        if (anchor.token === item.token) continue;
        hasOtherAnchor = true;
        const shared = path.findIndex((parent) => anchor.path.includes(parent));
        if (shared !== -1) {
          distance = Math.min(distance, shared + anchor.path.indexOf(path[shared]) + 2);
        }
      }
      // A lone inferred anchor can still establish proximity for nearby hints.
      if (!hasOtherAnchor && isAnchor(item)) distance = 0;
      if (!item.node.form && distance === Infinity) continue;
      const previous = closest.get(item.token);
      if (!previous || distance < previous.distance) closest.set(item.token, { item, distance });
    }
    return controls.filter((item) => item.explicit || closest.get(item.token)?.item === item);
  }

  function nearbyHints(controls) {
    const anchors = controls.filter(isAnchor);
    const keptAnchors = closestHints(anchors, anchors);
    const keptOthers = closestHints(controls.filter((item) => !isAnchor(item)), keptAnchors);
    const kept = new Set([...keptAnchors, ...keptOthers]);
    return controls.filter((item) => kept.has(item));
  }

  function addressGroups(owner, controls) {
    const scopes = new Map();
    const explicitScopes = controls.filter((item) => item.explicit);
    const purposes = new Set(controls.map((item) => item.purpose).filter(Boolean));
    for (const item of controls) {
      let { section, purpose } = item;
      // A hint can join a single compatible explicit scope. When shipping and
      // billing compete, a field with no purpose stays in its own default scope.
      if (!item.explicit && (purpose || purposes.size <= 1)) {
        const compatible = explicitScopes.filter((other) => !purpose || purpose === other.purpose);
        const keys = new Set(compatible.map((other) => other.section + '|' + other.purpose));
        if (keys.size === 1) ({ section, purpose } = compatible[0]);
      }
      const key = section + '|' + purpose;
      if (!scopes.has(key)) scopes.set(key, { owner, key, purpose, controls: [] });
      scopes.get(key).controls.push(item);
    }
    return Array.from(scopes.values()).filter((group) => {
      const explicitTokens = new Set(group.controls.filter((item) => item.explicit).map((item) => item.token));
      const explicitStreet = explicitTokens.has('street-address');
      const explicitLines = explicitTokens.has('address-line1') || explicitTokens.has('address-line2');
      group.controls = nearbyHints(group.controls.filter(({ explicit, token }) => explicit || !(explicitTokens.has(token)
        || (explicitStreet && streetTokens.has(token)) || (explicitLines && token === 'street-address'))));
      const tokens = group.controls.map((item) => item.token);
      const hasStreet = tokens.some((token) => streetTokens.has(token));
      const places = tokens.filter((token) => placeTokens.has(token)).length;
      const explicitAnchor = group.controls.some((item) => item.explicit && isAnchor(item));
      // Repeated explicit fields can be two unnamed sections. Ambiguous layouts stay
      // unavailable. Inferred fields need more evidence than a lone city or ZIP.
      return (explicitAnchor || (hasStreet && tokens.length > 1) || places >= 2)
        && new Set(tokens).size === tokens.length
        && !(tokens.includes('street-address') && (tokens.includes('address-line1') || tokens.includes('address-line2')));
    });
  }

  function describe(groups, doc) {
    return groups.map((group, index) => ({
      id: String(index),
      // `purpose` and `number` are what the popup localises into a name; `label` is only an opaque
      // identifier the service worker validates and relays.
      purpose: group.purpose === 'shipping' || group.purpose === 'billing' ? group.purpose : '',
      number: index + 1,
      label: (group.purpose || 'address') + '-' + (index + 1),
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

  function optionDisabled(option) {
    return option.disabled || option.parentElement?.disabled;
  }

  function fillable(node) {
    if (node.value === '') return true;
    if (node.tagName !== 'SELECT') return false;
    const options = Array.from(node.options);
    const selected = options.find((option) => option.selected);
    if (!selected) return false;
    if (optionDisabled(selected)) return true;
    // A single-line select without a selected attribute defaults to its first
    // enabled option. Use that too when checking for a changed selection.
    const defaultOption = options.findLast((option) => option.defaultSelected)
      ?? (node.size <= 1 ? options.find((option) => !optionDisabled(option)) : undefined);
    return selected === defaultOption;
  }

  function valueFor(control, address) {
    let value = control.token === 'street-address'
      ? [address.addressLine1, address.addressLine2].filter((part) => typeof part === 'string' && part.trim()).join(control.node.tagName.toLowerCase() === 'textarea' ? '\n' : ', ')
      : address[fields[control.token]];
    if ((typeof value !== 'string' || !value.trim()) && countryTokens.has(control.token)) {
      value = control.token === 'country' ? address.countryName : address.countryCode;
    }
    if (typeof value !== 'string' || !value.trim()) return null;
    if (control.node.type === 'number' && !/^\d+$/.test(value)) return null;
    if (control.node.tagName.toLowerCase() !== 'select') return value;
    const candidates = [value];
    if (control.token === 'country' && address.countryName) candidates.push(address.countryName);
    if (control.token === 'country-name' && address.countryCode) candidates.push(address.countryCode);
    const normalize = (text) => String(text).trim().toLowerCase();
    const matches = Array.from(control.node.options).filter((option) => !optionDisabled(option)
      && candidates.some((candidate) => normalize(candidate) === normalize(option.value)
        || normalize(candidate) === normalize(option.textContent)));
    return matches.length === 1 ? matches[0].value : null;
  }

  function withNameParts(address) {
    const fullName = address.fullName?.trim();
    if (!fullName || (address.givenName?.trim() && address.familyName?.trim())) return address;
    const given = address.givenName?.trim();
    const family = address.familyName?.trim();
    if (given) {
      return { ...address, familyName: fullName.startsWith(given + ' ') ? fullName.slice(given.length).trim() : '' };
    }
    if (family) {
      return { ...address, givenName: fullName.endsWith(' ' + family) ? fullName.slice(0, -family.length).trim() : '' };
    }
    const [givenName, ...rest] = fullName.split(/\s+/);
    return { ...address, givenName, familyName: rest.join(' ') };
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
    const profile = withNameParts(address);
    // A country change can synchronously populate a region select. Defer
    // that select until after the country regardless of their DOM order.
    const dependentRegion = (control) => control.token === 'address-level1' && control.node.tagName === 'SELECT';
    const fillOrder = chosen.controls.some((control) => countryTokens.has(control.token))
      ? [...chosen.controls.filter((control) => !dependentRegion(control)), ...chosen.controls.filter(dependentRegion)]
      : chosen.controls;
    let filled = 0;
    for (const control of fillOrder) {
      // input/change handlers may synchronously replace other controls. Recheck
      // before each write instead of following stale references into a new form.
      if (!valid()) return { type: 'error', code: 'address_form_changed', filled };
      if (!fillable(control.node)) continue;
      // Earlier change handlers can repopulate a dependent select without
      // replacing its node. Match the current options now, never a cached value
      // that could clear the select after its option was removed.
      const value = valueFor(control, profile);
      if (value === null || value === control.node.value) continue;
      setValue(control.node, value);
      filled += 1;
    }
    return filled ? { type: 'addressFilled', filled } : { type: 'error', code: 'address_no_values' };
  }

  return { collect, describe, apply, setNativeAddressValue };
})();
if (typeof module !== 'undefined') module.exports = mypassAddressForm;
