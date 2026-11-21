// The popup header: a single-key generator button, the "+" menu (creation only) and the "more"
// menu (Open MyPass, Lock MyPass, Help) as macOS-style dropdowns with the WAI-ARIA menu-button
// keyboard model; and the locked panel keeps its form centred and stable when an error appears.
const assert = require('node:assert');
const { test } = require('node:test');
const { openPopup, html } = require('./popupHarness.js');

function menuPopup({ lock = { type: 'ok' }, openApp = { type: 'ok' } } = {}) {
  // `openApp` may be a function of the message (to answer a newEntry request differently).
  if (typeof openApp === 'function') {
    const answer = openApp;
    return openPopup({ reply(message) {
      if (message.type === 'getCredentials') return { type: 'credentialList', credentials: [] };
      if (message.type === 'lockVault') return lock;
      if (message.type === 'openApp') return answer(message);
      return {};
    } });
  }
  return openPopup({ reply(message) {
    if (message.type === 'getCredentials') return { type: 'credentialList', credentials: [] };
    if (message.type === 'lockVault') return lock;
    if (message.type === 'openApp') return openApp;
    return {};
  } });
}

function press(h, id, key, target = h.el(id)) {
  let prevented = false;
  for (const fn of h.el(id).events.keydown || []) {
    fn({ key, target, preventDefault() { prevented = true; } });
  }
  return prevented;
}

function block(id) {
  return html.match(new RegExp(`<div id="${id}"[\\s\\S]*?</div>`))?.[0] || '';
}

test('the generator button is one key turned 45 degrees and keeps its accessible name', () => {
  const button = html.match(/<button id="open-generator-btn"[\s\S]*?<\/button>/)?.[0];
  assert.ok(button);
  assert.match(button, /data-i18n-aria-label="passwordGenerator"/);
  assert.match(button, /data-i18n-title="passwordGenerator"/);
  assert.match(button, /rotate\(-45 12 12\)/);
  assert.equal((button.match(/<circle/g) || []).length, 1, 'one key bow, no dice pips');
  assert.doesNotMatch(button, /<rect/);
  assert.match(button, /stroke="currentColor"/);
});

test('both header buttons are menu buttons wired to role=menu dropdowns', () => {
  for (const [button, menu] of [['add-entry-btn', 'add-entry-menu'], ['more-menu-btn', 'more-menu']]) {
    const tag = html.match(new RegExp(`<button id="${button}"[^>]*>`))?.[0];
    assert.match(tag, /aria-haspopup="menu"/);
    assert.match(tag, /aria-expanded="false"/);
    assert.match(tag, new RegExp(`aria-controls="${menu}"`));
    assert.match(html, new RegExp(`<div id="${menu}" class="mp-menu" role="menu"[^>]*hidden>`));
  }
  assert.match(html, /<button id="more-menu-btn"[^>]*data-i18n-aria-label="moreActions"/);
});

