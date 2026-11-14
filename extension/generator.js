// Password generator window. Smart passwords come from the app so site rules
// still apply. Random, memorable, and PIN are built here. "Use by default for
// suggestions" is what the in-page Generate button and the popup's one-click
// Generate use when the chosen type is not Smart.

if (typeof document !== 'undefined') {
  const TYPE_LABELS = {
    smart: 'Smart password',
    random: 'Random password',
    memorable: 'Memorable password',
    pin: 'PIN'
  };
  const params = new URLSearchParams(location.search);
  const purpose = params.get('purpose') === 'insert' ? 'insert' : 'autofill';
  const pageUrl = params.get('url') || '';
  let state = defaultGeneratorSettings();
  let currentPassword = '';
  let hydrating = false;
  let smartRequest = 0;
  let latestEntries = [];
  let historyNotice = '';
  let openMenuId = null;
  const revealed = new Set();

  const historyView = document.getElementById('history-view');
  const copyBtn = document.getElementById('copy-btn');
  const autofillBtn = document.getElementById('autofill-btn');
  const passwordDisplay = document.getElementById('password-display');
  const strengthFill = document.getElementById('strength-fill');
  const generatorError = document.getElementById('generator-error');
  const typeBtn = document.getElementById('type-btn');
  const typeMenu = document.getElementById('type-menu');
  const smartHint = document.getElementById('smart-hint');
  const randomOptions = document.getElementById('random-options');
  const memorableOptions = document.getElementById('memorable-options');
  const pinOptions = document.getElementById('pin-options');
  const randomLength = document.getElementById('random-length');
  const randomLengthNumber = document.getElementById('random-length-number');
  const randomDigits = document.getElementById('random-digits');
  const randomSymbols = document.getElementById('random-symbols');
  const memorableWords = document.getElementById('memorable-words');
  const memorableWordsNumber = document.getElementById('memorable-words-number');
  const memorableSeparator = document.getElementById('memorable-separator');
  const memorableCapitalize = document.getElementById('memorable-capitalize');
  const memorableFullWords = document.getElementById('memorable-full-words');
  const pinLength = document.getElementById('pin-length');
  const pinLengthNumber = document.getElementById('pin-length-number');
  const useDefault = document.getElementById('use-default');
  const historySearch = document.getElementById('history-search');
  const historyList = document.getElementById('history-list');

  const canFillSite = /^https?:\/\//i.test(pageUrl);
  if (purpose === 'insert') autofillBtn.textContent = 'Use password';
  else if (!canFillSite) {
    autofillBtn.disabled = true;
    autofillBtn.title = 'Open a website to fill this password.';
  }

  function pageDomain() {
    if (!canFillSite) return '';
    try {
      return new URL(pageUrl).hostname;
    } catch (e) {
      return '';
    }
  }

  function setError(text) {
    generatorError.hidden = !text;
    generatorError.textContent = text || '';
  }

  function renderPassword(password) {
    passwordDisplay.textContent = '';
    for (const char of String(password || '')) {
      const span = document.createElement('span');
      if (/[0-9]/.test(char)) span.className = 'pw-digit';
      else if (/[a-zA-Z]/.test(char)) span.className = 'pw-letter';
      else span.className = 'pw-symbol';
      span.textContent = char;
      passwordDisplay.appendChild(span);
    }
  }

  function renderStrength(password) {
    const strength = passwordStrength(password);
    strengthFill.style.width = (strength * 100) + '%';
    strengthFill.style.background = strengthColor(strength);
  }

  function showPassword(password) {
    currentPassword = password || '';
    setError('');
    renderPassword(currentPassword);
    renderStrength(currentPassword);
  }

  function recordLocal(password) {
    if (!password || state.type === 'smart') return;
    chrome.runtime.sendMessage({
      type: 'recordGeneratedPassword',
      password: password,
      url: pageUrl
    }, () => { void chrome.runtime.lastError; });
  }

  function generate(record) {
    if (state.type === 'smart') {
      const requestId = ++smartRequest;
      setError('');
      chrome.runtime.sendMessage({
        type: 'generatePassword',
        domain: pageDomain(),
        url: pageUrl,
        preview: true
      }, (response) => {
        if (requestId !== smartRequest) return;
        if (!response || response.type !== 'generatedPassword' || !response.password) {
          showPassword('');
          setError(response && response.type === 'locked'
            ? 'MyPass is locked.'
            : ((response && response.message) || 'Could not generate a password.'));
          return;
        }
        showPassword(response.password);
      });
      return;
    }
    const password = generateFromSettings(state);
    showPassword(password);
    if (record) recordLocal(password);
  }

  function saveSettings() {
    chrome.storage.local.set({ [GENERATOR_SETTINGS_KEY]: state });
  }

  function updateTypeUI() {
    typeBtn.textContent = TYPE_LABELS[state.type] || TYPE_LABELS.smart;
    document.querySelectorAll('.type-option').forEach((button) => {
      button.classList.toggle('selected', button.getAttribute('data-type') === state.type);
    });
    randomOptions.hidden = state.type !== 'random';
    memorableOptions.hidden = state.type !== 'memorable';
    pinOptions.hidden = state.type !== 'pin';
    const host = pageDomain();
    smartHint.hidden = state.type !== 'smart' || !host;
    smartHint.textContent = host ? 'MyPass suggests a password that matches this site.' : '';
  }

  function selectType(type) {
    state.type = type;
    typeMenu.hidden = true;
    updateTypeUI();
    if (hydrating) return;
    saveSettings();
    generate(true);
  }

  function bindRange(range, number, min, max, read) {
    range.addEventListener('input', () => {
      number.value = range.value;
      if (hydrating) return;
      read(Number(range.value));
      saveSettings();
      generate(false);
    });
    range.addEventListener('change', () => {
      if (!hydrating) recordLocal(currentPassword);
    });
    number.addEventListener('change', () => {
      const value = clampNumber(number.value, min, max, Number(range.value));
      range.value = String(value);
      number.value = String(value);
      if (hydrating) return;
      read(value);
      saveSettings();
      generate(true);
    });
  }

  function bindToggle(input, apply) {
    input.addEventListener('change', () => {
      if (hydrating) return;
      apply(input.checked);
      saveSettings();
      generate(true);
    });
  }

  function copyText(text) {
    if (!text) return;
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(() => {
        copyBtn.textContent = 'Copied';
        setTimeout(() => { copyBtn.textContent = 'Copy'; }, 1200);
      }).catch(() => {});
    }
  }

  function renderHistory(entries) {
    const filtered = filterPasswordHistory(entries, historySearch.value);
    historyList.textContent = '';
    if (!filtered.length) {
      const empty = document.createElement('p');
      empty.className = 'history-empty';
      empty.textContent = historyNotice
        ? historyNotice
        : (historySearch.value.trim() ? 'No matching passwords.' : 'No generated passwords yet.');
      historyList.appendChild(empty);
      return;
    }
    groupPasswordHistory(filtered).forEach((group) => {
      const heading = document.createElement('h2');
      heading.className = 'history-day';
      heading.textContent = group.label;
      historyList.appendChild(heading);
      group.entries.forEach((entry) => historyList.appendChild(renderHistoryRow(entry)));
    });
  }

  function renderHistoryRow(entry) {
    const row = document.createElement('article');
    row.className = 'history-row' + (openMenuId === entry.id ? ' menu-open' : '');
    const top = document.createElement('div');
    top.className = 'history-top';
    const title = document.createElement('div');
    title.className = 'history-title';
    const host = historyHost(entry.url);
    title.textContent = formatHistoryTime(entry.createdAt) + (host ? ' - ' + host : '');
    const actions = document.createElement('div');
    actions.className = 'history-actions';
    const show = document.createElement('button');
    show.type = 'button';
    show.className = 'history-show';
    const isShown = revealed.has(entry.id);
    show.textContent = isShown ? 'Hide' : 'Show';
    show.addEventListener('click', () => {
      if (revealed.has(entry.id)) revealed.delete(entry.id);
      else revealed.add(entry.id);
      renderHistory(latestEntries);
    });
    const menuBtn = document.createElement('button');
    menuBtn.type = 'button';
    menuBtn.className = 'history-menu-btn';
    menuBtn.setAttribute('aria-label', 'Password actions');
    menuBtn.textContent = '▾';
    const menu = document.createElement('div');
    menu.className = 'history-menu';
    menu.hidden = openMenuId !== entry.id;
    const copy = document.createElement('button');
    copy.type = 'button';
    copy.textContent = 'Copy';
    copy.addEventListener('click', () => {
      copyText(entry.password);
      openMenuId = null;
      renderHistory(latestEntries);
    });
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.textContent = 'Delete';
    remove.addEventListener('click', () => {
      chrome.runtime.sendMessage({ type: 'deleteGeneratedPassword', id: entry.id }, () => {
        latestEntries = latestEntries.filter((item) => item.id !== entry.id);
        revealed.delete(entry.id);
        openMenuId = null;
        renderHistory(latestEntries);
      });
    });
    menu.appendChild(copy);
    menu.appendChild(remove);
    menuBtn.addEventListener('click', (event) => {
      event.stopPropagation();
      openMenuId = openMenuId === entry.id ? null : entry.id;
      renderHistory(latestEntries);
    });
    actions.appendChild(show);
    actions.appendChild(menuBtn);
    top.appendChild(title);
    top.appendChild(actions);
    row.appendChild(top);
    row.appendChild(menu);
    const secret = document.createElement('div');
    secret.className = 'history-secret' + (isShown ? ' is-revealed' : '');
    secret.textContent = isShown ? entry.password : '••••••••';
    row.appendChild(secret);
    return row;
  }

  function loadHistory() {
    chrome.runtime.sendMessage({ type: 'getGeneratedPasswordHistory' }, (response) => {
      if (!response || response.type !== 'generatedPasswordHistory') {
        latestEntries = [];
        historyNotice = response && response.type === 'locked'
          ? 'Unlock MyPass to see password history.'
          : 'MyPass is unavailable.';
        renderHistory(latestEntries);
        return;
      }
      historyNotice = '';
      latestEntries = response.entries || [];
      renderHistory(latestEntries);
    });
  }

  function applySettings(next) {
    hydrating = true;
    state = next;
    randomLength.value = String(state.random.length);
    randomLengthNumber.value = String(state.random.length);
    randomDigits.checked = state.random.digits;
    randomSymbols.checked = state.random.symbols;
    memorableWords.value = String(state.memorable.words);
    memorableWordsNumber.value = String(state.memorable.words);
    memorableSeparator.value = state.memorable.separator;
    memorableCapitalize.checked = state.memorable.capitalize;
    memorableFullWords.checked = state.memorable.fullWords;
    pinLength.value = String(state.pin.length);
    pinLengthNumber.value = String(state.pin.length);
    useDefault.checked = state.useAsDefault;
    updateTypeUI();
    hydrating = false;
    generate(true);
  }

  bindRange(randomLength, randomLengthNumber, 8, 64, (value) => { state.random.length = value; });
  bindRange(memorableWords, memorableWordsNumber, 3, 10, (value) => { state.memorable.words = value; });
  bindRange(pinLength, pinLengthNumber, 3, 12, (value) => { state.pin.length = value; });
  bindToggle(randomDigits, (checked) => { state.random.digits = checked; });
  bindToggle(randomSymbols, (checked) => { state.random.symbols = checked; });
  bindToggle(memorableCapitalize, (checked) => { state.memorable.capitalize = checked; });
  bindToggle(memorableFullWords, (checked) => { state.memorable.fullWords = checked; });
  memorableSeparator.addEventListener('change', () => {
    if (hydrating) return;
    state.memorable.separator = memorableSeparator.value;
    saveSettings();
    generate(true);
  });
  useDefault.addEventListener('change', () => {
    if (hydrating) return;
    state.useAsDefault = useDefault.checked;
    saveSettings();
  });
  typeBtn.addEventListener('click', () => {
    typeMenu.hidden = !typeMenu.hidden;
  });
  document.querySelectorAll('.type-option').forEach((button) => {
    button.addEventListener('click', () => selectType(button.getAttribute('data-type')));
  });
  copyBtn.addEventListener('click', () => copyText(currentPassword));
  document.getElementById('regenerate-btn').addEventListener('click', () => generate(true));
  autofillBtn.addEventListener('click', () => {
    if (!currentPassword) return;
    if (purpose === 'insert') {
      chrome.storage.session.set({
        insertGeneratedPassword: { password: currentPassword, at: Date.now() }
      }, () => window.close());
      return;
    }
    chrome.runtime.sendMessage({
      type: 'fillGeneratedPassword',
      password: currentPassword,
      url: pageUrl
    }, (response) => {
      if (response && response.type === 'generatedPasswordFilled') window.close();
    });
  });
  document.getElementById('close-btn').addEventListener('click', () => window.close());

  let historyCloseTimer = 0;
  let historyCloseListener = null;

  function historySlides() {
    return !(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  }

  function cancelHistoryClose() {
    if (historyCloseTimer) {
      window.clearTimeout(historyCloseTimer);
      historyCloseTimer = 0;
    }
    if (historyCloseListener) {
      historyView.removeEventListener('transitionend', historyCloseListener);
      historyCloseListener = null;
    }
  }

  function openHistory() {
    cancelHistoryClose();
    typeMenu.hidden = true;
    historyView.hidden = false;
    const reveal = () => document.body.classList.add('show-history');
    if (historySlides()) requestAnimationFrame(() => requestAnimationFrame(reveal));
    else reveal();
    loadHistory();
  }

  function closeHistory() {
    cancelHistoryClose();
    document.body.classList.remove('show-history');
    const finish = () => {
      if (document.body.classList.contains('show-history')) return;
      historyView.hidden = true;
      openMenuId = null;
    };
    if (!historySlides()) {
      finish();
      return;
    }
    historyCloseListener = (event) => {
      if (event.target !== historyView || event.propertyName !== 'transform') return;
      cancelHistoryClose();
      finish();
    };
    historyView.addEventListener('transitionend', historyCloseListener);
    historyCloseTimer = window.setTimeout(() => {
      cancelHistoryClose();
      finish();
    }, 320);
  }

  document.getElementById('history-link').addEventListener('click', openHistory);
  document.getElementById('history-back').addEventListener('click', closeHistory);
  historySearch.addEventListener('input', () => renderHistory(latestEntries));
  document.addEventListener('click', (event) => {
    if (!typeBtn.contains(event.target) && !typeMenu.contains(event.target)) typeMenu.hidden = true;
    if (openMenuId && !event.target.closest('.history-menu') && !event.target.closest('.history-menu-btn')) {
      openMenuId = null;
      renderHistory(latestEntries);
    }
  });

  chrome.storage.local.get([GENERATOR_SETTINGS_KEY], (stored) => {
    applySettings(normalizeGeneratorSettings(stored && stored[GENERATOR_SETTINGS_KEY]));
  });
}
