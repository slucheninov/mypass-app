"""Offline release CLI regressions; all mutations are in disposable repository copies."""
import hashlib
import importlib.util
import plistlib
from unittest.mock import patch
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import sys
sys.dont_write_bytecode = True
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
BASH = 'bash'
if os.name == 'nt':
    system = os.environ.get('SystemRoot', 'C:\\Windows').lower()
    BASH = next(str(Path(p) / 'bash.exe') for p in os.environ['PATH'].split(os.pathsep)
                if not p.lower().startswith(system) and (Path(p) / 'bash.exe').is_file())

class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in ('scripts', 'src-tauri', 'extension'):
            shutil.copytree(ROOT / name, self.root / name,
                            ignore=shutil.ignore_patterns('binaries', 'gen', 'icons', '__pycache__'))
        for name in ('Cargo.toml', 'Cargo.lock', 'package.json'):
            shutil.copy2(ROOT / name, self.root / name)
        self.version = tomllib.loads((self.root / 'src-tauri/Cargo.toml').read_text())['package']['version']
        major, minor, patch_version = map(int, self.version.split('.'))
        self.next_version = f'{major}.{minor}.{patch_version + 1}'
        bin_dir = self.root / 'stub-bin'
        bin_dir.mkdir()
        cargo = bin_dir / 'cargo'
        cargo.write_text('#!/usr/bin/env bash\necho "$*" >> "$CARGO_TEST_LOG"\nexit "${CARGO_TEST_EXIT:-0}"\n', newline="\n")
        cargo.chmod(0o755)
        (bin_dir / 'cargo.cmd').write_text('@echo %*>>"%CARGO_TEST_LOG%"\n@exit /b %CARGO_TEST_EXIT%\n')
        self.env = dict(os.environ)
        self.env['CARGO_TEST_EXIT'] = '0'
        self.env['CARGO_TEST_LOG'] = str(self.root / 'cargo.log')
        self.env['PATH'] = str(bin_dir) + os.pathsep + self.env['PATH']
        for key in ('MYPASS_SIGNING_IDENTITY', 'APPLE_SIGNING_IDENTITY'):
            self.env.pop(key, None)

    def run_script(self, script, *args, ok=True):
        p = subprocess.run([BASH, 'scripts/' + script, *map(str, args)],
                           cwd=self.root, env=self.env, capture_output=True, text=True)
        self.assertEqual(p.returncode == 0, ok, p.stdout + p.stderr)
        return p.stdout + p.stderr

class VersionTests(Fixture):
    def test_check_version_names_disagreeing_file_and_tag(self):
        self.run_script('check-version.sh', self.version)
        self.run_script('check-version.sh', self.next_version, ok=False)
        path = self.root / 'extension/manifest.json'
        data = json.loads(path.read_text()); data['version'] = self.next_version
        path.write_text(json.dumps(data))
        self.assertIn('extension/manifest.json', self.run_script('check-version.sh', ok=False))

    def test_bump_updates_versions_and_lock_offline(self):
        self.run_script('bump-version.sh', self.next_version)
        self.run_script('check-version.sh', self.next_version)
        self.assertEqual((self.root / 'cargo.log').read_text().splitlines(), ['update -w --offline'])
        lock = (self.root / 'Cargo.lock').read_text()
        self.assertIn(f'name = "mypass-app"\nversion = "{self.next_version}"', lock)
        for version in (self.next_version, self.version, 'bad', '1.2', '01.2.3'):
            self.run_script('bump-version.sh', version, ok=False)

    def test_bump_uses_the_resolved_cargo_launcher(self):
        spec = importlib.util.spec_from_file_location('release', ROOT / 'scripts/release.py')
        release = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(release)
        # Native Windows process lookup ignores cargo.cmd for a bare "cargo" command.
        # Use an explicit launcher path (including spaces) so this also regresses on Unix.
        launcher = self.root / 'stub-bin' / ('cargo.cmd' if os.name == 'nt' else 'cargo')
        selected = launcher.with_name('selected cargo' + launcher.suffix)
        launcher.rename(selected)
        with patch.object(release, 'ROOT', self.root), patch.dict(os.environ, self.env, clear=True), \
                patch.object(release.shutil, 'which', return_value=str(selected)) as resolve:
            release.bump(self.next_version)
            self.assertEqual(release.check_version(), self.next_version)
        resolve.assert_called_once_with('cargo')

    def test_bump_rolls_back_when_cargo_fails(self):
        paths = ['src-tauri/Cargo.toml', 'Cargo.lock', 'package.json', 'extension/manifest.json']
        before = {p: (self.root / p).read_bytes() for p in paths}
        self.env['CARGO_TEST_EXIT'] = '1'
        self.run_script('bump-version.sh', self.next_version, ok=False)
        self.assertEqual((self.root / 'cargo.log').read_text().splitlines(),
                         ['update -w --offline', 'check -p mypass-app --offline'])
        self.assertEqual(before, {p: (self.root / p).read_bytes() for p in paths})

    def test_bump_atomic_when_a_file_is_unwritable(self):
        paths = ['src-tauri/Cargo.toml', 'Cargo.lock', 'package.json', 'extension/manifest.json']
        before = {p: (self.root / p).read_bytes() for p in paths}
        (self.root / 'extension/manifest.json').chmod(0o444)
        self.run_script('bump-version.sh', self.next_version, ok=False)
        self.assertEqual(before, {p: (self.root / p).read_bytes() for p in paths})

