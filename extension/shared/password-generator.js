// Local password shapes the Mac app's generator does not have: random length
// with digits/symbols, a word password, and a PIN. Smart passwords stay in the
// app so site rules still apply. The service worker importScripts this file
// before background.js. The generator window loads it as a classic script.
// Node tests require() it.

// `var`, not const: background.js reaches this through globalThis in the service
// worker, and a global const is not a property of the global object.
var GENERATOR_SETTINGS_KEY = 'passwordGeneratorSettings';

const GENERATOR_WORDS = [
  'able', 'account', 'apple', 'april', 'autumn', 'baker', 'balance', 'bamboo', 'banner', 'barrel',
  'basket', 'beach', 'beacon', 'berry', 'bicycle', 'birch', 'blanket', 'blossom', 'bonus', 'bottle',
  'branch', 'breeze', 'bridge', 'bright', 'bronze', 'cabin', 'camel', 'candle', 'canyon', 'carpet',
  'castle', 'cedar', 'cellar', 'chair', 'cherry', 'citrus', 'cloud', 'cobalt', 'coffee', 'comet',
  'copper', 'coral', 'corner', 'cottage', 'cradle', 'crane', 'creek', 'crimson', 'crystal', 'daisy',
  'dancer', 'delta', 'desert', 'dinner', 'dolphin', 'dragon', 'drawer', 'eagle', 'earth', 'ember',
  'engine', 'falcon', 'feather', 'field', 'forest', 'fossil', 'frost', 'garden', 'ginger', 'glacier',
  'glove', 'golden', 'granite', 'gravel', 'harbor', 'harvest', 'hazel', 'heron', 'honey', 'horizon',
  'island', 'ivory', 'jacket', 'jasmine', 'journal', 'kettle', 'kindle', 'ladder', 'lantern', 'lemon',
  'linen', 'lizard', 'lodge', 'lotus', 'magnet', 'maple', 'marble', 'meadow', 'melon', 'mercury',
  'mirror', 'moment', 'mosaic', 'mountain', 'needle', 'nectar', 'nickel', 'north', 'notebook', 'novel',
  'oasis', 'ocean', 'olive', 'onyx', 'orange', 'orchid', 'otter', 'paddle', 'palace', 'panda',
  'paper', 'pastel', 'pebble', 'pepper', 'piano', 'pilot', 'pine', 'planet', 'plum', 'pocket',
  'pond', 'poplar', 'prairie', 'prism', 'pumpkin', 'quartz', 'quiet', 'rabbit', 'radar', 'raisin',
  'ranger', 'raven', 'ribbon', 'river', 'robin', 'rocket', 'saffron', 'sailor', 'sand', 'scarlet',
  'shadow', 'sierra', 'silver', 'sketch', 'solar', 'sparrow', 'spring', 'stone', 'summer', 'sunset',
  'table', 'thunder', 'tiger', 'timber', 'topaz', 'tower', 'trail', 'tulip', 'turtle', 'umber',
  'valley', 'velvet', 'violet', 'voyage', 'walnut', 'willow', 'window', 'winter', 'yellow', 'zephyr'
];

const GENERATOR_SEPARATORS = {
  hyphen: '-',
  space: ' ',
  period: '.',
  comma: ',',
  underscore: '_'
};

function randomUnit() {
  if (typeof crypto !== 'undefined' && crypto.getRandomValues) {
    const buf = new Uint32Array(1);
    crypto.getRandomValues(buf);
    return buf[0] / 4294967296;
  }
  return Math.random();
}

function randomIndex(size) {
  return Math.floor(randomUnit() * size);
}

function clampNumber(value, min, max, fallback) {
  const number = Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.min(max, Math.max(min, Math.round(number)));
}

function pickChar(alphabet) {
  return alphabet.charAt(randomIndex(alphabet.length));
}

function shuffleChars(chars) {
  const copy = chars.slice();
  for (let index = copy.length - 1; index > 0; index -= 1) {
    const swap = randomIndex(index + 1);
    const held = copy[index];
    copy[index] = copy[swap];
    copy[swap] = held;
  }
  return copy;
}

