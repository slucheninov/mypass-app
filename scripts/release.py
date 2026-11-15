#!/usr/bin/env python3
"""Release metadata, deterministic extension packaging and fail-closed bundle inspection.

Python's standard library keeps these checks usable from the Rust-only test entry point.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parent.parent
TEAM = 'W645B93PPQ'
APP_ID = 'com.mypass.app'
HOST_ID = 'com.mypass.chromehost'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def semver(value):
    require(re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', value), 'expected X.Y.Z without leading zeros')
    require(all(int(n) <= 65535 for n in value.split('.')), 'version exceeds Chrome component limit')
    return tuple(map(int, value.split('.')))


def version():
    return tomllib.loads((ROOT / 'src-tauri/Cargo.toml').read_text())['package']['version']


def versions():
    found = {'src-tauri/Cargo.toml': version()}
    for name in ('package.json', 'extension/manifest.json', 'extension/package.json'):
        data = json.loads((ROOT / name).read_text())
        if 'version' in data:
            found[name] = data['version']
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    found['Cargo.lock'] = next(p['version'] for p in lock['package'] if p['name'] == 'mypass-app')
    require('version' not in json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text()), 'src-tauri/tauri.conf.json must omit version')
    return found


def check_version(expected=None):
    expected = expected or version()
    semver(expected)
    for path, actual in versions().items():
        require(actual == expected, f'{path}: version {actual}, expected {expected}')
    return expected


def bump(new):
    target = semver(new)
    current = versions()
    require(all(target > semver(v) for v in current.values()), 'new version must be greater than every current version')
    paths = [ROOT / p for p in current]
    for path in paths:
        require(path.stat().st_mode & stat.S_IWUSR and os.access(path, os.W_OK), f'{path}: not writable')
        require(not path.is_symlink(), f'{path}: symlink refused')
    originals = {p: p.read_bytes() for p in paths}
    updates = {}
    for path in paths:
        text = originals[path].decode()
        if path.name == 'Cargo.lock':
            text, count = re.subn(r'(\[\[package\]\]\nname = "mypass-app"\nversion = ")[^"]+("\n)', rf'\g<1>{new}\2', text)
        elif path.suffix == '.toml':
            text, count = re.subn(r'(?m)^version = "[^"]+"$', f'version = "{new}"', text)
        else:
            text, count = re.subn(r'("version"\s*:\s*")[^"]+("\s*[,}])', rf'\g<1>{new}\2', text)
        require(count == 1, f'{path}: expected exactly one version')
        updates[path] = text.encode()
    temporary = []
    try:
        for path, data in updates.items():
            fd, temp = tempfile.mkstemp(prefix='.version-', dir=path.parent)
            temporary.append((path, Path(temp)))
            with os.fdopen(fd, 'wb') as stream:
                stream.write(data)
            os.chmod(temp, stat.S_IMODE(path.stat().st_mode))
        for path, temp in temporary:
            os.replace(temp, path)
        # Resolve only workspace package versions, offline; any error restores every original.
        # Resolve PATH launchers explicitly: Windows CreateProcess skips cargo.cmd for
        # a bare 'cargo', unlike Unix exec lookup (or a shell).
        cargo = shutil.which('cargo')
        require(cargo is not None, 'cargo is required on PATH')
        refreshed = subprocess.run([cargo, 'update', '-w', '--offline'], cwd=ROOT, check=False)
        if refreshed.returncode:
            subprocess.run([cargo, 'check', '-p', 'mypass-app', '--offline'], cwd=ROOT, check=True)
        check_version(new)
    except BaseException:
        for path, data in originals.items():
            path.write_bytes(data)
        raise
    finally:
        for _, temp in temporary:
            temp.unlink(missing_ok=True)
    subprocess.run(['git', 'diff', '--stat'], cwd=ROOT, check=False)


def expected_release_assets(ver, platforms=('macos-arm64',)):
    semver(ver)
    return [*(f'MyPass-{ver}-{p}.dmg' for p in platforms), f'MyPass-chrome-extension-{ver}.zip', 'SHA256SUMS.txt']


def extension_files(folder):
    for path in sorted(folder.rglob('*')):
        rel = path.relative_to(folder)
        if any(p in ('tests', 'node_modules', '.git', '__pycache__') for p in rel.parts):
            continue
        if path.name in ('package.json', '.DS_Store') or path.name.endswith('.test.js'):
            continue
        require(not path.is_symlink(), f'extension symlink refused: {rel}')
        if path.is_file():
            yield path, rel.as_posix()


def package_extension(out, stage):
    ver = check_version()
    out.mkdir(parents=True, exist_ok=True)
    files = list(extension_files(ROOT / 'extension'))
    if stage:
        require(not stage.exists() or not any(stage.iterdir()), 'stage directory must be empty')
        stage.mkdir(parents=True, exist_ok=True)
    archive_path = out / f'MyPass-chrome-extension-{ver}.zip'
    with zipfile.ZipFile(archive_path, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path, rel in files:
            info = zipfile.ZipInfo(rel, date_time=(2020, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, path.read_bytes())
            if stage:
                dest = stage / rel
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, dest)
    verify_zip(archive_path, ver)
    print(archive_path)


def verify_zip(path, ver, extension=None):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        require(len(names) == len(set(names)), 'extension zip duplicate entries')
        require('manifest.json' in names, 'extension zip missing manifest.json')
        for name in names:
            p = Path(name)
            require(not p.is_absolute() and '..' not in p.parts and '\\' not in name, 'extension zip unsafe path')
            require('tests' not in p.parts and p.name not in ('package.json', '.DS_Store') and not name.endswith('.test.js'), f'extension zip development file: {name}')
        require(json.loads(archive.read('manifest.json'))['version'] == ver, 'extension zip version mismatch')
        if extension:
            files = {}
            for p in sorted(extension.rglob('*')):
                rel = p.relative_to(extension)
                require(not p.is_symlink(), f'bundled extension symlink: {rel}')
                require('tests' not in rel.parts and p.name not in ('package.json', '.DS_Store') and not p.name.endswith('.test.js'), f'bundled extension development file: {rel}')
                if p.is_file():
                    files[rel.as_posix()] = p.read_bytes()
            require(set(names) == set(files), 'extension zip differs from bundled extension file set')
            for name, data in files.items():
                require(archive.read(name) == data, f'extension zip differs from bundle: {name}')


def command(*args):
    result = subprocess.run(args, capture_output=True, text=True)
    require(result.returncode == 0, f'{args[0]} {args[1:]}: {result.stderr.strip()}')
    return result.stdout + result.stderr


def requirement(identifier):
    return f'identifier "{identifier}" and anchor apple generic and certificate leaf[subject.OU] = "{TEAM}"'


def verify_bundle(app, host_source, zip_path, allow_unsigned):
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    for key, value in {'CFBundleIdentifier': APP_ID, 'LSMinimumSystemVersion': '14.0', 'CFBundleDevelopmentRegion': 'en', 'CFBundleLocalizations': ['en', 'uk', 'ru']}.items():
        require(info.get(key) == value, f'Info.plist {key}: expected {value}')
    resources = app / 'Contents/Resources'
    for lang in ('en', 'uk', 'ru'):
        require((resources / f'{lang}.lproj/InfoPlist.strings').stat().st_size > 0, f'{lang}: empty InfoPlist.strings')
    require({p.name for p in resources.glob('*.lproj')} == {'en.lproj', 'uk.lproj', 'ru.lproj'}, 'unexpected bundle localizations')
    require((resources / 'LICENSE').is_file() and (resources / 'THIRD_PARTY_NOTICES.md').is_file(), 'missing bundled notices')
    verify_zip(zip_path, info['CFBundleShortVersionString'], resources / 'extension')
    host = app / 'Contents/MacOS/mypass-native-host'
    require(host.read_bytes() == host_source.read_bytes(), 'bundled host differs from staged cargo sidecar')
    # A linker-only seal is expected on unsigned arm64 builds. Never accept it in release mode.
    unsigned = False
    for target, identifier in ((app, APP_ID), (host, HOST_ID)):
        display = command('codesign', '-d', '--verbose=4', str(target))
        linker_signed = 'Signature=adhoc' in display and 'linker-signed' in display
        if linker_signed:
            require(allow_unsigned, f'{identifier}: UNSIGNED development build requires --allow-unsigned')
            unsigned = True
            continue
        require('Signature=adhoc' not in display, f'{identifier}: non-linker ad-hoc signature refused')
        require(f'TeamIdentifier={TEAM}' in display, f'{identifier}: TeamIdentifier')
        require(re.search(r'flags=0x[0-9a-f]+\([^)]*\bruntime\b', display), f'{identifier}: hardened runtime')
        dr = command('codesign', '-d', '-r-', str(target))
        actual = next((l.removeprefix('designated => ').strip() for l in dr.splitlines() if l.startswith('designated => ')), '')
        # Apple's renderer drops optional quotes around the alphanumeric team value.
        # Compare the entire DR against Apple's rendering of our exact required expression.
        expected = command('csreq', '-r=' + requirement(identifier), '-t').strip()
        require(expected and actual == expected, f'{identifier}: designated requirement mismatch: {actual}')
        entitlements = command('codesign', '-d', '--entitlements', ':-', str(target))
        # No entitlements are needed at all; reject get-task-allow even if false.
        require('<key>' not in entitlements, f'{identifier}: unexpected entitlements')
        command('codesign', '--verify', '--strict', str(target))
    if unsigned:
        require(allow_unsigned, 'UNSIGNED development build')
        print('UNSIGNED development build: signature checks unavailable (linker seals only)')
    else:
        command('codesign', '--verify', '--deep', '--strict', str(app))
    print('bundle structure, host provenance and extension zip verified')


def assets_check(folder, ver):
    expected = expected_release_assets(ver)
    require(sorted(p.name for p in folder.iterdir()) == sorted(expected), 'release asset set mismatch')
    # A checksum file cannot hash itself. Hash exactly the two payload artifacts, relative names.
    lines = (folder / 'SHA256SUMS.txt').read_text().splitlines()
    require(len(lines) == len(expected) - 1, 'SHA256SUMS.txt payload count')
    require(lines == [f'{hashlib.sha256((folder / n).read_bytes()).hexdigest()}  {n}' for n in expected[:-1]], 'SHA256SUMS.txt mismatch')
    verify_zip(folder / expected[-2], ver)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='mode', required=True)
    sub.add_parser('version').add_argument('expected', nargs='?')
    sub.add_parser('bump').add_argument('version')
    sub.add_parser('assets').add_argument('version')
    notes = sub.add_parser('notes'); notes.add_argument('version'); notes.add_argument('output', type=Path)
    check = sub.add_parser('check-assets'); check.add_argument('folder', type=Path); check.add_argument('version')
    ext = sub.add_parser('extension'); ext.add_argument('--out', type=Path, default=ROOT / 'release'); ext.add_argument('--stage-dir', type=Path)
    verify = sub.add_parser('verify'); verify.add_argument('app', type=Path); verify.add_argument('--host-source', required=True, type=Path); verify.add_argument('--extension-zip', required=True, type=Path); verify.add_argument('--allow-unsigned', action='store_true')
    args = parser.parse_args()
    if args.mode == 'version': print(check_version(args.expected))
    elif args.mode == 'bump': bump(args.version)
    elif args.mode == 'assets': print('\n'.join(expected_release_assets(args.version)))
    elif args.mode == 'notes':
        semver(args.version)
        text = (ROOT / 'scripts/package/RELEASE_NOTES_HEADER.md').read_text().replace('{{VERSION}}', args.version)
        require('{{VERSION}}' not in text, 'release notes contain unresolved version')
        args.output.write_text(text)
    elif args.mode == 'check-assets': assets_check(args.folder, args.version)
    elif args.mode == 'extension': package_extension(args.out, args.stage_dir)
    elif args.mode == 'verify': verify_bundle(args.app, args.host_source, args.extension_zip, args.allow_unsigned)

if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError, KeyError, StopIteration, zipfile.BadZipFile) as error:
        sys.exit(f'release: {error}')
