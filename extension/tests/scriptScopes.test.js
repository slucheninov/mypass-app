// popup.html loads shared/*.js and then popup.js as SEPARATE classic <script> tags, with
// no module/type="module" wrapper. Classic scripts in the same document share ONE global
// scope (one GlobalEnvironmentRecord): every top-level `function`, `var`, `const`, or
// `let` in an earlier script lands in that shared scope before the next script's own
// top-level declarations are processed.
//
// entry-visuals.js declares `letterFromTitle` and `colorForTitle` as top-level function
// declarations. If popup.js ALSO declares a top-level lexical binding (`const`/`let`, including
// via destructuring) with either of those names, the browser's GlobalDeclarationInstantiation
// step for popup.js throws a SyntaxError — "Identifier 'X' has already been declared" — before
// a single statement of popup.js runs. This is exactly what shipped: popup.js used
// `const { letterFromTitle, colorForTitle } = ...` at top level, which collided with the
// function declarations entry-visuals.js had already put in the shared global scope, and the
// entire popup went dead in the browser. The same rule applies to every other shared script
// popup.html loads, and to the content-script list in manifest.json: those files share one
// isolated-world global, and content.js must not redeclare their names at column 0.
//
// No node test could catch this directly: under `require`, popup.js and entry-visuals.js are
// each loaded as their own CommonJS module with a private module scope, so the `require`
// branch of popup.js's shim is taken and there is no global-scope collision to observe.
// `vm.runInContext` doesn't reproduce it either — a fresh context still permits a lexical
// declaration to shadow a `var`/function-declared global rather than throwing, because that
// collision is a load-time GlobalDeclarationInstantiation check across the SPECIFIC "two
// classic scripts in one document" sequence, not a generic runtime property lookup.
//
// So this test works statically instead: it parses the top-level declared names out of both
// files' SOURCE TEXT (function/const/let/var declarations starting at column 0 — i.e. actually
// top-level, not nested in a block or IIFE) and asserts the two name sets never intersect. If
// they ever do, popup.js will throw exactly the SyntaxError this bug shipped with, the moment
// popup.html loads it after entry-visuals.js.

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