function generateRandomPassword(options) {
  const settings = options || {};
  const length = clampNumber(settings.length, 8, 64, 20);
  const digits = settings.digits !== false;
  const symbols = settings.symbols !== false;
  const lower = 'abcdefghijklmnopqrstuvwxyz';
  const upper = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';
  const digit = '0123456789';
  const symbol = '!@#$%^&*()-_=+[]{};:,.<>?';
  let pool = lower + upper;
  const required = [pickChar(lower), pickChar(upper)];
  if (digits) {
    pool += digit;
    required.push(pickChar(digit));
  }
  if (symbols) {
    pool += symbol;
    required.push(pickChar(symbol));
  }
  const chars = required.slice();
  while (chars.length < length) chars.push(pickChar(pool));
  return shuffleChars(chars).join('');
}

function generatePinPassword(options) {
  const length = clampNumber(options && options.length, 3, 12, 4);
  let pin = '';
  for (let index = 0; index < length; index += 1) pin += String(randomIndex(10));
  return pin;
}

function shortPasswordChunk() {
  const consonants = 'bcdfghjklmnpqrstvwxyz';
  const vowels = 'aeiou';
  const patterns = ['cvc', 'cvcv', 'cvcc'];
  const pattern = patterns[randomIndex(patterns.length)];
  let word = '';
  for (const kind of pattern) {
    word += pickChar(kind === 'c' ? consonants : vowels);
  }
  return word;
}

function pickWords(count) {
  const copy = GENERATOR_WORDS.slice();
  const chosen = [];
  const limit = Math.min(count, copy.length);
  for (let index = 0; index < limit; index += 1) {
    chosen.push(copy.splice(randomIndex(copy.length), 1)[0]);
  }
  while (chosen.length < count) chosen.push(GENERATOR_WORDS[randomIndex(GENERATOR_WORDS.length)]);
  return chosen;
}

function generateMemorablePassword(options) {
  const settings = options || {};
  const count = clampNumber(settings.words, 3, 10, 4);
  const separator = Object.prototype.hasOwnProperty.call(GENERATOR_SEPARATORS, settings.separator)
    ? GENERATOR_SEPARATORS[settings.separator]
    : '-';
  const fullWords = settings.fullWords !== false;
  const capitalize = settings.capitalize === true;
  const words = fullWords ? pickWords(count) : Array.from({ length: count }, shortPasswordChunk);
  return words.map((word) => {
    if (!capitalize) return word;
    return word.charAt(0).toUpperCase() + word.slice(1);
  }).join(separator);
}

function defaultGeneratorSettings() {
  return {
    type: 'smart',
    useAsDefault: false,
    random: { length: 20, digits: true, symbols: true },
    memorable: { words: 4, separator: 'hyphen', capitalize: false, fullWords: true },
    pin: { length: 4 }
  };
}

function normalizeGeneratorSettings(raw) {
  const defaults = defaultGeneratorSettings();
  const source = raw && typeof raw === 'object' ? raw : {};
  const random = source.random && typeof source.random === 'object' ? source.random : {};
  const memorable = source.memorable && typeof source.memorable === 'object' ? source.memorable : {};
  const pin = source.pin && typeof source.pin === 'object' ? source.pin : {};
  const types = ['smart', 'random', 'memorable', 'pin'];
  return {
    type: types.indexOf(source.type) === -1 ? defaults.type : source.type,
    useAsDefault: source.useAsDefault === true,
    random: {
      length: clampNumber(random.length, 8, 64, defaults.random.length),
      digits: random.digits !== false,
      symbols: random.symbols !== false
    },
    memorable: {
      words: clampNumber(memorable.words, 3, 10, defaults.memorable.words),
      separator: Object.prototype.hasOwnProperty.call(GENERATOR_SEPARATORS, memorable.separator)
        ? memorable.separator
        : defaults.memorable.separator,
      capitalize: memorable.capitalize === true,
      fullWords: memorable.fullWords !== false
    },
    pin: {
      length: clampNumber(pin.length, 3, 12, defaults.pin.length)
    }
  };
}

