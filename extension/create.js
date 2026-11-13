// New login, card, and note window. The layout follows the Mac editors
// (EntryEditView, CardEditView, NotesEditView). The bridge has no group list,
// so this window does not offer a group picker and the app stores the entry
// in the vault root.

const INSERT_PASSWORD_KEY = 'insertGeneratedPassword';

const MATCH_HELP = {
  'registrable-domain': 'Matches example.com and any subdomain, such as login.example.com.',
  host: 'Matches only this exact hostname and port. A different subdomain will not match.',
  'starts-with': 'Matches any page on the same site as this address — the scheme and host must be identical, so a lookalike domain such as example.com.attacker.com will never match even though it starts with the same text. Within that same site, this can still match more than intended: example.com/app also matches example.com/app-legacy, since both begin the same way.',
  exact: 'Matches only this exact page address, including its path and query.',
  never: 'Hides this entry from the browser extension only — it stays fully visible and usable inside MyPass.'
};

function digitsOnly(raw) {
  return String(raw || '').replace(/\D/g, '');
}

function formatCardNumber(raw) {
  const digits = digitsOnly(raw).slice(0, 19);
  return digits.replace(/(\d{4})(?=\d)/g, '$1 ');
}

function formatExpiry(raw) {
  const digits = digitsOnly(raw).slice(0, 4);
  if (digits.length <= 2) return digits;
  return digits.slice(0, 2) + '/' + digits.slice(2);
}

function wrapMarkdown(value, start, end, prefix, suffix) {
  const text = String(value || '');
  const from = Math.max(0, Math.min(start, text.length));
  const to = Math.max(from, Math.min(end, text.length));
  const selected = text.slice(from, to);
  const next = text.slice(0, from) + prefix + selected + suffix + text.slice(to);
  const cursor = selected ? from + prefix.length + selected.length + suffix.length : from + prefix.length;
  return { value: next, cursor: cursor };
}

function describeCreateError(response) {
  if (!response) return 'Could not save.';
  if (response.type === 'locked') return 'MyPass is locked.';
  if (response.type === 'pairingRequired') return 'Connect MyPass first.';
  if (response.message === 'title required') return 'Title is required.';
  return response.message || 'Could not save.';
}