class PackageTests(Fixture):
    def test_dev_app_unsigned_branch_explicitly_disables_tauri_signing(self):
        text = (self.root / 'scripts/dev-app.sh').read_text()
        unsigned = next(line for line in text.splitlines() if 'env -u APPLE_SIGNING_IDENTITY pnpm tauri build' in line)
        self.assertIn('--no-sign', unsigned)

    def test_host_rejects_a_certificate_name_before_building(self):
        self.env['MYPASS_SIGNING_IDENTITY'] = 'Apple Development: ambiguous name'
        message = self.run_script('build-host.sh', '--release', '--target', 'aarch64-apple-darwin', ok=False)
        self.assertIn('40-hex', message)
        self.assertNotIn('Traceback', message)

    def test_release_notes_substitute_every_version_placeholder(self):
        out = self.root / 'notes.md'
        subprocess.run([sys.executable, str(self.root / 'scripts/release.py'), 'notes', '1.2.3', str(out)], check=True)
        self.assertIn('MyPass 1.2.3', out.read_text())
        self.assertNotIn('{{VERSION}}', out.read_text())

    def test_artifact_names_and_exact_relative_checksums(self):
        names = self.run_script('release-assets.sh', '1.2.3').splitlines()
        self.assertEqual(names, ['MyPass-1.2.3-macos-arm64.dmg', 'MyPass-chrome-extension-1.2.3.zip', 'SHA256SUMS.txt'])
        self.assertEqual(len(set(names)), 3)

    def test_extension_zip_reproducible_and_staged_without_development_files(self):
        out = self.root / 'out'
        self.run_script('package-extension.sh', '--out', out)
        path = out / f'MyPass-chrome-extension-{self.version}.zip'
        first = hashlib.sha256(path.read_bytes()).digest()
        with zipfile.ZipFile(path) as archive:
            names = archive.namelist()
            self.assertIn('manifest.json', names)
            self.assertTrue(all('tests' not in Path(n).parts and not n.endswith('.test.js') and Path(n).name not in ('package.json', '.DS_Store') for n in names))
        stage = self.root / 'stage'
        self.run_script('package-extension.sh', '--out', out, '--stage-dir', stage)
        self.assertEqual(first, hashlib.sha256(path.read_bytes()).digest())
        self.assertTrue((stage / 'manifest.json').is_file())
        manifest = self.root / 'extension/manifest.json'
        manifest.write_text(manifest.read_text().replace(self.version, self.next_version))
        self.run_script('package-extension.sh', '--out', out, ok=False)

    def test_release_refuses_no_identity_and_dry_run_never_signs(self):
        self.assertIn('identity', self.run_script('package-macos.sh', '--release', '--dry-run', ok=False))
        self.assertIn('UNSIGNED development build', self.run_script('package-macos.sh', '--allow-unsigned', '--dry-run'))
        self.run_script('package-macos.sh', '--dry-run', ok=False)
        for identity in ('-', 'Apple Development: name', ''):
            self.env['MYPASS_SIGNING_IDENTITY'] = identity
            self.run_script('package-macos.sh', '--dry-run', ok=False)
        self.env['MYPASS_SIGNING_IDENTITY'] = 'ABCDEF0123456789ABCDEF0123456789ABCDEF01'
        self.assertIn(self.env['MYPASS_SIGNING_IDENTITY'], self.run_script('package-macos.sh', '--dry-run'))

    def test_build_host_passes_identity_hash_to_codesign_unchanged(self):
        target = self.root / 'target'
        built = target / 'aarch64-apple-darwin/release/mypass-host'
        built.parent.mkdir(parents=True); built.write_bytes(b'cargo output')
        metadata = self.root / 'metadata.json'
        metadata.write_text(json.dumps({'target_directory': str(target)}))
        self.env['CARGO_METADATA_JSON'] = str(metadata)
        self.env['CODESIGN_LOG'] = str(self.root / 'codesign.log')
        self.env['MYPASS_SIGNING_IDENTITY'] = 'AbCdEf0123456789AbCdEf0123456789AbCdEf01'
        cargo = self.root / 'stub-bin/cargo'
        cargo.write_text('#!/usr/bin/env bash\nif [[ "$1" == metadata ]]; then cat "$CARGO_METADATA_JSON"; fi\n', newline="\n")
        codesign = self.root / 'stub-bin/codesign'
        codesign.write_text('#!/usr/bin/env bash\nprintf "%s\\n" "$@" > "$CODESIGN_LOG"\n', newline="\n")
        codesign.chmod(0o755)
        self.run_script('build-host.sh', '--release', '--target', 'aarch64-apple-darwin')
        args = (self.root / 'codesign.log').read_text().splitlines()
        self.assertEqual(args[args.index('--sign') + 1], self.env['MYPASS_SIGNING_IDENTITY'])
        self.assertTrue(any('certificate leaf[subject.OU] = "W645B93PPQ"' in a for a in args))

    def test_release_requires_clean_exact_tag(self):
        for args in (['init', '-q'], ['add', '.'], ['-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'fixture']):
            subprocess.run(['git', *args], cwd=self.root, check=True, capture_output=True)
        self.env['MYPASS_SIGNING_IDENTITY'] = 'A' * 40
        self.run_script('package-macos.sh', '--release', '--dry-run', ok=False)
        subprocess.run(['git', 'tag', f'v{self.version}'], cwd=self.root, check=True)
        self.run_script('package-macos.sh', '--release', '--dry-run')
        (self.root / 'package.json').write_text('{}')
        self.run_script('package-macos.sh', '--release', '--dry-run', ok=False)

class VerifyTests(Fixture):
    def setUp(self):
        super().setUp()
        spec = importlib.util.spec_from_file_location('release', ROOT / 'scripts/release.py')
        self.release = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.release)
        self.app = self.root / 'MyPass.app'
        self.resources = self.app / 'Contents/Resources'
        self.resources.mkdir(parents=True)
        info = {'CFBundleIdentifier': 'com.mypass.app', 'CFBundleShortVersionString': self.version,
                'LSMinimumSystemVersion': '14.0', 'CFBundleDevelopmentRegion': 'en',
                'CFBundleLocalizations': ['en', 'uk', 'ru']}
        (self.app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
        for lang in ('en', 'uk', 'ru'):
            path = self.resources / f'{lang}.lproj/InfoPlist.strings'
            path.parent.mkdir(); path.write_text('strings')
        for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md'):
            (self.resources / name).write_text('notice')
        self.run_script('package-extension.sh', '--out', self.root / 'out',
                        '--stage-dir', self.resources / 'extension')
        self.zip = self.root / f'out/MyPass-chrome-extension-{self.version}.zip'
        self.host = self.app / 'Contents/MacOS/mypass-native-host'
        self.host.parent.mkdir(); self.host.write_bytes(b'cargo-host')
        self.source = self.root / 'sidecar'; self.source.write_bytes(b'cargo-host')
        self.variant = 'signed'
        self.calls = []

    def codesign(self, *args):
        self.calls.append(args)
        if args[0] == 'csreq': return args[1].removeprefix('-r=').replace('"W645B93PPQ"', 'W645B93PPQ')
        if '--verbose=4' in args:
            if self.variant == 'unsigned': return 'Signature=adhoc\nflags=0x20002(adhoc,linker-signed)'
            if self.variant == 'adhoc': return 'Signature=adhoc\nflags=0x2(adhoc)'
            if self.variant == 'runtime': return 'TeamIdentifier=W645B93PPQ\nflags=0x0(none)'
            if self.variant == 'team': return 'TeamIdentifier=OTHER\nflags=0x10000(runtime)'
            return 'TeamIdentifier=W645B93PPQ\nflags=0x10000(runtime)'
        if '-r-' in args:
            identifier = 'com.mypass.chromehost' if args[-1].endswith('mypass-native-host') else 'com.mypass.app'
            dr = self.release.requirement(identifier).replace('"W645B93PPQ"', 'W645B93PPQ')
            if self.variant == 'cn': dr += ' and certificate leaf[subject.CN] = "Name"'
            if self.variant == 'hash': dr += ' and cdhash H"012345"'
            return 'designated => ' + dr
        if '--entitlements' in args and self.variant == 'entitlements':
            return '<plist><dict><key>com.apple.security.get-task-allow</key><true/></dict></plist>'
        return ''

    def verify(self, allow=False):
        with patch.object(self.release, 'command', self.codesign):
            self.release.verify_bundle(self.app, self.source, self.zip, allow)

    def test_signed_bundle_checks_deep_strict_requirements_and_provenance(self):
        self.verify()
        self.assertIn(('codesign', '--verify', '--deep', '--strict', str(self.app)), self.calls)
        self.source.write_bytes(b'replacement')
        with self.assertRaisesRegex(ValueError, 'staged cargo sidecar'): self.verify()

    def test_verifier_fails_closed_for_wrong_signatures(self):
        for variant in ('cn', 'hash', 'runtime', 'team', 'entitlements', 'adhoc'):
            with self.subTest(variant=variant):
                self.variant = variant
                with self.assertRaises(ValueError): self.verify(allow=True)

    def test_unsigned_requires_explicit_allow(self):
        self.variant = 'unsigned'
        with self.assertRaisesRegex(ValueError, 'allow-unsigned'): self.verify()
        self.verify(allow=True)

    def test_verifier_rejects_bundled_development_files(self):
        (self.resources / 'extension/package.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'bundled extension development file'): self.verify()

    def test_verifier_rejects_missing_localization_and_zip_development_files(self):
        with zipfile.ZipFile(self.zip, 'a') as archive: archive.writestr('tests/bad.test.js', 'bad')
        with self.assertRaisesRegex(ValueError, 'development file'): self.verify()
        (self.resources / 'en.lproj/InfoPlist.strings').unlink()
        with self.assertRaises(FileNotFoundError): self.verify()

    def test_asset_set_checks_relative_payload_hashes_and_rejects_extra_files(self):
        folder = self.root / 'out'
        names = self.release.expected_release_assets(self.version)
        (folder / names[0]).write_bytes(b'dmg')
        (folder / 'SHA256SUMS.txt').write_text(''.join(
            f'{hashlib.sha256((folder / n).read_bytes()).hexdigest()}  {n}\n' for n in names[:-1]))
        self.release.assets_check(folder, self.version)
        (folder / 'unexpected').write_text('extra')
        with self.assertRaisesRegex(ValueError, 'asset set'): self.release.assets_check(folder, self.version)


class VersionAgnosticTests(Fixture):
    def test_release_suites_pass_after_a_version_bump(self):
        # Exercise the real scripts and copied suites, not just version helpers. Explicit class
        # selection prevents recursively invoking this regression in the disposable copy.
        bumped = self.next_version
        self.run_script('bump-version.sh', bumped)
        self.run_script('check-version.sh', bumped)
        result = subprocess.run([sys.executable, 'scripts/tests/release-tests.py',
                                 'VersionTests', 'PackageTests', 'VerifyTests'],
                                cwd=self.root, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

if __name__ == '__main__':
    unittest.main()