function topLevelDeclaredNames(source) {
  const names = new Set();

  // Top-level `function NAME(...)` and `function* NAME(...)` declarations (column 0).
  const fnRe = /^function\*?\s+([A-Za-z_$][\w$]*)/gm;
  let m;
  while ((m = fnRe.exec(source))) names.add(m[1]);

  // Top-level `const`/`let`/`var` declarations (column 0), including destructuring.
  // Matches the declarator list start; handles simple names and `{ a, b: c, ...rest }` /
  // `[a, b]` patterns across the (possibly multi-line) binding target.
  const lexRe = /^(?:const|let|var)\s+([\s\S]*?)(?:=|;|$)/gm;
  while ((m = lexRe.exec(source))) {
    const target = m[1];
    if (/^[A-Za-z_$][\w$]*\s*$/.test(target)) {
      // Simple `const NAME = ...` / `const NAME;`
      names.add(target.trim());
      continue;
    }
    if (/^[{[]/.test(target.trim())) {
      // Destructuring: pull out bound identifiers, ignoring object keys before ':'.
      // `{ letterFromTitle, colorForTitle }` -> both names.
      // `{ letterFromTitle: lft }` -> only `lft` (the actual binding).
      const bindingRe = /(?:^|[,{[]\s*)(?:\.\.\.)?([A-Za-z_$][\w$]*)\s*(?::\s*([A-Za-z_$][\w$]*))?/g;
      let bm;
      while ((bm = bindingRe.exec(target))) {
        const bound = bm[2] || bm[1];
        if (bound) names.add(bound);
      }
    }
  }

  return names;
}

function readNames(relPath) {
  const source = fs.readFileSync(path.join(__dirname, '..', relPath), 'utf8');
  return topLevelDeclaredNames(source);
}

function assertDisjoint(groupLabel, relPaths) {
  const owner = new Map();
  for (const relPath of relPaths) {
    for (const name of readNames(relPath)) {
      const previous = owner.get(name);
      assert.strictEqual(
        previous,
        undefined,
        `${groupLabel}: top-level name '${name}' is declared in both ${previous} and ${relPath}. ` +
          `Those files are classic scripts in one global scope, so the second declaration is a ` +
          `load-time SyntaxError ("Identifier already been declared") and the page never runs. ` +
          `Bind through a namespace object instead of redeclaring the name.`
      );
      owner.set(name, relPath);
    }
  }
  return owner;
}

const popupHtml = fs.readFileSync(path.join(__dirname, '..', 'popup.html'), 'utf8');
const popupScripts = [];
const scriptRe = /<script\s+src="([^"]+)"><\/script>/g;
let scriptMatch;
while ((scriptMatch = scriptRe.exec(popupHtml))) popupScripts.push(scriptMatch[1]);
assert.ok(popupScripts.includes('popup.js'), 'sanity: popup.html loads popup.js');
assert.ok(
  popupScripts.indexOf('shared/entry-visuals.js') < popupScripts.indexOf('popup.js'),
  'entry-visuals.js must load before popup.js'
);

const popupGroup = assertDisjoint('popup.html', popupScripts);
assert.ok(popupGroup.has('letterFromTitle'), 'sanity: parser sees entry-visuals.js top-level function letterFromTitle');
assert.ok(popupGroup.has('colorForTitle'), 'sanity: parser sees entry-visuals.js top-level function colorForTitle');
assert.ok(popupGroup.get('classifyGetCredentialsResponse') === 'shared/bridge-status.js');
assert.ok(popupGroup.get('isBase32Secret') === 'shared/base32.js');

const manifest = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'manifest.json'), 'utf8'));
const contentScripts = manifest.content_scripts.find((entry) => entry.js.includes('content.js'));
assert.ok(contentScripts, 'manifest content script entry that loads content.js');
const contentGroup = assertDisjoint('content_scripts', contentScripts.js);
assert.strictEqual(contentGroup.get('hostnameMatches'), 'shared/url-match.js');
assert.strictEqual(contentGroup.get('findFillableFields'), 'shared/form-detection.js');
assert.strictEqual(contentGroup.get('detectManualSetupKey'), 'shared/totp-detection.js');
assert.ok(![...contentGroup.values()].includes('content.js'), 'content.js must not add top-level names; it is an IIFE after the shared scripts');

// background-loader.js importScripts the shared helpers and then background.js
// into one worker global, so their top-level names must not overlap.
assertDisjoint('service worker', [
  'shared/url-match.js',
  'shared/password-generator.js',
  'shared/security-watch.js',
  'shared/notification-settings.js',
  'shared/autofill-settings.js',
  'shared/prompt-record.js',
  'background.js'
]);

function scriptSources(html) {
  const sources = [];
  const re = /<script\s+src="([^"]+)"><\/script>/g;
  let match;
  while ((match = re.exec(html))) sources.push(match[1]);
  return sources;
}

const createScripts = scriptSources(fs.readFileSync(path.join(__dirname, '..', 'create.html'), 'utf8'));
assert.ok(createScripts.indexOf('shared/entry-visuals.js') < createScripts.indexOf('create.js'));
assertDisjoint('create.html', createScripts);

const generatorScripts = scriptSources(fs.readFileSync(path.join(__dirname, '..', 'generator.html'), 'utf8'));
assert.ok(generatorScripts.indexOf('shared/password-generator.js') < generatorScripts.indexOf('generator.js'));
assertDisjoint('generator.html', generatorScripts);

const promptScripts = scriptSources(fs.readFileSync(path.join(__dirname, '..', 'prompt.html'), 'utf8'));
assert.ok(promptScripts.indexOf('shared/prompt-card.js') < promptScripts.indexOf('prompt.js'), 'the card loads before the page script');
assert.ok(!promptScripts.includes('popup.js'), 'the prompt window is its own page, not popup.html');
assertDisjoint('prompt.html', promptScripts);

console.log('scriptScopes.test.js: no global scope collisions OK');