test('the + menu holds only creation items: New login, card, note, address and file', () => {
  const h = menuPopup();
  h.fire('add-entry-btn', 'click');
  const rows = h.el('add-entry-menu').children;
  assert.deepStrictEqual(rows.map((row) => row.children[1].textContent), ['New login', 'New card', 'New note', 'New address', 'New file']);
  assert.deepStrictEqual(rows.map((row) => row.children[0].className), [
    'add-entry-icon add-entry-icon-login', 'add-entry-icon add-entry-icon-card', 'add-entry-icon add-entry-icon-note',
    'add-entry-icon add-entry-icon-address', 'add-entry-icon add-entry-icon-files',
  ]);
  for (const row of rows) {
    assert.match(row.children[0].innerHTML, /^<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"/);
    assert.match(row.children[0].innerHTML, /currentColor/);
  }
  for (const cls of ['add-entry-icon-address', 'add-entry-icon-files']) {
    assert.match(html, new RegExp(`\\.${cls} \\{ color: #[0-9a-f]{6}; \\}`));
  }
  for (const row of rows) {
    assert.equal(row.role, 'menuitem');
    assert.equal(row.tabindex, '-1');
    assert.equal(row.className, 'mp-menu-item');
  }
  const markup = block('add-entry-menu');
  assert.doesNotMatch(markup, /lock-vault-btn|help|lockMyPass/);
});

test('the more menu lists Open MyPass, a separator, Lock MyPass and Help', () => {
  const markup = block('more-menu');
  assert.match(markup, /id="open-app-menu-btn"[\s\S]*data-i18n="openMyPass"[\s\S]*<hr class="mp-menu-separator">[\s\S]*id="lock-vault-btn"[\s\S]*data-i18n="lockMyPass"[\s\S]*id="help-menu-btn"[\s\S]*data-i18n="help"/);
  assert.equal((markup.match(/role="menuitem"/g) || []).length, 3);
  assert.equal((markup.match(/tabindex="-1"/g) || []).length, 3);
});

test('opening a menu focuses its first row; arrows wrap, Home/End jump', () => {
  const h = menuPopup();
  h.fire('more-menu-btn', 'click');
  assert.equal(h.el('more-menu').hidden, false);
  assert.equal(h.el('more-menu-btn').getAttribute('aria-expanded'), 'true');
  assert.equal(h.focused(), 'open-app-menu-btn');
  assert.ok(press(h, 'more-menu', 'ArrowDown', h.el('open-app-menu-btn')));
  assert.equal(h.focused(), 'lock-vault-btn');
  press(h, 'more-menu', 'ArrowDown', h.el('lock-vault-btn'));
  assert.equal(h.focused(), 'help-menu-btn');
  press(h, 'more-menu', 'ArrowDown', h.el('help-menu-btn'));
  assert.equal(h.focused(), 'open-app-menu-btn', 'ArrowDown wraps to the top');
  press(h, 'more-menu', 'ArrowUp', h.el('open-app-menu-btn'));
  assert.equal(h.focused(), 'help-menu-btn', 'ArrowUp wraps to the bottom');
  press(h, 'more-menu', 'Home', h.el('help-menu-btn'));
  assert.equal(h.focused(), 'open-app-menu-btn');
  press(h, 'more-menu', 'End', h.el('open-app-menu-btn'));
  assert.equal(h.focused(), 'help-menu-btn');
  assert.equal(press(h, 'more-menu', 'a', h.el('help-menu-btn')), false, 'other keys pass through');
});

test('Escape closes the menu and returns focus to its button; Tab closes it', () => {
  const h = menuPopup();
  h.fire('more-menu-btn', 'click');
  press(h, 'more-menu', 'Escape', h.el('lock-vault-btn'));
  assert.equal(h.el('more-menu').hidden, true);
  assert.equal(h.el('more-menu-btn').getAttribute('aria-expanded'), 'false');
  assert.equal(h.focused(), 'more-menu-btn');

  h.fire('add-entry-btn', 'click');
  assert.equal(h.el('add-entry-menu').hidden, false);
  press(h, 'add-entry-menu', 'Escape');
  assert.equal(h.el('add-entry-menu').hidden, true);
  assert.equal(h.focused(), 'add-entry-btn');

  h.fire('more-menu-btn', 'click');
  assert.equal(press(h, 'more-menu', 'Tab', h.el('open-app-menu-btn')), false, 'Tab keeps its default move');
  assert.equal(h.el('more-menu').hidden, true);
});

test('ArrowUp on a closed menu button opens it on the last row, ArrowDown on the first', () => {
  const h = menuPopup();
  assert.ok(press(h, 'more-menu-btn', 'ArrowUp'));
  assert.equal(h.el('more-menu').hidden, false);
  assert.equal(h.focused(), 'help-menu-btn');
  press(h, 'more-menu', 'Escape', h.el('help-menu-btn'));
  press(h, 'more-menu-btn', 'ArrowDown');
  assert.equal(h.focused(), 'open-app-menu-btn');
  assert.equal(press(h, 'more-menu-btn', 'Enter'), false, 'Enter is left to the native click');
});

test('the button toggles its menu and only one dropdown is open at a time', () => {
  const h = menuPopup();
  h.fire('add-entry-btn', 'click');
  h.fire('more-menu-btn', 'click');
  assert.equal(h.el('add-entry-menu').hidden, true);
  assert.equal(h.el('add-entry-btn').getAttribute('aria-expanded'), 'false');
  assert.equal(h.el('more-menu').hidden, false);
  h.fire('more-menu-btn', 'click');
  assert.equal(h.el('more-menu').hidden, true);
  h.fire('more-menu-btn', 'click');
  h.fire('category-btn', 'click');
  assert.equal(h.el('more-menu').hidden, true, 'the category picker closes the header menus');
});

test('Open MyPass asks the app to come forward without an error', async () => {
  const h = menuPopup();
  h.fire('more-menu-btn', 'click');
  h.fire('open-app-menu-btn', 'click');
  assert.equal(h.el('more-menu').hidden, true);
  assert.deepStrictEqual(h.sent.at(-1), { type: 'openApp' });
  await h.advance(0);
  assert.equal(h.el('lock-vault-error').hidden, true);
});

test('Open MyPass failures are shown under the header', async () => {
  for (const [reply, pattern] of [[{ type: 'pairingRequired' }, /Connect this browser/], [{ type: 'error' }, /Could not open MyPass/]]) {
    const h = menuPopup({ openApp: reply });
    h.fire('more-menu-btn', 'click');
    h.fire('open-app-menu-btn', 'click');
    await h.advance(0);
    assert.equal(h.el('lock-vault-error').hidden, false);
    assert.match(h.el('lock-vault-error').textContent, pattern);
  }
});

function clickRow(h, label) {
  h.fire('add-entry-btn', 'click');
  const row = h.el('add-entry-menu').children.find((r) => r.children[1].textContent === label);
  assert.ok(row, label);
  for (const fn of row.events.click || []) fn({ preventDefault() {}, target: row });
}

test('New address and New file ask MyPass to open its editor and close the popup', async () => {
  for (const [label, newEntry] of [['New address', 'Address'], ['New file', 'Files']]) {
    const h = menuPopup();
    clickRow(h, label);
    assert.equal(h.el('add-entry-menu').hidden, true);
    assert.deepStrictEqual(h.sent.at(-1), { type: 'openApp', newEntry });
    await h.advance(0);
    assert.equal(h.closed(), 1);
    assert.equal(h.el('lock-vault-error').hidden, true);
  }
});

test('New login, card and note still use the extension create window, never openApp', () => {
  for (const [label, type] of [['New login', 'Login'], ['New card', 'Card'], ['New note', 'Notes']]) {
    const h = menuPopup();
    clickRow(h, label);
    assert.ok(!h.sent.some((m) => m.type === 'openApp'), label);
    assert.equal(h.windowsCreated.length, 1);
    assert.match(h.windowsCreated[0], new RegExp(`create\\.html\\?type=${type}`));
  }
});

test('New address and New file failures stay in the popup and are shown under the header', async () => {
  for (const [reply, pattern] of [[{ type: 'pairingRequired' }, /Connect this browser/], [{ type: 'error' }, /Could not open MyPass/], [{}, /Could not open MyPass/]]) {
    const h = menuPopup({ openApp: () => reply });
    clickRow(h, 'New file');
    await h.advance(0);
    assert.equal(h.closed(), 0);
    assert.equal(h.el('lock-vault-error').hidden, false);
    assert.match(h.el('lock-vault-error').textContent, pattern);
  }
});

test('New address when MyPass is not running shows the locked panel with its Launch button', async () => {
  const h = menuPopup({ openApp: () => ({ type: 'error', message: 'app_not_running' }) });
  clickRow(h, 'New address');
  await h.advance(0);
  assert.equal(h.closed(), 0);
  assert.equal(h.el('locked-panel').hidden, false);
  assert.equal(h.el('launch-app-btn').hidden, false);
});

test('Help opens the help card; Back and Escape return focus to the more button', () => {
  const h = menuPopup();
  h.fire('more-menu-btn', 'click');
  h.fire('help-menu-btn', 'click');
  assert.equal(h.el('more-menu').hidden, true);
  assert.equal(h.el('help-popover').hidden, false);
  assert.equal(h.focused(), 'help-back-btn');
  h.fire('help-back-btn', 'click');
  assert.equal(h.el('help-popover').hidden, true);
  assert.equal(h.focused(), 'more-menu-btn');
  h.fire('more-menu-btn', 'click');
  h.fire('help-menu-btn', 'click');
  press(h, 'help-popover', 'Escape', h.el('help-back-btn'));
  assert.equal(h.el('help-popover').hidden, true);
  assert.equal(h.focused(), 'more-menu-btn');
  h.fire('more-menu-btn', 'click');
  h.fire('help-menu-btn', 'click');
  h.fire('add-entry-btn', 'click');
  assert.equal(h.el('help-popover').hidden, true, 'another menu closes the help card');
});

test('Lock MyPass keeps its replies and errors', () => {
  for (const outcome of ['ok', 'locked', 'pairingRequired', 'error']) {
    const h = menuPopup({ lock: { type: outcome } });
    h.fire('more-menu-btn', 'click');
    h.fire('lock-vault-btn', 'click');
    assert.equal(h.el('more-menu').hidden, true);
    if (outcome === 'ok' || outcome === 'locked') {
      assert.equal(h.el('locked-panel').hidden, false);
      assert.equal(h.el('more-menu-btn').hidden, true);
    } else {
      assert.equal(h.el('lock-vault-error').hidden, false);
      assert.match(h.el('lock-vault-error').textContent, outcome === 'error' ? /lock/i : /Connect this browser/);
      assert.equal(h.focused(), 'more-menu-btn');
      h.fire('more-menu-btn', 'click');
      assert.equal(h.el('lock-vault-error').hidden, true, 'reopening a menu clears the error');
    }
  }
});

test('the Lock row shows the assigned shortcut and nothing when none is set', () => {
  const { api } = menuPopup();
  assert.equal(api.lockShortcutLabel([{ name: 'lock-vault', shortcut: '⇧⌘L' }]), '⇧⌘L');
  assert.equal(api.lockShortcutLabel([{ name: 'lock-vault', shortcut: '' }]), '');
  assert.equal(api.lockShortcutLabel([{ name: '_execute_action', shortcut: '⇧⌘Y' }]), '');
  assert.equal(api.lockShortcutLabel(undefined), '');
  assert.match(block('more-menu'), /id="lock-shortcut-hint" class="mp-menu-shortcut"[^>]*hidden/);
});

test('the locked panel centres its form and reserves the error line', () => {
  const panel = html.match(/#locked-panel:not\(\[hidden\]\) \{([^}]*)\}/)?.[1] || '';
  assert.match(panel, /align-content: center/);
  assert.match(panel, /min-height: \d+px/);
  const reserved = html.match(/#unlock-error\[hidden\] \{([^}]*)\}/)?.[1] || '';
  assert.match(reserved, /display: block !important/);
  assert.match(reserved, /visibility: hidden/);
  assert.match(html.match(/#unlock-error \{([^}]*)\}/)?.[1] || '', /min-height: 16px/);
});
