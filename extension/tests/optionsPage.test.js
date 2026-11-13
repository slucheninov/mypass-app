const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8'));

assert.deepStrictEqual(manifest.options_ui, { page: 'app/app.html', open_in_tab: true });
assert.strictEqual(manifest.options_page, undefined, 'one options mechanism only');
// The standard Options entry is used. Chrome opens it without a hash, so the
// page itself normalises to #/page/settings.
assert.ok(!manifest.permissions.includes('contextMenus'), 'contextMenus must not be requested');

const router = require('../app/router.js');
assert.strictEqual(router.DEFAULT_ROUTE, '/page/settings');
for (const hash of ['', '#', '#/', '#/page/settings']) {
  assert.deepStrictEqual(router.parseRoute(hash), { name: 'settings' }, JSON.stringify(hash));
  assert.strictEqual(router.canonicalHash(hash), '#/page/settings', JSON.stringify(hash));
}
assert.deepStrictEqual(router.parseRoute('#/page/nope'), { name: 'notFound' });

const htmlPath = path.join(root, 'app', 'app.html');
assert.ok(fs.existsSync(htmlPath), 'app/app.html must exist');
const html = fs.readFileSync(htmlPath, 'utf8');
assert.ok(/<script[^>]+src="router\.js"/.test(html), 'router.js script tag');
assert.ok(/<script[^>]+src="app\.js"/.test(html), 'app.js script tag');
// Guard: every <script> must be external (has src) and carry no body.
// Tag detection is case-insensitive and tolerant of attributes and of
// any junk before ">" in the closing tag (</script >, </script\t\n bar>),
// which browsers accept.
function inlineScripts(source) {
  const bad = [];
  const re = /<script(?=[\s/>])[^>]*>([\s\S]*?)<\/script(?=[\s/>])[^>]*>/gi;
  let m;
  while ((m = re.exec(source)) !== null) {
    const open = m[0].slice(0, m[0].indexOf('>') + 1);
    if (!/\bsrc\s*=/i.test(open) || m[1].trim() !== '') bad.push(m[0]);
  }
  return bad;
}
assert.deepStrictEqual(inlineScripts(html), [], 'inline script forbidden by MV3 CSP');
// Self-checks: the guard must fail on bad input.
assert.strictEqual(inlineScripts('<SCRIPT>alert(1)</SCRIPT >').length, 1, 'upper-case inline script detected');
assert.strictEqual(inlineScripts('<script>alert(1)</script\n>').length, 1, 'whitespace before > detected');
assert.strictEqual(inlineScripts('<script>x</script\t\n bar>').length, 1, 'end tag with trailing junk detected');
// router.js's element is never closed by </scriptx>, so it swallows the app.js tag as body text.
assert.ok(inlineScripts('<script src="router.js"></scriptx><script src="app.js"></script>').length >= 1, '</scriptx> is not an end tag, so the next tag is swallowed and flagged');
assert.strictEqual(inlineScripts('<scriptx>y</scriptx>').length, 0, '<scriptx> is not a script tag');
assert.strictEqual(inlineScripts('<script src="a.js">alert(1)</script>').length, 1, 'src with body detected');
assert.strictEqual(inlineScripts('<ScRiPt src="a.js"></SCRIPT >').length, 0, 'external script accepted');
assert.ok(html.includes('id="page-settings"'), 'settings pane container');
console.log('optionsPage tests passed');