if (typeof document !== 'undefined') {
  const params = new URLSearchParams(location.search);
  const requested = params.get('type');
  const entryType = requested === 'Card' || requested === 'Notes' ? requested : 'Login';
  let favorite = false;
  let saving = false;

  const navTitle = document.getElementById('nav-title');
  const hero = document.getElementById('hero');
  const heroIcon = document.getElementById('hero-icon');
  const heroLetter = document.getElementById('hero-letter');
  const heroTitle = document.getElementById('hero-title');
  const loginCard = document.getElementById('login-card');
  const cardCard = document.getElementById('card-card');
  const notesEditor = document.getElementById('notes-editor');
  const formError = document.getElementById('form-error');
  const favoriteBtn = document.getElementById('favorite-btn');
  const loginTitle = document.getElementById('login-title');
  const cardTitle = document.getElementById('card-title');
  const notesTitle = document.getElementById('notes-title');
  const loginPassword = document.getElementById('login-password');
  const loginUrl = document.getElementById('login-url');
  const matchMode = document.getElementById('match-mode');
  const matchHelp = document.getElementById('match-help');
  const cardNumber = document.getElementById('card-number');
  const cardExpiry = document.getElementById('card-expiry');
  const notesBody = document.getElementById('notes-body');

  const titles = { Login: 'New Entry', Card: 'New Card', Notes: 'New Note' };
  navTitle.textContent = titles[entryType];
  document.title = titles[entryType];
  document.body.classList.toggle('is-notes', entryType === 'Notes');
  hero.hidden = entryType === 'Notes';
  loginCard.hidden = entryType !== 'Login';
  cardCard.hidden = entryType !== 'Card';
  notesEditor.hidden = entryType !== 'Notes';

  if (entryType === 'Login' && params.get('url')) loginUrl.value = params.get('url');
  if (entryType === 'Login' && params.get('username')) {
    document.getElementById('login-username').value = params.get('username');
  }

  function titleValue() {
    if (entryType === 'Notes') return notesTitle.value;
    if (entryType === 'Card') return cardTitle.value;
    return loginTitle.value;
  }

  function paintHero() {
    const typed = titleValue().trim();
    const shown = typed || '(untitled)';
    heroTitle.textContent = shown;
    heroLetter.textContent = letterFromTitle(shown);
    heroIcon.style.background = colorForTitle(shown);
  }

  function setError(text) {
    formError.hidden = !text;
    formError.textContent = text || '';
  }

  function paintFavorite() {
    favoriteBtn.textContent = favorite ? '★' : '☆';
    favoriteBtn.setAttribute('aria-pressed', favorite ? 'true' : 'false');
    favoriteBtn.setAttribute('aria-label', favorite ? 'Remove from Favorites' : 'Add to Favorites');
  }

  function paintMatchHelp() {
    matchHelp.textContent = MATCH_HELP[matchMode.value] || MATCH_HELP['registrable-domain'];
  }

  function openGeneratorForInsert() {
    if (!chrome.windows || typeof chrome.windows.create !== 'function') return;
    const screen = window.screen || {};
    const width = 420;
    const height = 640;
    const availWidth = screen.availWidth || width;
    const availHeight = screen.availHeight || height;
    chrome.windows.create({
      url: chrome.runtime.getURL('generator.html?purpose=insert'),
      type: 'popup',
      width: width,
      height: height,
      left: Math.round((screen.availLeft || 0) + Math.max(0, (availWidth - width) / 2)),
      top: Math.round((screen.availTop || 0) + Math.max(0, (availHeight - height) / 2)),
      focused: true
    });
  }

  function bindReveal(button, field, showLabel, hideLabel) {
    button.addEventListener('click', () => {
      const revealed = field.type === 'text';
      field.type = revealed ? 'password' : 'text';
      button.textContent = revealed ? 'Show' : 'Hide';
      button.setAttribute('aria-label', revealed ? showLabel : hideLabel);
    });
  }

  function saveEntry() {
    if (saving) return;
    setError('');
    saving = true;
    const saveBtn = document.getElementById('save-btn');
    saveBtn.disabled = true;
    const notes = entryType === 'Notes'
      ? notesBody.value
      : entryType === 'Login' ? document.getElementById('login-notes').value : '';
    const otpVisible = !document.getElementById('otp-row').hidden;
    chrome.runtime.sendMessage({
      type: 'createEntry',
      entryType: entryType,
      title: titleValue(),
      username: entryType === 'Login' ? document.getElementById('login-username').value : '',
      password: entryType === 'Login' ? loginPassword.value : '',
      url: entryType === 'Login' ? loginUrl.value : '',
      cardNumber: entryType === 'Card' ? digitsOnly(cardNumber.value) : '',
      cardholderName: entryType === 'Card' ? document.getElementById('card-holder').value : '',
      expiry: entryType === 'Card' ? cardExpiry.value : '',
      cvv: entryType === 'Card' ? document.getElementById('card-cvv').value : '',
      notes: notes,
      favorite: favorite,
      match: entryType === 'Login' ? matchMode.value : '',
      otp: entryType === 'Login' && otpVisible ? document.getElementById('otp-input').value : ''
    }, (response) => {
      saving = false;
      saveBtn.disabled = false;
      if (response && response.type === 'entryCreated') {
        window.close();
        return;
      }
      setError(describeCreateError(response));
    });
  }

  function applyMarkdown(prefix, suffix) {
    const wrapped = wrapMarkdown(notesBody.value, notesBody.selectionStart, notesBody.selectionEnd, prefix, suffix);
    notesBody.value = wrapped.value;
    notesBody.focus();
    notesBody.setSelectionRange(wrapped.cursor, wrapped.cursor);
  }

  loginTitle.addEventListener('input', paintHero);
  cardTitle.addEventListener('input', paintHero);
  matchMode.addEventListener('change', paintMatchHelp);
  favoriteBtn.addEventListener('click', () => {
    favorite = !favorite;
    paintFavorite();
  });
  document.getElementById('cancel-btn').addEventListener('click', () => window.close());
  document.getElementById('save-btn').addEventListener('click', saveEntry);
  document.addEventListener('keydown', (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault();
      saveEntry();
    }
  });
  document.getElementById('login-generate').addEventListener('click', openGeneratorForInsert);
  bindReveal(
    document.getElementById('login-password-reveal'), loginPassword, 'Show password', 'Hide password'
  );
  bindReveal(
    document.getElementById('card-number-reveal'), cardNumber, 'Show card number', 'Hide card number'
  );
  bindReveal(
    document.getElementById('card-cvv-reveal'), document.getElementById('card-cvv'), 'Show CVV', 'Hide CVV'
  );
  cardNumber.addEventListener('input', () => {
    cardNumber.value = formatCardNumber(cardNumber.value);
  });
  cardExpiry.addEventListener('input', () => {
    cardExpiry.value = formatExpiry(cardExpiry.value);
  });
  document.getElementById('otp-add').addEventListener('click', () => {
    document.getElementById('otp-add-row').hidden = true;
    document.getElementById('otp-row').hidden = false;
    document.getElementById('otp-input').focus();
  });
  document.getElementById('otp-remove').addEventListener('click', () => {
    document.getElementById('otp-input').value = '';
    document.getElementById('otp-row').hidden = true;
    document.getElementById('otp-add-row').hidden = false;
  });
  document.getElementById('md-bold').addEventListener('click', () => applyMarkdown('**', '**'));
  document.getElementById('md-italic').addEventListener('click', () => applyMarkdown('*', '*'));
  document.getElementById('md-code').addEventListener('click', () => applyMarkdown('`', '`'));

  if (chrome.storage && chrome.storage.onChanged && entryType === 'Login') {
    chrome.storage.onChanged.addListener((changes, area) => {
      if (area !== 'session' || !changes[INSERT_PASSWORD_KEY] || !changes[INSERT_PASSWORD_KEY].newValue) return;
      const next = changes[INSERT_PASSWORD_KEY].newValue;
      if (!next.password) return;
      loginPassword.value = next.password;
      chrome.storage.session.remove(INSERT_PASSWORD_KEY);
    });
  }

  paintHero();
  paintFavorite();
  paintMatchHelp();
}

if (typeof module !== 'undefined') {
  module.exports = {
    INSERT_PASSWORD_KEY,
    MATCH_HELP,
    digitsOnly,
    formatCardNumber,
    formatExpiry,
    wrapMarkdown,
    describeCreateError
  };
}