function generateFromSettings(settings) {
  const normalized = normalizeGeneratorSettings(settings);
  if (normalized.type === 'pin') return generatePinPassword(normalized.pin);
  if (normalized.type === 'memorable') return generateMemorablePassword(normalized.memorable);
  if (normalized.type === 'random') return generateRandomPassword(normalized.random);
  return '';
}

function passwordStrength(password) {
  const value = String(password || '');
  if (!value) return 0;
  let pool = 0;
  if (/[a-z]/.test(value)) pool += 26;
  if (/[A-Z]/.test(value)) pool += 26;
  if (/\d/.test(value)) pool += 10;
  if (/[^a-zA-Z0-9]/.test(value)) pool += 32;
  if (!pool) pool = 10;
  const entropy = value.length * Math.log2(pool);
  return Math.max(0, Math.min(1, entropy / 80));
}

function strengthColor(strength) {
  const t = Math.max(0, Math.min(1, Number(strength) || 0));
  const stops = [
    { at: 0, rgb: [229, 72, 77] },
    { at: 0.5, rgb: [245, 165, 36] },
    { at: 1, rgb: [48, 164, 108] }
  ];
  let left = stops[0];
  let right = stops[stops.length - 1];
  for (let index = 0; index < stops.length - 1; index += 1) {
    if (t >= stops[index].at && t <= stops[index + 1].at) {
      left = stops[index];
      right = stops[index + 1];
      break;
    }
  }
  const span = right.at - left.at || 1;
  const local = (t - left.at) / span;
  const rgb = left.rgb.map((channel, index) => Math.round(channel + (right.rgb[index] - channel) * local));
  return 'rgb(' + rgb.join(', ') + ')';
}

function historyHost(url) {
  if (!url) return '';
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return '';
    return parsed.hostname;
  } catch (e) {
    return '';
  }
}

function formatHistoryDay(timestamp) {
  return new Date(timestamp).toLocaleDateString('en-US', {
    month: 'long',
    day: 'numeric',
    year: 'numeric'
  });
}

function formatHistoryTime(timestamp) {
  return new Date(timestamp).toLocaleTimeString('en-US', {
    hour: 'numeric',
    minute: '2-digit'
  });
}

function groupPasswordHistory(entries) {
  const groups = [];
  const index = new Map();
  (entries || []).forEach((entry) => {
    const label = formatHistoryDay(entry.createdAt);
    if (!index.has(label)) {
      const group = { label: label, entries: [] };
      index.set(label, group);
      groups.push(group);
    }
    index.get(label).entries.push(entry);
  });
  return groups;
}

function filterPasswordHistory(entries, query) {
  const needle = String(query || '').trim().toLowerCase();
  if (!needle) return (entries || []).slice();
  return (entries || []).filter((entry) => {
    const host = historyHost(entry.url).toLowerCase();
    const password = String(entry.password || '').toLowerCase();
    const day = formatHistoryDay(entry.createdAt).toLowerCase();
    const time = formatHistoryTime(entry.createdAt).toLowerCase();
    return host.indexOf(needle) !== -1
      || password.indexOf(needle) !== -1
      || day.indexOf(needle) !== -1
      || time.indexOf(needle) !== -1;
  });
}

if (typeof module !== 'undefined') {
  module.exports = {
    GENERATOR_SETTINGS_KEY,
    GENERATOR_SEPARATORS,
    defaultGeneratorSettings,
    normalizeGeneratorSettings,
    generateFromSettings,
    generateRandomPassword,
    generatePinPassword,
    generateMemorablePassword,
    passwordStrength,
    strengthColor,
    historyHost,
    formatHistoryDay,
    formatHistoryTime,
    groupPasswordHistory,
    filterPasswordHistory
  };
}
