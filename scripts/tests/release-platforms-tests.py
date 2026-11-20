"""Offline combined-release integrity and publication failures; never contacts GitHub."""
import hashlib
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest
import zipfile
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('fixtures', Path(__file__).with_name('release-tests.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class ReleasePlatforms(fixtures.Fixture):
    def setUp(self):
        super().setUp()
        spec = importlib.util.spec_from_file_location('release', self.root / 'scripts/release.py')
        self.release = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.release)
        self.folder = self.root / 'assets'
        self.run_script('package-extension.sh', '--out', self.folder)
        self.dmg = f'MyPass-{self.version}-macos-arm64.dmg'
        self.win = f'MyPass-{self.version}-windows-x64.zip'
        self.ext = f'MyPass-chrome-extension-{self.version}.zip'
        (self.folder / self.dmg).write_bytes(b'dmg')
        self.sums('SHA256SUMS.txt', [self.dmg, self.ext])
        # Real package verifier receives synthetic, well-formed x64 PE fixtures.
        import struct
        def pe(subsystem):
            data = bytearray(256)
            data[:2] = b'MZ'
            struct.pack_into('<I', data, 60, 64)
            data[64:68] = b'PE\0\0'
            struct.pack_into('<H', data, 68, 0x8664)
            struct.pack_into('<H', data, 84, 112)
            struct.pack_into('<H', data, 88, 0x20b)
            struct.pack_into('<H', data, 156, subsystem)
            return data
        with zipfile.ZipFile(self.folder / self.win, 'w') as archive:
            archive.writestr('MyPass.exe', pe(2))
            archive.writestr('mypass-native-host.exe', pe(3))
            archive.writestr('INSTALL.txt', (self.root / 'scripts/package/INSTALL-windows.txt').read_text().replace('\n', '\r\n'))
            for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md'):
                shutil.copy2(fixtures.ROOT / name, self.root / name)
                archive.writestr(name, (self.root / name).read_bytes())
            for path, rel in self.release.extension_files(self.root / 'extension'):
                archive.writestr('extension/' + rel, path.read_bytes())
        self.sums(self.win + '.sha256', [self.win])
        self.env['ASSETS'] = str(self.folder)
        self.env['GH_LOG'] = str(self.root / 'gh.log')
        gh = self.root / 'stub-bin/gh'
        gh.write_text('''#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> "$GH_LOG"
case "$1 $2" in
  'release view') echo "${DRAFT-true}"; [[ "${GH_FAIL:-0}" == 0 ]] || exit 1 ;;
  'release download') [[ "${GH_DOWNLOAD_FAIL:-0}" == 0 ]] || exit 1; mkdir -p "$5"; cp "$ASSETS/"* "$5/" ;;
  'release edit') exit 0 ;;
  *) exit 99 ;;
esac
''', newline="\n")
        gh.chmod(0o755)

    def sums(self, manifest, names):
        (self.folder / manifest).write_text(''.join(
            f'{hashlib.sha256((self.folder / name).read_bytes()).hexdigest()}  {name}\n' for name in names))

    def publish(self, mac='success', win='success', ok=True):
        return self.run_script('publish-release.sh', f'v{self.version}', self.root / 'download', mac, win, ok=ok)

    def test_shell_mocks_keep_lf_with_windows_default_newlines(self):
        original = Path.write_text

        def windows_write_text(path, data, *args, **kwargs):
            kwargs.setdefault('newline', '\r\n')
            return original(path, data, *args, **kwargs)

        fixture = ReleasePlatforms('test_exact_modes_and_mapping')
        self.addCleanup(fixture.doCleanups)
        with patch.object(Path, 'write_text', windows_write_text):
            fixture.setUp()
        for name in ('gh', 'cargo'):
            self.assertNotIn(b'\r', (fixture.root / 'stub-bin' / name).read_bytes())

    @unittest.skipUnless(sys.platform == 'darwin', 'Apple system Bash regression')
    def test_system_bash_refusals_stop_before_download(self):
        for draft, failure in [('false', '0'), ('', '0'), ('true', '1')]:
            with self.subTest(draft=draft, api_failure=failure):
                self.env.update(DRAFT=draft, GH_FAIL=failure)
                result = subprocess.run(
                    ['/bin/bash', 'scripts/publish-release.sh', f'v{self.version}',
                     str(self.root / 'download'), 'success', 'success'],
                    cwd=self.root, env=self.env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual((self.root / 'gh.log').read_text().splitlines(),
                                 [f'release view v{self.version} --json isDraft --jq .isDraft'])
                (self.root / 'gh.log').unlink()

    def test_failures_stop_even_when_caller_disables_errexit(self):
        for failure in ('draft', 'api', 'download', 'assets'):
            with self.subTest(failure=failure):
                self.env.update(DRAFT='false' if failure == 'draft' else 'true',
                                GH_FAIL='1' if failure == 'api' else '0',
                                GH_DOWNLOAD_FAIL='1' if failure == 'download' else '0')
                if failure == 'assets':
                    (self.folder / self.win).unlink()
                result = subprocess.run(
                    [fixtures.BASH, '-c', 'if source "$0" "$@"; then exit 0; else exit $?; fi',
                     'scripts/publish-release.sh', f'v{self.version}',
                     str(self.root / 'download'), 'success', 'success'],
                    cwd=self.root, env=self.env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                log = (self.root / 'gh.log').read_text()
                self.assertNotIn('--draft=false', log)
                if failure in ('draft', 'api'):
                    self.assertNotIn('release download', log)
                (self.root / 'gh.log').unlink()
                shutil.rmtree(self.root / 'download', ignore_errors=True)

    def test_exact_modes_and_mapping(self):
        self.assertEqual(self.release.expected_release_assets(self.version, ('windows-x64',)), [self.win, self.win + '.sha256'])
        self.assertEqual(set(self.release.expected_release_assets(self.version, ('macos-arm64', 'windows-x64'))),
                         {self.dmg, self.ext, 'SHA256SUMS.txt', self.win, self.win + '.sha256'})
        with self.assertRaises(ValueError): self.release.expected_release_assets(self.version, ('linux',))
        self.publish()
        flags = ' --prerelease --latest=false' if '-' in self.version else ''
        self.assertEqual((self.root / 'gh.log').read_text().splitlines()[-1],
                         f'release edit v{self.version} --draft=false{flags}')
        for mode, names in [('macos', [self.dmg, self.ext, 'SHA256SUMS.txt']), ('windows', [self.win, self.win + '.sha256'])]:
            folder = self.root / mode
            folder.mkdir()
            for name in names: shutil.copy2(self.folder / name, folder / name)
            subprocess.run([sys.executable, self.root / 'scripts/release.py', 'check-assets', folder,
                            self.version, '--platform', mode], check=True)

    def test_mismatched_tag_stops_before_github(self):
        self.run_script('publish-release.sh', f'v{self.next_version}-alpha.1',
                        self.root / 'download', 'success', 'success', ok=False)
        self.assertFalse((self.root / 'gh.log').exists())

    def test_missing_windows_and_extra_assets_block_publish(self):
        for missing in (self.win, self.win + '.sha256'):
            path = self.folder / missing
            data = path.read_bytes(); path.unlink()
            self.publish(ok=False)
            self.assertNotIn('--draft=false', (self.root / 'gh.log').read_text())
            path.write_bytes(data)
            shutil.rmtree(self.root / 'download')
        (self.folder / 'extra.zip').write_bytes(b'extra')
        self.publish(ok=False)
        self.assertNotIn('--draft=false', (self.root / 'gh.log').read_text())

    def test_checksum_schema_and_mismatch_block_publish(self):
        for manifest, name in [('SHA256SUMS.txt', self.dmg), (self.win + '.sha256', self.win)]:
            path = self.folder / manifest
            good = path.read_text()
            for bad in [good + good, good.replace(name, '../' + name), good.replace(name, '/' + name),
                        'malformed\n', good.replace(good[:64], '0' * 64)]:
                path.write_text(bad)
                self.publish(ok=False)
                self.assertNotIn('--draft=false', (self.root / 'gh.log').read_text())
                shutil.rmtree(self.root / 'download')
            path.write_text(good)
        (self.folder / self.dmg).write_bytes(b'tampered')
        self.publish(ok=False)

    def test_failed_cancelled_skipped_producers_never_contact_gh(self):
        for result in ('failure', 'cancelled', 'skipped', ''):
            for mac, win in [(result, 'success'), ('success', result)]:
                self.publish(mac, win, ok=False)
                self.assertFalse((self.root / 'gh.log').exists())

    def test_published_and_api_failure_are_refused(self):
        self.env['DRAFT'] = 'false'
        self.publish(ok=False)
        self.assertNotIn('--draft=false', (self.root / 'gh.log').read_text())
        self.env['GH_FAIL'] = '1'
        self.publish(ok=False)

    def test_download_failure_does_not_publish(self):
        self.env['GH_DOWNLOAD_FAIL'] = '1'
        self.publish(ok=False)
        self.assertNotIn('--draft=false', (self.root / 'gh.log').read_text())

    def test_manifest_record_order_is_not_positional(self):
        path = self.folder / 'SHA256SUMS.txt'
        path.write_text('\n'.join(reversed(path.read_text().splitlines())) + '\n')
        self.publish()

    def test_windows_payload_tampering_blocks_publish(self):
        with (self.folder / self.win).open('ab') as stream:
            stream.write(b'tampering')
        self.publish(ok=False)
        self.assertNotIn('--draft=false', (self.root / 'gh.log').read_text())

    def test_windows_development_contents_rejected_even_with_valid_hash(self):
        with zipfile.ZipFile(self.folder / self.win, 'a') as archive:
            archive.writestr('extension/tests/bad.test.js', 'bad')
        self.sums(self.win + '.sha256', [self.win])
        self.publish(ok=False)
        self.assertNotIn('--draft=false', (self.root / 'gh.log').read_text())


if __name__ == '__main__':
    unittest.main()
