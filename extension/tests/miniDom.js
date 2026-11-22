// A small HTML document for the node suites: parses real markup (elements, attributes, text,
// void elements, raw <script>/<style> text) into nodes with the DOM surface the shared scripts
// read. Not a browser: layout is a fixed box per element (override with data-rect="x,y,w,h"),
// computed style comes from the inline `style` attribute and `hidden`, and selectors are simple
// compound selectors (tag, #id, .class, [attr], [attr="v"], :disabled) with descendant
// combinators and comma lists. Test-only support code, not a suite (no `.test.js` suffix).

const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
const RAW = new Set(['script', 'style', 'textarea', 'title']);
const FORM_CONTROLS = new Set(['INPUT', 'SELECT', 'TEXTAREA', 'BUTTON']);

function decode(text) {
  return text.replace(/&(amp|lt|gt|quot|#39|nbsp);/g, (_, name) => ({ amp: '&', lt: '<', gt: '>', quot: '"', '#39': "'", nbsp: ' ' })[name]);
}

class Node {
  constructor(doc) {
    this.ownerDocument = doc;
    this.parentNode = null;
    this.childNodes = [];
  }
  get parentElement() {
    return this.parentNode && this.parentNode.nodeType === 1 ? this.parentNode : null;
  }
  get isConnected() {
    let node = this;
    while (node.parentNode) node = node.parentNode;
    return node === this.ownerDocument;
  }
  getRootNode() {
    let node = this;
    while (node.parentNode) node = node.parentNode;
    return node;
  }
  contains(other) {
    for (let node = other; node; node = node.parentNode) if (node === this) return true;
    return false;
  }
  appendChild(child) {
    if (child.parentNode) child.remove();
    child.parentNode = this;
    this.childNodes.push(child);
    return child;
  }
  remove() {
    if (!this.parentNode) return;
    const list = this.parentNode.childNodes;
    list.splice(list.indexOf(this), 1);
    this.parentNode = null;
  }
  get textContent() {
    return this.childNodes.map((child) => child.textContent).join('');
  }
  set textContent(value) {
    this.childNodes.forEach((child) => { child.parentNode = null; });
    this.childNodes = [];
    if (value) this.appendChild(new Text(this.ownerDocument, String(value)));
  }
}

class Text extends Node {
  constructor(doc, value) {
    super(doc);
    this.nodeType = 3;
    this.nodeValue = value;
  }
  get textContent() { return this.nodeValue; }
  set textContent(value) { this.nodeValue = String(value); }
}

class Element extends Node {
  constructor(doc, tag, attrs) {
    super(doc);
    this.nodeType = 1;
    this.tagName = tag.toUpperCase();
    this.attrs = new Map(Object.entries(attrs || {}));
    this.listeners = {};
    this.style = {
      props: {},
      setProperty(name, value) { this.props[name] = value; },
      getPropertyValue(name) { return this.props[name] || ''; },
    };
    this._value = null;
    if (this.tagName === 'INPUT' || this.tagName === 'TEXTAREA') this._value = null;
  }
  get children() { return this.childNodes.filter((child) => child.nodeType === 1); }
  get firstElementChild() { return this.children[0] || null; }
  get previousElementSibling() {
    if (!this.parentNode) return null;
    const siblings = this.parentNode.childNodes.filter((child) => child.nodeType === 1);
    return siblings[siblings.indexOf(this) - 1] || null;
  }
  get nextElementSibling() {
    if (!this.parentNode) return null;
    const siblings = this.parentNode.childNodes.filter((child) => child.nodeType === 1);
    return siblings[siblings.indexOf(this) + 1] || null;
  }
  getAttribute(name) { return this.attrs.has(name) ? this.attrs.get(name) : null; }
  hasAttribute(name) { return this.attrs.has(name); }
  setAttribute(name, value) { this.attrs.set(name, String(value)); }
  removeAttribute(name) { this.attrs.delete(name); }
  get id() { return this.getAttribute('id') || ''; }
  get name() { return this.getAttribute('name') || ''; }
  get className() { return this.getAttribute('class') || ''; }
  get placeholder() { return this.getAttribute('placeholder') || ''; }
  get type() {
    if (this.tagName === 'INPUT') return (this.getAttribute('type') || 'text').toLowerCase();
    if (this.tagName === 'BUTTON') return (this.getAttribute('type') || 'submit').toLowerCase();
    if (this.tagName === 'SELECT') return this.multiple ? 'select-multiple' : 'select-one';
    return '';
  }
  set type(value) { this.setAttribute('type', value); }
  get disabled() { return this.hasAttribute('disabled'); }
  set disabled(value) { if (value) this.setAttribute('disabled', ''); else this.removeAttribute('disabled'); }
  get readOnly() { return this.hasAttribute('readonly'); }
  get multiple() { return this.hasAttribute('multiple'); }
  get size() { return Number(this.getAttribute('size') || 0); }
  get hidden() { return this.hasAttribute('hidden'); }
  set hidden(value) { if (value) this.setAttribute('hidden', ''); else this.removeAttribute('hidden'); }
  get options() { return this.tagName === 'SELECT' ? this.querySelectorAll('option') : undefined; }
  get selected() {
    if (this._selected !== undefined) return this._selected;
    const select = this.closest('select');
    if (!select) return false;
    const options = select.options;
    const marked = options.filter((option) => option._selected === true || (option._selected === undefined && option.hasAttribute('selected')));
    if (marked.length) return marked[marked.length - 1] === this;
    return options[0] === this;
  }
  set selected(value) {
    const select = this.closest('select');
    if (value && select) for (const option of select.options) option._selected = false;
    this._selected = !!value;
  }
  get defaultSelected() { return this.hasAttribute('selected'); }
  get value() {
    if (this.tagName === 'SELECT') {
      const chosen = this.options.find((option) => option.selected);
      return chosen ? chosen.value : '';
    }
    if (this.tagName === 'OPTION') return this.hasAttribute('value') ? this.getAttribute('value') : this.textContent;
    if (this._value !== null) return this._value;
    if (this.tagName === 'TEXTAREA') return this.textContent;
    return this.getAttribute('value') || '';
  }
  set value(value) {
    if (this.tagName === 'SELECT') {
      for (const option of this.options) option._selected = option.value === String(value);
      return;
    }
    this._value = String(value);
  }
  get form() {
    if (!FORM_CONTROLS.has(this.tagName)) return undefined;
    const owner = this.getAttribute('form');
    if (owner) return this.ownerDocument.getElementById(owner);
    return this.closest('form');
  }
  get labels() {
    if (!['INPUT', 'SELECT', 'TEXTAREA'].includes(this.tagName)) return undefined;
    const found = [];
    const wrapping = this.parentElement && this.parentElement.closest('label');
    if (wrapping) found.push(wrapping);
    if (this.id) {
      for (const label of this.ownerDocument.querySelectorAll('label')) {
        if (label.getAttribute('for') === this.id && !found.includes(label)) found.push(label);
      }
    }
    return found;
  }
  matches(selector) {
    return parseSelector(selector).some((chain) => matchChain(this, chain));
  }
  closest(selector) {
    for (let node = this; node && node.nodeType === 1; node = node.parentNode) {
      if (node.matches(selector)) return node;
    }
    return null;
  }
  querySelectorAll(selector) {
    const chains = parseSelector(selector);
    const out = [];
    walk(this, (node) => { if (node !== this && chains.some((chain) => matchChain(node, chain))) out.push(node); });
    return out;
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  getBoundingClientRect() {
    const raw = this.getAttribute('data-rect');
    const [x, y, width, height] = raw ? raw.split(',').map(Number) : [0, 0, 100, 30];
    return { x, y, left: x, top: y, width, height, right: x + width, bottom: y + height };
  }
  addEventListener(type, fn) { (this.listeners[type] = this.listeners[type] || []).push(fn); }
  removeEventListener(type, fn) { this.listeners[type] = (this.listeners[type] || []).filter((f) => f !== fn); }
  dispatchEvent(event) {
    const path = [];
    for (let node = this; node; node = node.parentNode) path.push(node);
    event.target = event.target || this;
    for (const node of path) {
      for (const fn of (node.listeners && node.listeners[event.type]) || []) fn(event);
      if (!event.bubbles) break;
    }
    return !event.defaultPrevented;
  }
  focus() { this.ownerDocument.activeElement = this; }
  // A shadow root is a detached element, so its text is not part of the host's or the page's
  // textContent. `shadowRoot` is set for a closed root too, so tests can look inside.
  attachShadow() {
    const root = new Element(this.ownerDocument, '#shadow-root');
    root.host = this;
    this.shadowRoot = root;
    return root;
  }
}

class Document extends Node {
  constructor() {
    super(null);
    this.ownerDocument = this;
    this.nodeType = 9;
    this.activeElement = null;
    this.listeners = {};
    const doc = this;
    this.defaultView = {
      innerWidth: 1280,
      innerHeight: 800,
      getComputedStyle: (node) => computedStyle(node),
      Event: class {
        constructor(type, init) { this.type = type; this.bubbles = !!(init && init.bubbles); this.defaultPrevented = false; }
        preventDefault() { this.defaultPrevented = true; }
      },
      HTMLInputElement: { prototype: {} },
      HTMLTextAreaElement: { prototype: {} },
      HTMLSelectElement: { prototype: {} },
      document: doc,
    };
  }
  get documentElement() { return this.children[0] || null; }
  get children() { return this.childNodes.filter((child) => child.nodeType === 1); }
  get body() { return this.documentElement ? this.documentElement.querySelector('body') : null; }
  get head() { return this.documentElement ? this.documentElement.querySelector('head') : null; }
  createElement(tag) { return new Element(this, tag); }
  createTextNode(text) { return new Text(this, text); }
  getElementById(id) { return this.querySelectorAll('[id]').find((node) => node.id === id) || null; }
  querySelectorAll(selector) { return this.documentElement ? [this.documentElement, ...this.documentElement.querySelectorAll('*')].filter((node) => node.matches(selector)) : []; }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  addEventListener(type, fn) { (this.listeners[type] = this.listeners[type] || []).push(fn); }
}

function walk(root, visit) {
  for (const child of root.childNodes) {
    if (child.nodeType !== 1) continue;
    visit(child);
    walk(child, visit);
  }
}

function inlineStyle(node) {
  const style = {};
  for (const part of String(node.getAttribute('style') || '').split(';')) {
    const [name, value] = part.split(':').map((s) => s && s.trim());
    if (name && value) style[name] = value;
  }
  return style;
}

function computedStyle(node) {
  const own = inlineStyle(node);
  let visibility = own.visibility;
  for (let parent = node.parentElement; !visibility && parent; parent = parent.parentElement) {
    visibility = inlineStyle(parent).visibility;
  }
  return {
    display: node.hasAttribute('hidden') ? 'none' : own.display || 'block',
    visibility: visibility || 'visible',
    opacity: own.opacity || '1',
    position: own.position || 'static',
  };
}

// `a b, c[x="y"]` -> [[{tag, attrs, ...}, ...], ...]: comma lists of descendant chains.
function parseSelector(selector) {
  return selector.split(',').map((group) => group.trim().split(/\s+/).map((part) => {
    const simple = { tag: null, id: null, classes: [], attrs: [], disabled: false };
    const re = /^([a-zA-Z*][\w-]*)|#([\w-]+)|\.([\w-]+)|\[([\w-]+)(?:([\^*$]?=)"([^"]*)")?\]|:(disabled)/g;
    let match;
    let consumed = 0;
    while ((match = re.exec(part)) && match.index === consumed) {
      consumed = re.lastIndex;
      if (match[1]) simple.tag = match[1] === '*' ? null : match[1].toUpperCase();
      else if (match[2]) simple.id = match[2];
      else if (match[3]) simple.classes.push(match[3]);
      else if (match[4]) simple.attrs.push({ name: match[4], op: match[5] || null, value: match[6] });
      else if (match[7]) simple.disabled = true;
    }
    if (consumed !== part.length) throw new Error('miniDom: unsupported selector ' + selector);
    return simple;
  }));
}

function matchSimple(node, simple) {
  if (!node || node.nodeType !== 1) return false;
  if (simple.tag && node.tagName !== simple.tag) return false;
  if (simple.id && node.id !== simple.id) return false;
  const classes = node.className.split(/\s+/);
  if (simple.classes.some((name) => !classes.includes(name))) return false;
  for (const { name, op, value } of simple.attrs) {
    if (!node.hasAttribute(name)) return false;
    const actual = node.getAttribute(name);
    if (op === '=' && actual !== value) return false;
    if (op === '^=' && !actual.startsWith(value)) return false;
    if (op === '*=' && !actual.includes(value)) return false;
    if (op === '$=' && !actual.endsWith(value)) return false;
  }
  if (simple.disabled) {
    const control = FORM_CONTROLS.has(node.tagName) || ['OPTION', 'FIELDSET'].includes(node.tagName);
    if (!control) return false;
    if (!node.disabled && !(node.parentElement && node.parentElement.closest('fieldset[disabled]'))) return false;
  }
  return true;
}

function matchChain(node, chain) {
  if (!matchSimple(node, chain[chain.length - 1])) return false;
  let index = chain.length - 2;
  for (let parent = node.parentElement; index >= 0 && parent; parent = parent.parentElement) {
    if (matchSimple(parent, chain[index])) index -= 1;
  }
  return index < 0;
}

// Parses `html` into a Document. A fragment without <html>/<body> is wrapped in both.
function parseHtml(html) {
  const doc = new Document();
  const source = /<html[\s>]/i.test(html) ? html : '<html><head></head><body>' + html + '</body></html>';
  const stack = [doc];
  const tagRe = /<!--[\s\S]*?-->|<!doctype[^>]*>|<\/([a-zA-Z][\w-]*)\s*>|<([a-zA-Z][\w-]*)((?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)*)\s*(\/?)>/gi;
  let last = 0;
  let match;
  const top = () => stack[stack.length - 1];
  const addText = (text) => {
    if (text && !(top() === doc)) top().appendChild(new Text(doc, decode(text)));
  };
  while ((match = tagRe.exec(source))) {
    addText(source.slice(last, match.index));
    last = tagRe.lastIndex;
    if (match[1]) {
      const tag = match[1].toUpperCase();
      for (let i = stack.length - 1; i > 0; i -= 1) {
        if (stack[i].tagName === tag) { stack.length = i; break; }
      }
      continue;
    }
    if (!match[2]) continue;
    const attrs = {};
    const attrRe = /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
    let attr;
    while ((attr = attrRe.exec(match[3] || ''))) {
      attrs[attr[1].toLowerCase()] = decode(attr[2] ?? attr[3] ?? attr[4] ?? '');
    }
    const element = new Element(doc, match[2], attrs);
    top().appendChild(element);
    const lower = match[2].toLowerCase();
    if (VOID.has(lower) || match[4]) continue;
    if (RAW.has(lower)) {
      const end = source.toLowerCase().indexOf('</' + lower, last);
      const stop = end === -1 ? source.length : end;
      if (stop > last) element.appendChild(new Text(doc, lower === 'textarea' || lower === 'title' ? decode(source.slice(last, stop)) : source.slice(last, stop)));
      const close = source.indexOf('>', stop);
      last = close === -1 ? source.length : close + 1;
      tagRe.lastIndex = last;
      continue;
    }
    stack.push(element);
  }
  addText(source.slice(last));
  return doc;
}

// Writes like the platform value setter does, then fires input and change (bubbling).
function typeInto(node, value) {
  node.value = value;
  const view = node.ownerDocument.defaultView;
  node.dispatchEvent(new view.Event('input', { bubbles: true }));
  node.dispatchEvent(new view.Event('change', { bubbles: true }));
}

module.exports = { parseHtml, typeInto };
