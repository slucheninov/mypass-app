const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const manifestPath = path.join(__dirname, '..', 'manifest.json');
const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));

assert.strictEqual(manifest.manifest_version, 3, 'manifest_version must be 3');
assert.strictEqual(manifest.name, 'MyPass');
assert.strictEqual(
  manifest.key,
  'MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA3g4jQjrf7R0zDFOQTwQGRpzY/Zl1iOChwlpfDJav6F6e5GiRJPy7S9/cC8yFvhJZsVLs49hQgLg+HqL206KziXRnSTFT1+SbxFzIGqwGjW1Q9u0koXFEzpE82kPQlxG6zeFqtjm35cPfhkCu6oqGFtImLBzP9JR2s4n5IMEQabVvKH2KxsKF84RK7+3MkBLJ/0bluRxXv+VpeG20QN6rQr/VTmC0B5YoIChLdM0ObWhBjzJl8VA64VCAYcaPfLVQ4ZSkLdrIFfNIi5Sc8/8Mtil8wJCON+W/ilWZnYakMvV5Y83fafu8lTHCeuNzbxGAObJFWeIoeaILkoiD1PCuRQIDAQAB',
  'pinned key must match unpacked extension ID hjijbabodfldconmhdcdphlbgaajkejn'
);
assert.ok(Array.isArray(manifest.permissions), 'permissions must be an array');
assert.ok(manifest.permissions.includes('nativeMessaging'), 'missing nativeMessaging permission');
assert.ok(manifest.permissions.includes('activeTab'), 'missing activeTab permission');
assert.ok(manifest.permissions.includes('webAuthenticationProxy'), 'missing webAuthenticationProxy permission');
assert.ok(
  manifest.permissions.includes('alarms'),
  'the passkey ceremony watchdog runs on chrome.alarms — a setTimeout does not survive service-worker termination'
);
assert.strictEqual(
  manifest.minimum_chrome_version,
  '127',
  'webAuthenticationProxy needs 115, but chrome.action.openPopup() is policy-installed-only on 118-126 and the confirm popup is not optional'
);
assert.ok(
  !manifest.permissions.includes('scripting'),
  'scripting permission is unused (content_scripts already covers injection) and should not be requested'
);
assert.strictEqual(manifest.background.service_worker, 'background-loader.js');
const loaderSource = fs.readFileSync(path.join(__dirname, '..', 'background-loader.js'), 'utf8');
const loaderCode = loaderSource.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '').trim();
assert.ok(
  loaderCode.startsWith("importScripts('shared/url-match.js')"),
  'url-match.js must be importScripts first, before background.js does any other work'
);
assert.ok(
  loaderCode.indexOf("importScripts('shared/url-match.js')") < loaderCode.indexOf("importScripts('shared/password-generator.js')")
    && loaderCode.indexOf("importScripts('shared/password-generator.js')") < loaderCode.indexOf("importScripts('background.js')"),
  'url-match.js and password-generator.js must exist before background.js runs'
);
const backgroundSource = fs.readFileSync(path.join(__dirname, '..', 'background.js'), 'utf8');
assert.ok(
  !backgroundSource.includes('importScripts('),
  'background.js must not call importScripts itself: Chrome rejects that call once other startup code has run, and the message listener never registers'
);
assert.strictEqual(manifest.action.default_popup, 'popup.html');
const contentScripts = manifest.content_scripts;
assert.strictEqual(contentScripts.length, 2, 'two entries: the QR decoder stays top-frame, the rest runs everywhere');

const [qrEntry, mainEntry] = contentScripts;

assert.deepStrictEqual(qrEntry.js, ['vendor/jsQR.js'], 'the 251 KB QR decoder is its own entry');
assert.ok(!qrEntry.all_frames, 'the QR decoder must NOT be injected into subframes — it is 82% of the payload and useless there');

assert.deepStrictEqual(
  mainEntry.js,
  [
    'shared/entry-visuals.js',
    'shared/url-match.js',
    'shared/bridge-status.js',
    'shared/base32.js',
    'shared/form-detection.js',
    'shared/page-identifier.js',
    'shared/security-watch.js',
    'shared/autofill-settings.js',
    'shared/address-form.js',
    'shared/totp-detection.js',
    'shared/placement.js',
    'content.js',
  ],
  'shared scripts load before content.js, which reads them'
);
assert.ok(
  mainEntry.js.indexOf('shared/base32.js') < mainEntry.js.indexOf('shared/totp-detection.js'),
  'totp-detection.js calls isBase32Secret from the base32.js global'
);
assert.strictEqual(mainEntry.all_frames, true, 'the icon must reach login forms inside iframes');
assert.ok(!mainEntry.js.includes('vendor/jsQR.js'), 'jsQR must not sneak back into the all-frames entry');
console.log('background.manifest.test.js: content script frame split OK');

const iconsDir = path.join(__dirname, '..', 'icons');
const expectedSizes = ['16', '32', '48', '128'];
assert.ok(manifest.icons, 'manifest must define top-level icons');
assert.ok(manifest.action.default_icon, 'manifest.action must define default_icon');
for (const size of expectedSizes) {
  const relPath = `icons/icon${size}.png`;
  assert.strictEqual(manifest.icons[size], relPath, `manifest.icons["${size}"] must be ${relPath}`);
  assert.strictEqual(manifest.action.default_icon[size], relPath, `action.default_icon["${size}"] must be ${relPath}`);
  const absPath = path.join(__dirname, '..', relPath);
  assert.ok(fs.existsSync(absPath), `${relPath} must exist on disk`);
  const stat = fs.statSync(absPath);
  assert.ok(stat.size > 0, `${relPath} must be non-empty`);
}

console.log('manifest.json OK');

assert.ok(manifest.permissions.includes('notifications'), 'passkey popup fallback requires notifications permission');
assert.deepStrictEqual(manifest.optional_permissions, ['privacy', 'management'], 'manager inspection remains an explicit opt-in');
