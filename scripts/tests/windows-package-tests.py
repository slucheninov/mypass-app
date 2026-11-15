"""Offline Windows package regressions; real contents in disposable repository copies."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import unittest
import zipfile
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
fixture_spec = importlib.util.spec_from_file_location('release_tests', Path(__file__).with_name('release-tests.py'))
fixture = importlib.util.module_from_spec(fixture_spec)
fixture_spec.loader.exec_module(fixture)


def pe(subsystem, machine=0x8664):
    data = bytearray(512)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 60, 128)
    data[128:132] = b'PE\0\0'
    struct.pack_into('<H', data, 132, machine)
    struct.pack_into('<H', data, 148, 240)
    struct.pack_into('<H', data, 152, 0x20b)
    struct.pack_into('<H', data, 220, subsystem)
    return bytes(data)


class WindowsPackageTests(fixture.Fixture):
    def setUp(self):
        super().setUp()
        for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md'):
            (self.root / name).write_bytes((fixture.ROOT / name).read_bytes())
        self.stage = self.root / 'built with spaces'
        self.stage.mkdir()
        (self.stage / 'mypass-app.exe').write_bytes(pe(2))
        (self.stage / 'mypass-native-host.exe').write_bytes(pe(3))
        spec = importlib.util.spec_from_file_location('windows_package', fixture.ROOT / 'scripts/windows-package.py')
        self.module = importlib.util.module_from_spec(spec)
        if spec.loader:
            spec.loader.exec_module(self.module)

    def package(self, *args, ok=True):
        return self.run_script('package-windows.sh', '--no-build', '--stage-from', self.stage,
                               '--out', self.root / 'output with spaces', *args, ok=ok)

    def archive(self):
        return self.root / 'output with spaces' / f'MyPass-{self.version}-windows-x64.zip'

    def test_script_strict_mode_syntax_and_argument_errors(self):
        script = self.root / 'scripts/package-windows.sh'
        self.assertIn('set -euo pipefail', script.read_text())
        subprocess.run([fixture.BASH, '-n', str(script)], check=True)
        for args in (['--unknown'], ['--out'], ['--stage-from', self.stage], ['--no-build']):
            self.run_script('package-windows.sh', *args, ok=False)

    def test_exact_set_names_checksum_crlf_and_reproducibility(self):
        self.package('--version', self.version)
        path = self.archive()
        expected = {'MyPass.exe', 'mypass-native-host.exe', 'INSTALL.txt', 'LICENSE', 'THIRD_PARTY_NOTICES.md'}
        expected |= {'extension/' + rel for _, rel in self.module.extension_files(self.root / 'extension')}
        with zipfile.ZipFile(path) as archive:
            self.assertEqual(set(archive.namelist()), expected)
            self.assertEqual(len(archive.namelist()), len(expected))
            self.assertEqual(archive.read('MyPass.exe'), pe(2))
            self.assertEqual(archive.read('mypass-native-host.exe'), pe(3))
            install = archive.read('INSTALL.txt')
            self.assertNotIn(b'\n', install.replace(b'\r\n', b''))
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        self.assertEqual(path.with_suffix('.zip.sha256').read_text(), f'{digest}  {path.name}\n')
        self.package()
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), digest)
        self.assertEqual(sorted(p.name for p in path.parent.iterdir()), [path.name, path.name + '.sha256'])

    def test_missing_inputs_wrong_pe_and_version_assertion_fail_closed(self):
        self.package('--version', self.next_version, ok=False)
        for name in ('mypass-app.exe', 'mypass-native-host.exe'):
            path = self.stage / name
            before = path.read_bytes()
            for invalid in (b'', b'not a PE', pe(3, 0xaa64), pe(2 if 'host' in name else 3)):
                path.write_bytes(invalid)
                self.package(ok=False)
            path.unlink()
            self.package(ok=False)
            path.write_bytes(before)

    def test_verifier_rejects_extra_duplicate_traversal_tamper_and_wrong_provenance(self):
        self.package()
        path = self.archive()
        original = path.read_bytes()
        for name in ('extra', '../escape', '/absolute', 'extension\\escape', 'MyPass.exe'):
            path.write_bytes(original)
            with zipfile.ZipFile(path, 'a') as archive:
                archive.writestr(name, 'bad')
            with self.assertRaises(ValueError):
                self.module.verify_archive(path, self.stage, self.version, self.root)
        path.write_bytes(original)
        (self.stage / 'mypass-native-host.exe').write_bytes(pe(3) + b'different')
        with self.assertRaisesRegex(ValueError, 'provenance'):
            self.module.verify_archive(path, self.stage, self.version, self.root)

    def test_install_english_lf_and_required_guidance(self):
        data = (self.root / 'scripts/package/INSTALL-windows.txt').read_bytes()
        self.assertNotIn(b'\r', data)
        text = data.decode('ascii')
        for term in ('Windows 10 22H2', 'Windows 11', 'x64', 'WebView2', 'Unblock', 'More info',
                     'Run anyway', 'HKCU', 'HKLM', 'Remove browser integration', 'Get-FileHash',
                     'extension', 'hjijbabodfldconmhdcdphlbgaajkejn', 'Windows Hello', 'overflow'):
            self.assertIn(term, text)

    def test_release_mode_requires_clean_exact_tag_and_own_build(self):
        self.run_script('package-windows.sh', '--release', ok=False)
        for args in (['init', '-q'], ['add', '.'], ['-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'fixture']):
            subprocess.run(['git', *args], cwd=self.root, check=True, capture_output=True)
        subprocess.run(['git', 'tag', f'v{self.version}'], cwd=self.root, check=True)
        with (self.root / 'scripts/package/INSTALL-windows.txt').open('a') as stream:
            stream.write('dirty fixture\n')
        self.assertIn('clean tree', self.run_script('package-windows.sh', '--release', ok=False))
        self.package('--release', ok=False)

    def test_windows_host_build_checks_target_tree_and_never_signs(self):
        target = self.root / 'target'
        binary = target / 'x86_64-pc-windows-msvc/release/mypass-host.exe'
        binary.parent.mkdir(parents=True)
        binary.write_bytes(pe(3))
        self.env['TEST_TARGET_DIR'] = str(target)
        self.env['MYPASS_SIGNING_IDENTITY'] = '-'
        cargo = self.root / 'stub-bin/cargo'
        cargo.write_text("""#!/usr/bin/env bash
echo "$*" >> "$CARGO_TEST_LOG"
if [[ "$1" == metadata ]]; then python3 -c 'import json,os; print(json.dumps({"target_directory":os.environ["TEST_TARGET_DIR"]}))'; fi
""")
        self.run_script('build-host.sh', '--release', '--target', 'x86_64-pc-windows-msvc')
        log = (self.root / 'cargo.log').read_text()
        self.assertIn('tree --locked -p mypass-host -e normal --target x86_64-pc-windows-msvc', log)
        self.assertIn('build -p mypass-host --release --target x86_64-pc-windows-msvc', log)
        self.assertEqual((self.root / 'src-tauri/binaries/mypass-native-host-x86_64-pc-windows-msvc.exe').read_bytes(), pe(3))

    def test_native_windows_host_build_ignores_apple_identities_without_target(self):
        target = self.root / 'native target with spaces'
        binary = target / 'release/mypass-host.exe'
        binary.parent.mkdir(parents=True)
        binary.write_bytes(pe(3))
        self.env['TEST_TARGET_DIR'] = str(target)
        self.env['CODESIGN_LOG'] = str(self.root / 'codesign.log')
        stubs = {
            'uname': 'echo MINGW64_NT-10.0',
            'rustc': 'echo "host: x86_64-pc-windows-msvc"',
            'codesign': 'echo called > "$CODESIGN_LOG"; exit 1',
            'cargo': """[[ -z "${MYPASS_SIGNING_IDENTITY+set}" && -z "${APPLE_SIGNING_IDENTITY+set}" ]] || exit 91
echo "$*" >> "$CARGO_TEST_LOG"
if [[ "$1" == metadata ]]; then python3 -c 'import json,os; print(json.dumps({"target_directory":os.environ["TEST_TARGET_DIR"]}))'; fi""",
        }
        for name, body in stubs.items():
            path = self.root / 'stub-bin' / name
            path.write_text('#!/usr/bin/env bash\n' + body + '\n')
            path.chmod(0o755)
        for identity in ('-', 'Apple Development: ambiguous name', 'A' * 40):
            with self.subTest(identity=identity):
                self.env['MYPASS_SIGNING_IDENTITY'] = identity
                self.env['APPLE_SIGNING_IDENTITY'] = identity
                self.run_script('build-host.sh', '--release')
                self.assertEqual((self.root / 'src-tauri/binaries/mypass-native-host-x86_64-pc-windows-msvc.exe').read_bytes(), pe(3))
        log = (self.root / 'cargo.log').read_text().splitlines()
        self.assertEqual(log.count('build -p mypass-host --release'), 3)
        self.assertEqual(log.count('tree --locked -p mypass-host -e normal --target x86_64-pc-windows-msvc'), 3)
        self.assertFalse((self.root / 'codesign.log').exists())

    def test_host_smoke_parser_rejects_empty_truncated_extra_and_wrong_frames(self):
        spec = importlib.util.spec_from_file_location('host_smoke', fixture.ROOT / 'scripts/windows-host-smoke.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        body = b'{"type":"error","message":"app_not_running"}'
        frame = struct.pack('<I', len(body)) + body
        module.check_frame(frame)
        for data in (b'', frame[:3], frame[:-1], frame + b'extra', struct.pack('<I', 2) + b'{}'):
            with self.assertRaises(ValueError):
                module.check_frame(data)

    def test_dependency_check_uses_requested_target_and_rejects_forbidden_tree(self):
        cargo = self.root / 'stub-bin/cargo'
        cargo.write_text('#!/usr/bin/env bash\necho "$*" >> "$CARGO_TEST_LOG"\necho "${TEST_TREE:-mypass-host}"\n')
        self.run_script('check-host-deps.sh', '--target', 'x86_64-pc-windows-msvc')
        self.assertIn('--target x86_64-pc-windows-msvc', (self.root / 'cargo.log').read_text())
        self.env['TEST_TREE'] = 'mypass-core'
        self.run_script('check-host-deps.sh', '--target', 'x86_64-pc-windows-msvc', ok=False)


if __name__ == '__main__':
    unittest.main()
